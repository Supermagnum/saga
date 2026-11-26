//! Minimal `msg send` / `msg listen` for host tests (issue #41).
//!
//! Handshake + first data/ack share one bi-stream.

use std::time::Duration;

use anyhow::{Context, Result};
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl};
use log::info;
use mock_token::{run_initiator, run_responder, HandshakeOutcome, MockIdentity};
use saga_msg::{
    open_message, seal_message, MsgKeyPair, MsgRole, SealedMessage, FRAME_DATA, FRAME_DELIVERED,
    SAGA_MSG_ALPN,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::call::resolve_relay_url;
use crate::identity::{dev_secret_key, resolve_peer_label};

async fn write_frame<W: AsyncWriteExt + Unpin>(w: &mut W, body: &[u8]) -> Result<()> {
    let len = (body.len() as u32).to_be_bytes();
    w.write_all(&len).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn bind_endpoint(
    identity: &str,
    relay: Option<&str>,
    public_relay: bool,
) -> Result<Endpoint> {
    let mut builder = Endpoint::builder(presets::N0)
        .alpns(vec![SAGA_MSG_ALPN.to_vec()])
        .secret_key(dev_secret_key(identity));
    if !public_relay {
        let url = resolve_relay_url(relay);
        info!("msg using relay url=[{url}]");
        let relay_url: RelayUrl = url.parse().context("relay url")?;
        let map = RelayMap::from_iter([relay_url]);
        builder = builder
            .relay_mode(RelayMode::Custom(map))
            .ca_tls_config(CaTlsConfig::insecure_skip_verify());
    }
    Ok(builder.bind().await?)
}

fn resolve_endpoint_id(peer: &str) -> Result<PublicKey> {
    if let Ok(pk) = peer.parse::<PublicKey>() {
        return Ok(pk);
    }
    let label = resolve_peer_label(peer);
    Ok(dev_secret_key(&label).public())
}

pub async fn run_msg_listen(
    identity: &str,
    relay: Option<&str>,
    public_relay: bool,
    max_secs: Option<u64>,
) -> Result<String> {
    let endpoint = bind_endpoint(identity, relay, public_relay).await?;
    endpoint.online().await;
    info!("msg listen id={} waiting", endpoint.id());
    let deadline = max_secs.map(|s| tokio::time::Instant::now() + Duration::from_secs(s));
    loop {
        if let Some(d) = deadline {
            if tokio::time::Instant::now() >= d {
                anyhow::bail!("listen timed out");
            }
        }
        let incoming = endpoint.accept().await.context("accept")?;
        let conn = incoming.await.context("inbound")?;
        if conn.alpn() != SAGA_MSG_ALPN {
            continue;
        }
        let identity_m = MockIdentity::from_label(identity);
        let (mut send, mut recv) = conn.accept_bi().await?;
        let local = *endpoint.id().as_bytes();
        let remote = *conn.remote_id().as_bytes();
        let outcome =
            run_responder(&identity_m, &local, &remote, false, &mut recv, &mut send).await;
        let HandshakeOutcome::Encrypted { output, .. } = outcome else {
            anyhow::bail!("handshake failed");
        };
        let keys = MsgKeyPair::derive(&output);
        let (send_key, recv_key) = keys.for_role(MsgRole::Responder);
        let frame = read_frame(&mut recv).await?;
        let sealed = SealedMessage::decode(&frame)?;
        let plain = open_message(recv_key, &sealed)?;
        let text = String::from_utf8(plain)?;
        let ack = seal_message(send_key, 1, FRAME_DELIVERED, &sealed.msg_id)?;
        write_frame(&mut send, &ack.encode()).await?;
        let _ = send.finish();
        let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
        info!("msg listen delivered bytes={}", text.len());
        println!("{text}");
        return Ok(text);
    }
}

pub async fn run_msg_send(
    identity: &str,
    peer: &str,
    text: &str,
    relay: Option<&str>,
    public_relay: bool,
) -> Result<String> {
    let endpoint = bind_endpoint(identity, relay, public_relay).await?;
    endpoint.online().await;
    let peer_label = resolve_peer_label(peer);
    let remote_pk = resolve_endpoint_id(peer)?;
    let mut last = String::new();
    for _ in 0..40 {
        let addr = EndpointAddr::new(remote_pk);
        match tokio::time::timeout(
            Duration::from_secs(3),
            endpoint.connect(addr, SAGA_MSG_ALPN),
        )
        .await
        {
            Ok(Ok(conn)) => {
                let identity_m = MockIdentity::from_label(identity);
                let (mut send, mut recv) = conn.open_bi().await?;
                let local = *endpoint.id().as_bytes();
                let remote = *conn.remote_id().as_bytes();
                let outcome = run_initiator(
                    &identity_m,
                    &peer_label,
                    &local,
                    &remote,
                    false,
                    &mut recv,
                    &mut send,
                )
                .await;
                let HandshakeOutcome::Encrypted { output, .. } = outcome else {
                    anyhow::bail!("handshake failed");
                };
                let keys = MsgKeyPair::derive(&output);
                let (send_key, recv_key) = keys.for_role(MsgRole::Initiator);
                let sealed = seal_message(send_key, 1, FRAME_DATA, text.as_bytes())?;
                write_frame(&mut send, &sealed.encode()).await?;
                let frame = tokio::time::timeout(Duration::from_secs(8), read_frame(&mut recv))
                    .await
                    .context("ack timeout")??;
                let ack = SealedMessage::decode(&frame)?;
                let _ = open_message(recv_key, &ack)?;
                let _ = send.finish();
                info!("msg send delivered to {peer_label}");
                return Ok("delivered".into());
            }
            Ok(Err(e)) => last = e.to_string(),
            Err(_) => last = "connect timeout".into(),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("NOT_REACHABLE: {last}")
}

/// In-process host round-trip (no external relay).
pub async fn host_msg_roundtrip(text: &str) -> Result<String> {
    let label_a = "15550100011";
    let label_b = "15550100010";
    let ep_a = Endpoint::builder(presets::Minimal)
        .alpns(vec![SAGA_MSG_ALPN.to_vec()])
        .secret_key(dev_secret_key(label_a))
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let ep_b = Endpoint::builder(presets::Minimal)
        .alpns(vec![SAGA_MSG_ALPN.to_vec()])
        .secret_key(dev_secret_key(label_b))
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?;
    let addr_b = ep_b.addr();
    let listen = tokio::spawn({
        let ep_b = ep_b.clone();
        let expect = text.to_string();
        async move {
            let incoming = ep_b.accept().await.expect("accept").await.expect("conn");
            assert_eq!(incoming.alpn(), SAGA_MSG_ALPN);
            let identity = MockIdentity::from_label(label_b);
            let (mut send, mut recv) = incoming.accept_bi().await.expect("bi");
            let local = *ep_b.id().as_bytes();
            let remote = *incoming.remote_id().as_bytes();
            let outcome =
                run_responder(&identity, &local, &remote, false, &mut recv, &mut send).await;
            let HandshakeOutcome::Encrypted { output, .. } = outcome else {
                panic!("handshake");
            };
            let keys = MsgKeyPair::derive(&output);
            let (send_key, recv_key) = keys.for_role(MsgRole::Responder);
            let frame = read_frame(&mut recv).await.expect("frame");
            let sealed = SealedMessage::decode(&frame).expect("decode");
            let plain = open_message(recv_key, &sealed).expect("open");
            let got = String::from_utf8(plain).expect("utf8");
            assert_eq!(got, expect);
            let ack = seal_message(send_key, 1, FRAME_DELIVERED, &sealed.msg_id).expect("ack");
            write_frame(&mut send, &ack.encode())
                .await
                .expect("ack write");
            let _ = send.finish();
            // Keep the connection alive until the initiator has read the ack.
            let _ = tokio::time::timeout(Duration::from_secs(2), send.stopped()).await;
            got
        }
    });

    let conn = ep_a.connect(addr_b, SAGA_MSG_ALPN).await?;
    let identity = MockIdentity::from_label(label_a);
    let (mut send, mut recv) = conn.open_bi().await?;
    let local = *ep_a.id().as_bytes();
    let remote = *conn.remote_id().as_bytes();
    let outcome = run_initiator(
        &identity, label_b, &local, &remote, false, &mut recv, &mut send,
    )
    .await;
    let HandshakeOutcome::Encrypted { output, .. } = outcome else {
        anyhow::bail!("handshake failed");
    };
    let keys = MsgKeyPair::derive(&output);
    let (send_key, recv_key) = keys.for_role(MsgRole::Initiator);
    let sealed = seal_message(send_key, 1, FRAME_DATA, text.as_bytes())?;
    write_frame(&mut send, &sealed.encode()).await?;
    let frame = read_frame(&mut recv).await?;
    let ack = SealedMessage::decode(&frame)?;
    let _ = open_message(recv_key, &ack)?;
    let _ = send.finish();
    Ok(listen.await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn host_msg_roundtrip_ok() {
        let got = host_msg_roundtrip("ping-msg").await.expect("roundtrip");
        assert_eq!(got, "ping-msg");
    }
}
