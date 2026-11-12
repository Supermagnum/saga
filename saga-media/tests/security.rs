//! Tamper / replay / wrong-key counters.

mod common;

use common::{dummy_handshake, linked_pipelines, tone_pcm};
use saga_media::{CallRole, SrtpDecrypt, SrtpKeyPair};

#[tokio::test]
async fn tamper_increments_auth_fail() {
    let mut pair = linked_pipelines().await;
    let mut frame = tone_pcm(1, 440.0);
    let protected = pair.caller.protect_pcm_for_test(&mut frame).unwrap();
    let mut bytes = protected.to_vec();
    let idx = bytes.len() / 2;
    bytes[idx] ^= 0xff;
    pair.callee.inject_srtp_for_test(&bytes);
    let snap = pair.counters_b.snapshot();
    assert!(
        snap.tamper_or_auth_fail >= 1,
        "expected auth fail, got {snap:?}"
    );
}

#[tokio::test]
async fn replay_increments_replay_counter() {
    let mut pair = linked_pipelines().await;
    let mut frame = tone_pcm(1, 440.0);
    let protected = pair.caller.protect_pcm_for_test(&mut frame).unwrap();
    pair.callee.inject_srtp_for_test(&protected);
    pair.callee.inject_srtp_for_test(&protected);
    let snap = pair.counters_b.snapshot();
    assert!(snap.replay >= 1, "expected replay, got {snap:?}");
}

#[tokio::test]
async fn wrong_key_rejects_all() {
    let mut pair = linked_pipelines().await;
    let hs_wrong = dummy_handshake(99);
    let wrong_keys = SrtpKeyPair::derive(&hs_wrong);
    let (_out, inn) = wrong_keys.for_role(CallRole::Callee);
    let mut dec = SrtpDecrypt::new(inn).unwrap();
    let mut fails = 0u64;
    for _ in 0..5 {
        let mut f = tone_pcm(1, 880.0);
        let p = pair.caller.protect_pcm_for_test(&mut f).unwrap();
        if dec.unprotect(&p).is_err() {
            fails += 1;
        }
    }
    assert_eq!(fails, 5, "wrong key must reject every packet");
}
