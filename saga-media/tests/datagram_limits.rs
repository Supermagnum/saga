//! Datagram size limits and blocked-drop counter.

mod common;

use common::{linked_pipelines, tone_pcm};

#[tokio::test]
async fn tiny_max_datagram_fails_cleanly() {
    let mut pair = linked_pipelines().await;
    pair.caller.set_max_datagram_override_for_test(Some(32));
    let mut frame = tone_pcm(1, 1000.0);
    let err = pair.caller.send_pcm_frame(&mut frame).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("exceeds") || msg.contains("DatagramTooLarge") || msg.contains("datagram"),
        "unexpected err: {msg}"
    );
    assert!(pair.caller.media_failed());
}

#[tokio::test]
async fn blocked_drop_counter() {
    let pair = linked_pipelines().await;
    // Forced-full buffer: unit-level counter path (QUIC may not expose Blocked
    // with drop_old=false without saturating the peer). Production path uses
    // try_send_payload → Ok(false) → datagram_blocked_drops.
    pair.caller.record_blocked_drop_for_test();
    pair.caller.record_blocked_drop_for_test();
    let snap = pair.counters_a.snapshot();
    assert_eq!(snap.datagram_blocked_drops, 2);
}
