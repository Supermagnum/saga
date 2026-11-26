//! Saga-to-Saga encrypted text over Iroh ALPN `saga/msg/1` (Part 3 / issue #41).
//!
//! Full send/receive path is available with `mock-token` (E2E / debug). Release
//! `nitrokey` builds advertise the ALPN; without a token session the send path
//! returns `TOKEN_NEEDED` and never sends plaintext.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use iroh::endpoint::Connection;
use log::info;
use saga_msg::{MAX_TEXT_BYTES, SAGA_MSG_ALPN};

#[cfg(feature = "mock-token")]
use crate::iroh_transport::runtime;

static FORCE_TOKEN_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

type DeliveredFn = Box<dyn Fn(String, String) + Send + Sync>;
static ON_DELIVERED: OnceLock<DeliveredFn> = OnceLock::new();

pub fn set_force_token_unavailable(force: bool) {
    FORCE_TOKEN_UNAVAILABLE.store(force, Ordering::SeqCst);
}

pub fn force_token_unavailable() -> bool {
    FORCE_TOKEN_UNAVAILABLE.load(Ordering::SeqCst)
}

pub fn set_on_delivered<F>(f: F)
where
    F: Fn(String, String) + Send + Sync + 'static,
{
    let _ = ON_DELIVERED.set(Box::new(f));
}

#[cfg(feature = "mock-token")]
fn notify_delivered(peer: &str, text: &str) {
    if let Some(cb) = ON_DELIVERED.get() {
        cb(peer.to_string(), text.to_string());
    }
}

pub fn send_text(peer_id: &str, text: &str) -> String {
    if force_token_unavailable() {
        return "TOKEN_NEEDED".into();
    }
    if text.len() > MAX_TEXT_BYTES {
        return "TOO_LARGE".into();
    }
    #[cfg(not(feature = "mock-token"))]
    {
        let _ = peer_id;
        "TOKEN_NEEDED".into()
    }
    #[cfg(feature = "mock-token")]
    {
        let peer = peer_id.to_string();
        let body = text.to_string();
        match runtime().block_on(async move { send_text_async(&peer, &body).await }) {
            Ok(s) => s,
            Err(e) => e,
        }
    }
}

pub async fn handle_inbound(conn: Connection) {
    info!("[Saga Msg] inbound from {}", conn.remote_id());
    if force_token_unavailable() {
        conn.close(0u32.into(), b"saga token needed");
        return;
    }
    #[cfg(not(feature = "mock-token"))]
    {
        conn.close(0u32.into(), b"saga token needed");
    }
    #[cfg(feature = "mock-token")]
    {
        handle_inbound_mock(conn).await;
    }
}

pub fn msg_alpn() -> &'static [u8] {
    SAGA_MSG_ALPN
}

#[cfg(feature = "mock-token")]
mod mock_path {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    use iroh::endpoint::Connection;
    use log::{info, warn};
    use mock_token::{
        run_initiator_with_options, run_responder_with_options, HandshakeOutcome, MockIdentity,
    };
    use saga_msg::{
        open_message, seal_message, MsgKeyPair, MsgRole, ReplayWindow, SealedMessage, FRAME_DATA,
        FRAME_DELIVERED, SAGA_MSG_ALPN,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::notify_delivered;
    use crate::iroh_transport::{
        bind_shared_endpoint, dial_with_retry, ensure_relay_before_dial, local_identity_label,
        parse_peer_endpoint_id,
    };

    static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);

    async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, body: &[u8]) -> Result<(), String> {
        if body.len() > u32::MAX as usize {
            return Err("frame too large".into());
        }
        let len = (body.len() as u32).to_be_bytes();
        w.write_all(&len)
            .await
            .map_err(|e| format!("write len: {e}"))?;
        w.write_all(body)
            .await
            .map_err(|e| format!("write body: {e}"))?;
        w.flush().await.map_err(|e| format!("flush: {e}"))?;
        Ok(())
    }

    async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Vec<u8>, String> {
        let mut len_buf = [0u8; 4];
        r.read_exact(&mut len_buf)
            .await
            .map_err(|e| format!("read len: {e}"))?;
        let len = u32::from_be_bytes(len_buf) as usize;
        if len == 0 || len > 64 * 1024 {
            return Err("bad frame length".into());
        }
        let mut buf = vec![0u8; len];
        r.read_exact(&mut buf)
            .await
            .map_err(|e| format!("read body: {e}"))?;
        Ok(buf)
    }

    pub(super) async fn send_text_async(peer_id: &str, text: &str) -> Result<String, String> {
        let remote = parse_peer_endpoint_id(peer_id)?;
        let endpoint = bind_shared_endpoint()?;
        let _ = ensure_relay_before_dial(endpoint, peer_id).await;
        let connection = dial_with_retry(endpoint, remote, peer_id, SAGA_MSG_ALPN, None)
            .await
            .map_err(|e| format!("NOT_REACHABLE:{e}"))?;

        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("msg open_bi: {e}"))?;
        let identity = MockIdentity::from_label(&local_identity_label());
        let local_eid = *endpoint.id().as_bytes();
        let remote_eid = *connection.remote_id().as_bytes();
        let outcome = run_initiator_with_options(
            &identity,
            peer_id,
            &local_eid,
            &remote_eid,
            false,
            false,
            &mut recv,
            &mut send,
        )
        .await;
        let output = match outcome {
            HandshakeOutcome::Encrypted { output, .. } => output,
            HandshakeOutcome::Failed { reason } => {
                return Err(format!("HANDSHAKE_FAILED:{reason}"));
            }
        };

        let keys = MsgKeyPair::derive(&output);
        let (send_key, recv_key) = keys.for_role(MsgRole::Initiator);
        let seq = NEXT_SEQ.fetch_add(1, Ordering::SeqCst);
        let sealed = seal_message(send_key, seq, FRAME_DATA, text.as_bytes())
            .map_err(|e| format!("seal: {e}"))?;
        write_frame(&mut send, &sealed.encode()).await?;

        let mut replay = ReplayWindow::new();
        let ack_result = tokio::time::timeout(Duration::from_secs(8), async {
            let frame = read_frame(&mut recv).await?;
            let sealed = SealedMessage::decode(&frame).map_err(|e| format!("ack decode: {e}"))?;
            if sealed.frame_type != FRAME_DELIVERED {
                return Err("unexpected frame".into());
            }
            if !replay.accept(sealed.seq, &sealed.msg_id) {
                return Err("ack replay".into());
            }
            let _plain = open_message(recv_key, &sealed).map_err(|e| format!("ack open: {e}"))?;
            Ok::<(), String>(())
        })
        .await;

        let _ = send.finish();
        drop(keys);
        drop(output);

        match ack_result {
            Ok(Ok(())) => Ok("delivered".into()),
            Ok(Err(e)) => {
                warn!("msg ack error: {e}");
                Ok("sent".into())
            }
            Err(_) => Ok("sent".into()),
        }
    }

    pub(super) async fn handle_inbound_mock(conn: Connection) {
        let endpoint = match bind_shared_endpoint() {
            Ok(e) => e,
            Err(e) => {
                warn!("[Saga Msg] no endpoint: {e}");
                conn.close(0u32.into(), b"saga no endpoint");
                return;
            }
        };
        let (mut send, mut recv) = match conn.accept_bi().await {
            Ok(v) => v,
            Err(e) => {
                warn!("[Saga Msg] accept_bi: {e}");
                return;
            }
        };
        let identity = MockIdentity::from_label(&local_identity_label());
        let local_eid = *endpoint.id().as_bytes();
        let remote_eid = *conn.remote_id().as_bytes();
        let outcome = run_responder_with_options(
            &identity,
            &local_eid,
            &remote_eid,
            false,
            false,
            &mut recv,
            &mut send,
        )
        .await;
        let (output, peer_label) = match outcome {
            HandshakeOutcome::Encrypted {
                output,
                peer_fingerprint_hex,
            } => {
                let peer_label = crate::iroh_transport::dev_label_for_endpoint(&conn.remote_id())
                    .unwrap_or(peer_fingerprint_hex);
                (output, peer_label)
            }
            HandshakeOutcome::Failed { reason } => {
                warn!("[Saga Msg] inbound handshake failed: {reason}");
                conn.close(0u32.into(), b"saga msg handshake fail");
                return;
            }
        };

        let keys = MsgKeyPair::derive(&output);
        let (send_key, recv_key) = keys.for_role(MsgRole::Responder);
        let mut replay = ReplayWindow::new();

        let frame = match read_frame(&mut recv).await {
            Ok(f) => f,
            Err(e) => {
                warn!("[Saga Msg] read frame: {e}");
                return;
            }
        };
        let sealed = match SealedMessage::decode(&frame) {
            Ok(s) => s,
            Err(e) => {
                warn!("[Saga Msg] decode: {e}");
                return;
            }
        };
        if sealed.frame_type != FRAME_DATA {
            warn!("[Saga Msg] unexpected frame type {}", sealed.frame_type);
            return;
        }
        if !replay.accept(sealed.seq, &sealed.msg_id) {
            warn!("[Saga Msg] replay/duplicate rejected");
            return;
        }
        let plain = match open_message(recv_key, &sealed) {
            Ok(p) => p,
            Err(e) => {
                warn!("[Saga Msg] open failed (tamper?): {e}");
                return;
            }
        };
        let text = match String::from_utf8(plain) {
            Ok(t) => t,
            Err(_) => {
                warn!("[Saga Msg] non-utf8 payload rejected");
                return;
            }
        };
        info!(
            "[Saga Msg] delivered from peer_label=[{peer_label}] bytes={}",
            text.len()
        );
        notify_delivered(&peer_label, &text);

        let ack_seq = NEXT_SEQ.fetch_add(1, Ordering::SeqCst);
        match seal_message(send_key, ack_seq, FRAME_DELIVERED, &sealed.msg_id) {
            Ok(ack) => {
                let _ = write_frame(&mut send, &ack.encode()).await;
                let _ = send.finish();
                let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
            }
            Err(e) => warn!("[Saga Msg] ack seal failed: {e}"),
        }

        drop(keys);
        drop(output);
    }
}

#[cfg(feature = "mock-token")]
use mock_path::{handle_inbound_mock, send_text_async};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alpn_distinct_from_voice() {
        assert_ne!(SAGA_MSG_ALPN, crate::iroh_transport::SAGA_VOICE_ALPN);
        assert_eq!(SAGA_MSG_ALPN, b"saga/msg/1");
    }

    #[test]
    fn token_fault_gate() {
        set_force_token_unavailable(true);
        assert_eq!(send_text("15550100010", "hi"), "TOKEN_NEEDED");
        set_force_token_unavailable(false);
    }
}
