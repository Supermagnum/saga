//! NitrokeySessionToken (software AUT) ↔ MockSessionToken interop.

use std::time::Duration;

use mock_token::{MockIdentity, MockSessionToken};
use saga_session::{SessionCurve, SessionToken, HANDSHAKE_HARD_TIMEOUT};
use saga_token_usb::test_card::software_nitrokey_token_from_seed;

const TIMEOUT: Duration = HANDSHAKE_HARD_TIMEOUT;

#[tokio::test]
async fn nitrokey_initiator_mock_responder_same_session_key() {
    let nk = software_nitrokey_token_from_seed(0x42).expect("software nk");
    // Mock peer on the same Brainpool P-256 curve as the software AUT key.
    let mock_id = MockIdentity::from_label_curve("interop-bob", SessionCurve::BrainpoolP256r1);
    let mock = MockSessionToken::new(mock_id);

    let local = [1u8; 32];
    let remote = [2u8; 32];

    let (init, pending) = nk
        .initiator_offer(SessionCurve::BrainpoolP256r1, TIMEOUT)
        .await
        .expect("offer");
    let (resp, mock_out) = mock
        .responder_reply(init, nk.long_term_cert(), &remote, &local, TIMEOUT)
        .await
        .expect("mock respond");
    let nk_out = nk
        .initiator_finish(
            pending,
            resp,
            mock.long_term_cert(),
            &local,
            &remote,
            TIMEOUT,
        )
        .await
        .expect("nk finish");

    assert_eq!(
        nk_out.saga_session_key.as_slice(),
        mock_out.saga_session_key.as_slice(),
        "session keys must match"
    );
}

#[tokio::test]
async fn mock_initiator_nitrokey_responder_same_session_key() {
    let nk = software_nitrokey_token_from_seed(0x7a).expect("software nk");
    let mock_id = MockIdentity::from_label_curve("interop-alice", SessionCurve::BrainpoolP256r1);
    let mock = MockSessionToken::new(mock_id);

    let local = [3u8; 32];
    let remote = [4u8; 32];

    let (init, pending) = mock
        .initiator_offer(SessionCurve::BrainpoolP256r1, TIMEOUT)
        .await
        .expect("mock offer");
    let (resp, nk_out) = nk
        .responder_reply(init, mock.long_term_cert(), &remote, &local, TIMEOUT)
        .await
        .expect("nk respond");
    let mock_out = mock
        .initiator_finish(pending, resp, nk.long_term_cert(), &local, &remote, TIMEOUT)
        .await
        .expect("mock finish");

    assert_eq!(
        nk_out.saga_session_key.as_slice(),
        mock_out.saga_session_key.as_slice()
    );
}

#[tokio::test]
async fn unknown_signer_rejected() {
    let nk = software_nitrokey_token_from_seed(1).expect("nk");
    let mock_id = MockIdentity::from_label_curve("other", SessionCurve::BrainpoolP256r1);
    let mock = MockSessionToken::new(mock_id);
    let wrong = MockIdentity::from_label_curve("wrong", SessionCurve::BrainpoolP256r1);
    let wrong_cert = wrong.long_term_cert();

    let (init, _pending) = nk
        .initiator_offer(SessionCurve::BrainpoolP256r1, TIMEOUT)
        .await
        .expect("offer");
    let result = mock
        .responder_reply(init, wrong_cert, &[9u8; 32], &[8u8; 32], TIMEOUT)
        .await;
    assert!(result.is_err(), "wrong signer must fail");
    let msg = result.err().unwrap().to_string();
    assert!(
        msg.contains("unknown") || msg.contains("fingerprint") || msg.contains("signer"),
        "got {msg}"
    );
}
