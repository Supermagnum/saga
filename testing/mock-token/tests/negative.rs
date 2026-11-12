//! Host negative tests: typed TokenError for each failure mode.

use ephemeral_session::{InitMessage, ResponseMessage, SessionCurve};
use mock_token::{
    MockIdentity, MockSessionToken, SessionToken, TokenError, HANDSHAKE_HARD_TIMEOUT,
};

const EA: &[u8] = b"endpoint-a";
const EB: &[u8] = b"endpoint-b";

async fn happy_init_bytes() -> (
    MockIdentity,
    MockIdentity,
    Vec<u8>,
    mock_token::InitiatorPending,
) {
    let alice = MockIdentity::from_label("neg-alice");
    let bob = MockIdentity::from_label("neg-bob");
    let token = MockSessionToken::with_ephemeral_seed(alice.clone(), 0x11);
    let (init, pending) = token
        .initiator_offer(SessionCurve::BrainpoolP384r1, HANDSHAKE_HARD_TIMEOUT)
        .await
        .unwrap();
    let bytes = init.serialise().unwrap().as_slice().to_vec();
    (alice, bob, bytes, pending)
}

fn expect_err<T>(r: Result<T, TokenError>) -> TokenError {
    match r {
        Ok(_) => panic!("expected TokenError"),
        Err(e) => e,
    }
}

#[tokio::test]
async fn tampered_signature_fails_bad_signature() {
    let (alice, bob, mut init_bytes, _pending) = happy_init_bytes().await;
    let last = init_bytes.len() - 1;
    init_bytes[last] ^= 0xff;
    let init = InitMessage::parse(&init_bytes).expect("parse may succeed with bad sig bytes");
    let resp_token = MockSessionToken::new(bob);
    let err = expect_err(
        resp_token
            .responder_reply(init, alice.long_term_cert(), EB, EA, HANDSHAKE_HARD_TIMEOUT)
            .await,
    );
    assert!(
        matches!(err, TokenError::BadSignature | TokenError::Protocol(_)),
        "got {err:?}"
    );
}

#[tokio::test]
async fn wrong_long_term_key_fails_unknown_or_bad_sig() {
    let (alice, _bob, init_bytes, pending) = happy_init_bytes().await;
    let impostor = MockIdentity::from_label("neg-impostor");
    let resp_token = MockSessionToken::with_ephemeral_seed(impostor, 0x22);
    let init = InitMessage::parse(&init_bytes).unwrap();
    let (resp, _) = resp_token
        .responder_reply(init, alice.long_term_cert(), EB, EA, HANDSHAKE_HARD_TIMEOUT)
        .await
        .expect("impostor can still respond with alice as peer cert");
    let init_token = MockSessionToken::with_ephemeral_seed(alice, 0x11);
    let err = expect_err(
        init_token
            .initiator_finish(
                pending,
                resp,
                MockIdentity::from_label("neg-bob").long_term_cert(),
                EA,
                EB,
                HANDSHAKE_HARD_TIMEOUT,
            )
            .await,
    );
    assert!(
        matches!(
            err,
            TokenError::BadSignature | TokenError::UnknownSigner | TokenError::Protocol(_)
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn wrong_curve_id_fails_curve_mismatch() {
    let alice = MockIdentity::from_label_curve("neg-alice", SessionCurve::BrainpoolP384r1);
    let bob = MockIdentity::from_label_curve("neg-bob", SessionCurve::BrainpoolP256r1);
    let init_token = MockSessionToken::new(alice.clone());
    let (init, _pending) = init_token
        .initiator_offer(SessionCurve::BrainpoolP384r1, HANDSHAKE_HARD_TIMEOUT)
        .await
        .unwrap();
    let resp_token = MockSessionToken::new(bob);
    let err = expect_err(
        resp_token
            .responder_reply(init, alice.long_term_cert(), EB, EA, HANDSHAKE_HARD_TIMEOUT)
            .await,
    );
    assert_eq!(err, TokenError::CurveMismatch);
}

#[tokio::test]
async fn truncated_message_fails() {
    let err = match InitMessage::parse(&[0x01, 0x02]) {
        Ok(_) => panic!("expected parse failure"),
        Err(e) => TokenError::from(e),
    };
    assert_eq!(err, TokenError::TruncatedMessage);
}

#[tokio::test]
async fn replayed_init_message_fails() {
    let (alice, bob, init_bytes, _) = happy_init_bytes().await;
    let resp_token = MockSessionToken::with_ephemeral_seed(bob, 0x33);
    let init1 = InitMessage::parse(&init_bytes).unwrap();
    let init2 = InitMessage::parse(&init_bytes).unwrap();
    resp_token
        .responder_reply(
            init1,
            alice.long_term_cert(),
            EB,
            EA,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .expect("first ok");
    let err = expect_err(
        resp_token
            .responder_reply(
                init2,
                alice.long_term_cert(),
                EB,
                EA,
                HANDSHAKE_HARD_TIMEOUT,
            )
            .await,
    );
    assert_eq!(err, TokenError::Replay);
}

#[tokio::test]
async fn truncated_response_parse_fails() {
    let err = match ResponseMessage::parse(&[0x01]) {
        Ok(_) => panic!("expected parse failure"),
        Err(e) => TokenError::from(e),
    };
    assert_eq!(err, TokenError::TruncatedMessage);
}
