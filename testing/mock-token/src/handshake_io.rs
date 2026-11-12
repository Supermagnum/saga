//! Length-prefixed InitMessage / ResponseMessage over an async byte stream.

use ephemeral_session::{InitMessage, LongTermCert, ResponseMessage, MAX_HANDSHAKE_BYTES};
use log::{info, warn};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::identity::MockIdentity;
use crate::mock_session::MockSessionToken;
use crate::token::{
    HandshakeOutput, SessionToken, TokenError, HANDSHAKE_HARD_TIMEOUT, MOCK_TOKEN_MARKER,
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

/// Initiator side of ephemeral-session over a byte stream.
pub async fn run_initiator<R, W>(
    identity: &MockIdentity,
    peer_label: &str,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
    force_fail: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "mock-token force_fail".into(),
        };
    }

    let token = MockSessionToken::new(identity.clone());
    let peer = MockIdentity::from_label_curve(peer_label, identity.curve);

    let (init, pending) = match token
        .initiator_offer(identity.curve, HANDSHAKE_HARD_TIMEOUT)
        .await
    {
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
        "[{MOCK_TOKEN_MARKER}] initiator [{}] sending InitMessage len={}",
        identity.label,
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

    match token
        .initiator_finish(
            pending,
            response,
            peer.long_term_cert(),
            local_endpoint_id,
            remote_endpoint_id,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
    {
        Ok(output) => {
            #[cfg(debug_assertions)]
            {
                let (a, b) = output.session_key_debug_prefix();
                info!(
                    "[{MOCK_TOKEN_MARKER}] initiator handshake complete session_key_hash_prefix={a:02x}{b:02x}"
                );
            }
            HandshakeOutcome::Encrypted { output }
        }
        Err(e) => {
            warn!("[{MOCK_TOKEN_MARKER}] initiator finish failed: {e}");
            HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    }
}

/// Responder side of ephemeral-session over a byte stream.
pub async fn run_responder<R, W>(
    identity: &MockIdentity,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
    force_fail: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "mock-token force_fail".into(),
        };
    }

    let token = MockSessionToken::new(identity.clone());
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

    let peer_cert = match resolve_peer_cert_from_init(&init) {
        Ok(c) => c,
        Err(e) => {
            return HandshakeOutcome::Failed {
                reason: e.to_string(),
            }
        }
    };

    let (resp, output) = match token
        .responder_reply(
            init,
            peer_cert,
            local_endpoint_id,
            remote_endpoint_id,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
    {
        Ok(v) => v,
        Err(e) => {
            warn!("[{MOCK_TOKEN_MARKER}] responder reply failed: {e}");
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

    #[cfg(debug_assertions)]
    {
        let (a, b) = output.session_key_debug_prefix();
        info!(
            "[{MOCK_TOKEN_MARKER}] responder [{}] handshake complete session_key_hash_prefix={a:02x}{b:02x}",
            identity.label
        );
    }
    HandshakeOutcome::Encrypted { output }
}

fn resolve_peer_cert_from_init(init: &InitMessage) -> Result<LongTermCert, TokenError> {
    let wire_fp = init.long_term_fingerprint.as_slice();
    for label in [
        "15550100010",
        "15550100011",
        "15550100012",
        "alicepeer1",
        "bobpeer12",
    ] {
        let id = MockIdentity::from_label_curve(label, init.curve);
        if id.fingerprint_hex().as_slice() == wire_fp {
            return Ok(id.long_term_cert());
        }
    }
    Err(TokenError::UnknownSigner)
}
