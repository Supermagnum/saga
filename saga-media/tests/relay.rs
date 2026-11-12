//! Round-trip via local iroh relay with direct IP transports disabled.

mod common;

use std::sync::Arc;

use common::{dummy_handshake, tone_pcm};
use iroh::endpoint::presets;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, RelayMode};
use iroh_roq::{Session, ALPN};
use saga_media::{signal_correlation, CallRole, MediaCounters, MediaPipeline, FRAME_SAMPLES};

#[tokio::test]
async fn roundtrip_over_relay() {
    let (relay_map, _relay_url, _guard) = iroh::test_utils::run_relay_server()
        .await
        .expect("start test relay");

    let ep1 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Custom(relay_map.clone()))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .clear_ip_transports()
        .bind()
        .await
        .expect("ep1");
    let ep2 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Custom(relay_map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify())
        .clear_ip_transports()
        .bind()
        .await
        .expect("ep2");

    ep2.online().await;
    let addr = ep2.addr();
    let flow = MediaPipeline::flow_id_hint();
    let hs = dummy_handshake(11);

    let accept = tokio::spawn({
        let ep2 = ep2.clone();
        async move {
            let incoming = ep2.accept().await.expect("incoming").await.expect("accept");
            let session = Session::new(incoming);
            let send = session.new_send_flow(flow).await.unwrap();
            let recv = session.new_receive_flow(flow).await.unwrap();
            (session, send, recv)
        }
    });

    let conn = ep1.connect(addr, ALPN).await.expect("connect via relay");
    let session_a = Session::new(conn);
    let send_a = session_a.new_send_flow(flow).await.unwrap();
    let recv_a = session_a.new_receive_flow(flow).await.unwrap();
    let (session_b, send_b, recv_b) = accept.await.unwrap();

    let ca = Arc::new(MediaCounters::default());
    let cb = Arc::new(MediaCounters::default());
    let mut caller = MediaPipeline::new(&hs, CallRole::Caller, send_a, recv_a, ca).unwrap();
    let mut callee = MediaPipeline::new(&hs, CallRole::Callee, send_b, recv_b, cb).unwrap();

    let pcm = tone_pcm(40, 1000.0);
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut played = Vec::new();
    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        caller.send_pcm_frame(&mut frame).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            callee.recv_one_datagram(),
        )
        .await
        .expect("relay recv timeout")
        .unwrap();
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    for _ in 0..16 {
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    let n = pcm.len().min(played.len());
    let corr = signal_correlation(&pcm[..n], &played[..n], FRAME_SAMPLES * 4);
    eprintln!("relay corr={corr:.3}");
    assert!(corr >= 0.85, "relay corr {corr} too low");

    // Keep sessions/endpoints alive through the assertion above.
    drop(session_a);
    drop(session_b);
    drop(ep1);
    drop(ep2);
}
