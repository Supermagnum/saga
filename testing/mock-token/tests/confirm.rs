//! Host tests for Saga confirmation MAC (channel-binding agreement).

use ephemeral_session::SessionCurve;
use mock_token::{
    initiator_mac, responder_mac, run_initiator_with_options, run_responder, verify_mac,
    MockIdentity, MockSessionToken, SessionToken, TokenError, HANDSHAKE_HARD_TIMEOUT,
};
use tokio::io::duplex;

const EA: &[u8] = b"endpoint-aaa-aaaaaaaa";
const EB: &[u8] = b"endpoint-bbb-bbbbbbbb";
const EC: &[u8] = b"endpoint-ccc-cccccccc";

#[tokio::test]
async fn mismatched_endpoint_sets_fail_confirmation() {
    let alice = MockIdentity::from_label("15550100011");
    let bob = MockIdentity::from_label("15550100010");

    let (io_a, io_b) = duplex(8192);
    let (mut a_read, mut a_write) = tokio::io::split(io_a);
    let (mut b_read, mut b_write) = tokio::io::split(io_b);

    // Alice binds (EA, EB); Bob binds (EA, EC) — same Init/Response crypto, different Saga keys.
    let (a_out, b_out) = tokio::join!(
        run_initiator_with_options(
            &alice,
            "15550100010",
            EA,
            EB,
            false,
            false,
            &mut a_read,
            &mut a_write
        ),
        run_responder(&bob, EC, EA, false, &mut b_read, &mut b_write),
    );

    assert!(
        matches!(a_out, mock_token::HandshakeOutcome::Failed { .. })
            || matches!(b_out, mock_token::HandshakeOutcome::Failed { .. }),
        "expected confirmation failure: {a_out:?} {b_out:?}"
    );
    let failed = [&a_out, &b_out].iter().any(|o| match o {
        mock_token::HandshakeOutcome::Failed { reason } => {
            reason.contains("confirmation") || reason.contains("Confirmation")
        }
        _ => false,
    });
    assert!(
        failed || matches!(a_out, mock_token::HandshakeOutcome::Failed { .. }),
        "expected typed confirmation failure: {a_out:?} {b_out:?}"
    );
}

#[tokio::test]
async fn tampered_mac_fails_verification() {
    let alice = MockIdentity::from_label("vector-alice");
    let bob = MockIdentity::from_label("vector-bob");
    let init_token = MockSessionToken::with_ephemeral_seed(alice.clone(), 0xC1);
    let resp_token = MockSessionToken::with_ephemeral_seed(bob.clone(), 0xD1);

    let (init, pending) = init_token
        .initiator_offer(SessionCurve::BrainpoolP384r1, HANDSHAKE_HARD_TIMEOUT)
        .await
        .unwrap();
    let (resp, out_r) = resp_token
        .responder_reply(init, alice.long_term_cert(), EB, EA, HANDSHAKE_HARD_TIMEOUT)
        .await
        .unwrap();
    let out_i = init_token
        .initiator_finish(
            pending,
            resp,
            bob.long_term_cert(),
            EA,
            EB,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .unwrap();

    assert_eq!(
        out_i.confirmation_key.as_slice(),
        out_r.confirmation_key.as_slice()
    );
    let key = mock_token::ConfirmationKey::from_bytes(*out_i.confirmation_key);
    let mut mac = initiator_mac(&key, &out_i.transcript_hash);
    mac[0] ^= 0xff;
    let err = verify_mac(&initiator_mac(&key, &out_i.transcript_hash), &mac).unwrap_err();
    assert_eq!(err, TokenError::ConfirmationFailed);

    // Peer MAC role mismatch also fails.
    let wrong_role = responder_mac(&key, &out_i.transcript_hash);
    let err = verify_mac(&initiator_mac(&key, &out_i.transcript_hash), &wrong_role).unwrap_err();
    assert_eq!(err, TokenError::ConfirmationFailed);
}

#[tokio::test]
async fn perturb_binding_on_one_side_fails() {
    let alice = MockIdentity::from_label("15550100011");
    let bob = MockIdentity::from_label("15550100010");

    let (io_a, io_b) = duplex(8192);
    let (mut a_read, mut a_write) = tokio::io::split(io_a);
    let (mut b_read, mut b_write) = tokio::io::split(io_b);

    let (a_out, b_out) = tokio::join!(
        run_initiator_with_options(
            &alice,
            "15550100010",
            EA,
            EB,
            false,
            true, // perturb alice binding
            &mut a_read,
            &mut a_write
        ),
        run_responder(&bob, EB, EA, false, &mut b_read, &mut b_write),
    );

    assert!(
        matches!(a_out, mock_token::HandshakeOutcome::Failed { .. })
            && matches!(b_out, mock_token::HandshakeOutcome::Failed { .. }),
        "perturb should fail both sides: {a_out:?} {b_out:?}"
    );
}
