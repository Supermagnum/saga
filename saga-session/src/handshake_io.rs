//! Length-prefixed InitMessage / ResponseMessage + confirmation MACs over a stream.

use ephemeral_session::{
    InitMessage, LongTermCert, ResponseMessage, SessionCurve, MAX_HANDSHAKE_BYTES,
};
use log::{info, warn};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroize;

use crate::confirm::{initiator_mac, responder_mac, verify_mac, ConfirmationKey, CONFIRM_MAC_LEN};
use crate::token::{
    HandshakeOutput, SessionToken, TokenError, HANDSHAKE_HARD_TIMEOUT, SESSION_LOG_MARKER,
};

pub enum HandshakeOutcome {
    Encrypted { output: HandshakeOutput },
    Failed { reason: String },
}

impl std::fmt::Debug for HandshakeOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HandshakeOutcome::Encrypted { output } => {
                let (a, b) = output.session_key_debug_prefix();
                write!(f, "Encrypted {{ session_key_hash_prefix: {a:02x}{b:02x} }}")
            }
            HandshakeOutcome::Failed { reason } => write!(f, "Failed {{ reason: {reason:?} }}"),
        }
    }
}

impl HandshakeOutcome {
    pub fn session_key(&self) -> Option<[u8; 32]> {
        match self {
            HandshakeOutcome::Encrypted { output } => Some(*output.saga_session_key),
            HandshakeOutcome::Failed { .. } => None,
        }
    }
}

/// XOR the first byte of `eid` for debug fault-injection of channel binding.
pub fn perturb_endpoint_id(eid: &[u8]) -> Vec<u8> {
    let mut out = eid.to_vec();
    if let Some(b) = out.first_mut() {
        *b ^= 0x5a;
    }
    out
}

async fn write_framed<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> Result<(), TokenError> {
    if bytes.len() > MAX_HANDSHAKE_BYTES || bytes.len() > u16::MAX as usize {
        return Err(TokenError::TruncatedMessage);
    }
    let len = (bytes.len() as u16).to_be_bytes();
    writer
        .write_all(&len)
        .await
        .map_err(|e| TokenError::Protocol(format!("write len: {e}")))?;
    writer
        .write_all(bytes)
        .await
        .map_err(|e| TokenError::Protocol(format!("write body: {e}")))?;
    Ok(())
}

async fn read_framed<R: AsyncRead + Unpin>(reader: &mut R) -> Result<Vec<u8>, TokenError> {
    let mut len_buf = [0u8; 2];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|_| TokenError::TruncatedMessage)?;
    let len = u16::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_HANDSHAKE_BYTES {
        return Err(TokenError::TruncatedMessage);
    }
    let mut buf = vec![0u8; len];
    reader
        .read_exact(&mut buf)
        .await
        .map_err(|_| TokenError::TruncatedMessage)?;
    Ok(buf)
}

async fn write_confirm_mac<W: AsyncWrite + Unpin>(
    writer: &mut W,
    mac: &[u8; CONFIRM_MAC_LEN],
) -> Result<(), TokenError> {
    writer
        .write_all(mac)
        .await
        .map_err(|e| TokenError::Protocol(format!("write confirm mac: {e}")))
}

async fn read_confirm_mac<R: AsyncRead + Unpin>(
    reader: &mut R,
) -> Result<[u8; CONFIRM_MAC_LEN], TokenError> {
    let mut buf = [0u8; CONFIRM_MAC_LEN];
    match tokio::time::timeout(HANDSHAKE_HARD_TIMEOUT, reader.read_exact(&mut buf)).await {
        Ok(Ok(_)) => Ok(buf),
        Ok(Err(_)) => Err(TokenError::ConfirmationFailed),
        Err(_) => Err(TokenError::Timeout),
    }
}

async fn exchange_confirm_as_initiator<R, W>(
    output: &HandshakeOutput,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), TokenError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let key = ConfirmationKey::from_bytes(*output.confirmation_key);
    let my_mac = initiator_mac(&key, &output.transcript_hash);
    let expect_peer = responder_mac(&key, &output.transcript_hash);
    write_confirm_mac(writer, &my_mac).await?;
    let _ = writer.flush().await;
    let peer = read_confirm_mac(reader).await?;
    verify_mac(&expect_peer, &peer)
}

async fn exchange_confirm_as_responder<R, W>(
    output: &HandshakeOutput,
    reader: &mut R,
    writer: &mut W,
) -> Result<(), TokenError>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let key = ConfirmationKey::from_bytes(*output.confirmation_key);
    let expect_peer = initiator_mac(&key, &output.transcript_hash);
    let my_mac = responder_mac(&key, &output.transcript_hash);
    // Recv then send before verify so a mismatched key still unblocks the initiator.
    let peer = read_confirm_mac(reader).await?;
    write_confirm_mac(writer, &my_mac).await?;
    let _ = writer.flush().await;
    verify_mac(&expect_peer, &peer)
}

/// Initiator: Init/Response + confirmation over a bi-stream.
#[allow(clippy::too_many_arguments)]
pub async fn run_initiator_session<T, R, W>(
    token: &T,
    peer_cert: LongTermCert,
    curve: SessionCurve,
    log_label: &str,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
    force_fail: bool,
    perturb_binding: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    T: SessionToken,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "handshake force_fail".into(),
        };
    }

    let bind_local = if perturb_binding {
        perturb_endpoint_id(local_endpoint_id)
    } else {
        local_endpoint_id.to_vec()
    };

    let (init, pending) = match token.initiator_offer(curve, HANDSHAKE_HARD_TIMEOUT).await {
        Ok(v) => v,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };

    let init_bytes = match init.serialise() {
        Ok(v) => v.as_slice().to_vec(),
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };
    info!(
        "[{SESSION_LOG_MARKER}] initiator [{log_label}] sending InitMessage len={}",
        init_bytes.len()
    );
    if let Err(e) = write_framed(writer, &init_bytes).await {
        return HandshakeOutcome::Failed {
            reason: e.to_string(),
        };
    }

    let resp_bytes = match read_framed(reader).await {
        Ok(b) => b,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };
    let response = match ResponseMessage::parse(&resp_bytes) {
        Ok(m) => m,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: TokenError::from(e).to_string(),
            }
        }
    };

    let mut output = match token
        .initiator_finish(
            pending,
            response,
            peer_cert,
            &bind_local,
            remote_endpoint_id,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
    {
        Ok(o) => o,
        Err(e) => {
            warn!("[{SESSION_LOG_MARKER}] initiator finish failed: {e}");
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            };
        }
    };

    if let Err(e) = exchange_confirm_as_initiator(&output, reader, writer).await {
        warn!("[{SESSION_LOG_MARKER}] initiator confirmation failed: {e}");
        return HandshakeOutcome::Failed {
            reason: e.to_string(),
        };
    }

    #[cfg(debug_assertions)]
    {
        let (a, b) = output.session_key_debug_prefix();
        info!(
            "[{SESSION_LOG_MARKER}] initiator handshake complete session_key_hash_prefix={a:02x}{b:02x}"
        );
    }
    output.confirmation_key.zeroize();
    HandshakeOutcome::Encrypted { output }
}

/// Responder: Init/Response + confirmation over a bi-stream.
#[allow(clippy::too_many_arguments)]
pub async fn run_responder_session<T, R, W>(
    token: &T,
    peer_cert: LongTermCert,
    #[allow(unused_variables)] log_label: &str,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
    force_fail: bool,
    perturb_binding: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    T: SessionToken,
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "handshake force_fail".into(),
        };
    }

    let bind_local = if perturb_binding {
        perturb_endpoint_id(local_endpoint_id)
    } else {
        local_endpoint_id.to_vec()
    };

    let init_bytes = match read_framed(reader).await {
        Ok(b) => b,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };
    let init = match InitMessage::parse(&init_bytes) {
        Ok(m) => m,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: TokenError::from(e).to_string(),
            }
        }
    };

    let (resp, mut output) = match token
        .responder_reply(
            init,
            peer_cert,
            &bind_local,
            remote_endpoint_id,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!("[{SESSION_LOG_MARKER}] responder reply failed: {e}");
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            };
        }
    };

    let resp_bytes = match resp.serialise() {
        Ok(v) => v.as_slice().to_vec(),
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };
    if let Err(e) = write_framed(writer, &resp_bytes).await {
        return HandshakeOutcome::Failed {
            reason: e.to_string(),
        };
    }
    let _ = writer.flush().await;

    if let Err(e) = exchange_confirm_as_responder(&output, reader, writer).await {
        warn!("[{SESSION_LOG_MARKER}] responder confirmation failed: {e}");
        return HandshakeOutcome::Failed {
            reason: e.to_string(),
        };
    }

    #[cfg(debug_assertions)]
    {
        let (a, b) = output.session_key_debug_prefix();
        info!(
            "[{SESSION_LOG_MARKER}] responder [{log_label}] handshake complete session_key_hash_prefix={a:02x}{b:02x}"
        );
    }
    output.confirmation_key.zeroize();
    HandshakeOutcome::Encrypted { output }
}
