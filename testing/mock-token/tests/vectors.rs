//! Fixed test vectors from pinned Galdralag-firmware rev
//! `b781bf6fa5ccf8700a5fb061583c31cfa00241a1` (2026-10-22).
//!
//! Vector generation mirrors `ephemeral-session` integration tests
//! (`crates/ephemeral-session/tests/session_protocol.rs`) using FakeTrng seeds
//! and the crate's own InitMessage / ResponseMessage serialisation.
//!
//! Firmware KDF cross-check: `ephemeral-session/examples/export_session_kdf_vectors.rs`
//! (FakeTrng seeds 7/8, BrainpoolP256r1) — cited for KDF shape; handshake
//! vectors below are Saga-owned against the same pinned rev.

use ephemeral_session::{InitMessage, ResponseMessage, SessionCurve};
use mock_token::{
    bind_iroh_endpoints, transcript_hash, MockIdentity, MockSessionToken, SessionToken,
    HANDSHAKE_HARD_TIMEOUT,
};

const ENDPOINT_A: &[u8] = b"saga-test-endpoint-aaaaaaaa";
const ENDPOINT_B: &[u8] = b"saga-test-endpoint-bbbbbbbb";

/// Fixed P-384 handshake: LT from labels + FakeTrng ephemeral seeds 0xC1 / 0xD1
/// (same seed constants as firmware `test_full_handshake_brainpool384r1`).
#[tokio::test]
async fn vector_p384_init_response_shared_secret_byte_equal() {
    let alice = MockIdentity::from_label("vector-alice");
    let bob = MockIdentity::from_label("vector-bob");
    let init_token = MockSessionToken::with_ephemeral_seed(alice.clone(), 0xC1);
    let resp_token = MockSessionToken::with_ephemeral_seed(bob.clone(), 0xD1);

    let (init, pending) = init_token
        .initiator_offer(SessionCurve::BrainpoolP384r1, HANDSHAKE_HARD_TIMEOUT)
        .await
        .expect("offer");
    let init_bytes = init.serialise().expect("ser init").as_slice().to_vec();

    let (resp, out_r) = resp_token
        .responder_reply(
            InitMessage::parse(&init_bytes).expect("parse init"),
            alice.long_term_cert(),
            ENDPOINT_B,
            ENDPOINT_A,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .expect("respond");
    let resp_bytes = resp.serialise().expect("ser resp").as_slice().to_vec();

    let out_i = init_token
        .initiator_finish(
            pending,
            ResponseMessage::parse(&resp_bytes).expect("parse resp"),
            bob.long_term_cert(),
            ENDPOINT_A,
            ENDPOINT_B,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .expect("finish");

    assert_eq!(
        out_i.shared_secret.as_slice(),
        out_r.shared_secret.as_slice()
    );
    assert_eq!(out_i.transcript_hash, out_r.transcript_hash);
    assert_eq!(
        out_i.saga_session_key.as_slice(),
        out_r.saga_session_key.as_slice()
    );

    // Committed expected bytes (regenerate only when changing the pin rev).
    let expected = include_str!("fixtures/p384_vector_c1_d1.json");
    let v: serde_json::Value = serde_json::from_str(expected).expect("fixture json");
    assert_eq!(
        hex::encode(&init_bytes),
        v["init_message_hex"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(&resp_bytes),
        v["response_message_hex"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(out_i.shared_secret.as_slice()),
        v["shared_secret_hex"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(out_i.transcript_hash),
        v["transcript_hash_hex"].as_str().unwrap()
    );
    assert_eq!(
        hex::encode(out_i.saga_session_key.as_slice()),
        v["saga_session_key_hex"].as_str().unwrap()
    );

    // Sanity: transcript matches ordered-EPK SHA-256 used as HKDF salt input hash.
    let th = transcript_hash(
        init.ephemeral_public_key.as_slice(),
        resp.ephemeral_public_key.as_slice(),
    );
    assert_eq!(th, out_i.transcript_hash);

    let bound = bind_iroh_endpoints(&out_i.profile_prk, ENDPOINT_A, ENDPOINT_B);
    assert_eq!(bound.as_slice(), out_i.saga_session_key.as_slice());
}

#[tokio::test]
async fn vector_p256_round_trip_both_curves_supported() {
    let curve = SessionCurve::BrainpoolP256r1;
    let alice = MockIdentity::from_label_curve("vector-alice", curve);
    let bob = MockIdentity::from_label_curve("vector-bob", curve);
    let init_token = MockSessionToken::with_ephemeral_seed(alice.clone(), 0xC0);
    let resp_token = MockSessionToken::with_ephemeral_seed(bob.clone(), 0xD0);

    let (init, pending) = init_token
        .initiator_offer(curve, HANDSHAKE_HARD_TIMEOUT)
        .await
        .expect("offer");
    let (resp, out_r) = resp_token
        .responder_reply(
            init,
            alice.long_term_cert(),
            ENDPOINT_B,
            ENDPOINT_A,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .expect("respond");
    let out_i = init_token
        .initiator_finish(
            pending,
            resp,
            bob.long_term_cert(),
            ENDPOINT_A,
            ENDPOINT_B,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .expect("finish");
    assert_eq!(
        out_i.saga_session_key.as_slice(),
        out_r.saga_session_key.as_slice()
    );
}
