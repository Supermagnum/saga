//! §5b re-key swap timing (host).

mod common;

use common::{dummy_handshake, linked_pipelines, tone_pcm};
use saga_media::CallRole;

#[tokio::test]
async fn rekey_swap_under_100ms() {
    let mut pair = linked_pipelines().await;
    // Prime one packet so contexts are live.
    let mut frame = tone_pcm(1, 440.0);
    let _ = pair.caller.protect_pcm_for_test(&mut frame).unwrap();

    let new_hs = dummy_handshake(42);
    let us = pair
        .caller
        .rekey_srtp(&new_hs, CallRole::Caller)
        .expect("rekey");
    eprintln!("rekey_swap_us={us}");
    assert!(
        us < 100_000,
        "rekey swap {us} us exceeds 100 ms audible-gap budget"
    );
    pair.caller.clear_prev_decrypt();
}
