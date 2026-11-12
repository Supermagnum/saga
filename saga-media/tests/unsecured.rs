//! Unsecured Iroh media: plain RTP policy + cross-mode rejection.

mod common;

use std::sync::Arc;

use common::{dummy_handshake, linked_pipelines, tone_pcm};
use iroh::endpoint::presets;
use iroh::{Endpoint, RelayMode};
use iroh_roq::{Session, ALPN};
use saga_media::{
    media_permit, CallRole, MediaCounters, MediaError, MediaPipeline, MediaWireMode,
    SecurityResolution, FRAME_SAMPLES, MEDIA_TAG_PLAIN, MEDIA_TAG_SRTP,
};

#[test]
fn policy_never_encrypted_allows_plain() {
    let mode = media_permit(SecurityResolution::NotEncrypted {
        prior_encrypted_history: false,
        user_continued_past_5a: false,
    })
    .unwrap();
    assert_eq!(mode, MediaWireMode::PlainRtp);
}

#[test]
fn policy_prior_encrypted_requires_5a_continue() {
    assert!(matches!(
        media_permit(SecurityResolution::NotEncrypted {
            prior_encrypted_history: true,
            user_continued_past_5a: false,
        }),
        Err(MediaError::MediaDeclined)
    ));
}

#[test]
fn policy_5a_continue_allows_plain() {
    let mode = media_permit(SecurityResolution::NotEncrypted {
        prior_encrypted_history: true,
        user_continued_past_5a: true,
    })
    .unwrap();
    assert_eq!(mode, MediaWireMode::PlainRtp);
}

#[test]
fn no_silent_downgrade_after_encrypted() {
    assert!(matches!(
        MediaPipeline::assert_no_silent_downgrade(true, MediaWireMode::PlainRtp),
        Err(MediaError::NoSilentDowngrade)
    ));
}

#[tokio::test]
async fn plain_rtp_roundtrip_after_not_encrypted() {
    let (mut caller, mut callee, _eps) = plain_pair().await;
    let pcm = tone_pcm(20, 1000.0);
    let frames = pcm.len() / FRAME_SAMPLES;
    let mut played = Vec::new();
    for f in 0..frames {
        let mut frame = pcm[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
        caller.send_pcm_frame(&mut frame).unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            callee.recv_one_datagram(),
        )
        .await
        .expect("timeout")
        .unwrap();
        let mut out = vec![0i16; FRAME_SAMPLES];
        let _ = callee.playout_frame(&mut out);
        played.extend_from_slice(&out);
    }
    assert!(caller.wire_mode() == MediaWireMode::PlainRtp);
    assert!(played.iter().any(|s| *s != 0));
}

#[tokio::test]
async fn plain_receiver_rejects_srtp_tagged() {
    let mut enc = linked_pipelines().await;
    let (_pc, mut plain, _keep) = plain_pair().await;
    let mut frame = tone_pcm(1, 440.0);
    let srtp_framed = enc.caller.protect_pcm_for_test(&mut frame).unwrap();
    assert_eq!(srtp_framed[0], MEDIA_TAG_SRTP);
    plain.inject_framed_for_test(&srtp_framed);
    assert!(
        plain.counters.snapshot().wrong_media_mode >= 1,
        "plain must reject SRTP-tagged datagrams"
    );
}

#[tokio::test]
async fn srtp_receiver_rejects_plain_tagged() {
    let mut pair = linked_pipelines().await;
    let (mut plain_caller, _plain_callee, _e) = plain_pair().await;
    let mut frame = tone_pcm(1, 440.0);
    let plain_framed = plain_caller.protect_pcm_for_test(&mut frame).unwrap();
    assert_eq!(plain_framed[0], MEDIA_TAG_PLAIN);
    pair.callee.inject_framed_for_test(&plain_framed);
    assert!(
        pair.counters_b.snapshot().wrong_media_mode >= 1,
        "SRTP pipeline must reject plain-tagged datagrams"
    );
}

#[tokio::test]
async fn start_declines_without_5a_continue() {
    let ep1 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let ep2 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let addr = ep2.addr();
    let flow = MediaPipeline::flow_id_hint();
    let accept = tokio::spawn({
        let ep2 = ep2.clone();
        async move {
            let incoming = ep2.accept().await.unwrap().await.unwrap();
            let session = Session::new(incoming);
            let send = session.new_send_flow(flow).await.unwrap();
            let recv = session.new_receive_flow(flow).await.unwrap();
            (session, send, recv)
        }
    });
    let conn = ep1.connect(addr, ALPN).await.unwrap();
    let session_a = Session::new(conn);
    let send_a = session_a.new_send_flow(flow).await.unwrap();
    let recv_a = session_a.new_receive_flow(flow).await.unwrap();
    let (_session_b, _send_b, _recv_b) = accept.await.unwrap();
    let hs = dummy_handshake(1);
    let started = MediaPipeline::start(
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: true,
            user_continued_past_5a: false,
        },
        Some(&hs),
        CallRole::Caller,
        send_a,
        recv_a,
        Arc::new(MediaCounters::default()),
    );
    assert!(
        matches!(started, Err(MediaError::MediaDeclined)),
        "expected MediaDeclined"
    );
    drop(session_a);
    drop(ep1);
    drop(ep2);
}

async fn plain_pair() -> (
    MediaPipeline,
    MediaPipeline,
    (Endpoint, Endpoint, Session, Session),
) {
    let ep1 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let ep2 = Endpoint::builder(presets::Minimal)
        .alpns(vec![ALPN.to_vec()])
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await
        .unwrap();
    let addr = ep2.addr();
    let flow = MediaPipeline::flow_id_hint();
    let accept = tokio::spawn({
        let ep2 = ep2.clone();
        async move {
            let incoming = ep2.accept().await.unwrap().await.unwrap();
            let session = Session::new(incoming);
            let send = session.new_send_flow(flow).await.unwrap();
            let recv = session.new_receive_flow(flow).await.unwrap();
            (session, send, recv)
        }
    });
    let conn = ep1.connect(addr, ALPN).await.unwrap();
    let session_a = Session::new(conn);
    let send_a = session_a.new_send_flow(flow).await.unwrap();
    let recv_a = session_a.new_receive_flow(flow).await.unwrap();
    let (session_b, send_b, recv_b) = accept.await.unwrap();
    let ca = Arc::new(MediaCounters::default());
    let cb = Arc::new(MediaCounters::default());
    let caller = MediaPipeline::start(
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: false,
            user_continued_past_5a: false,
        },
        None,
        CallRole::Caller,
        send_a,
        recv_a,
        ca,
    )
    .unwrap();
    let callee = MediaPipeline::new_plain(send_b, recv_b, cb).unwrap();
    (caller, callee, (ep1, ep2, session_a, session_b))
}
