//! One-shot generator for tests/fixtures/p384_vector_c1_d1.json
use ephemeral_session::{InitMessage, ResponseMessage, SessionCurve};
use mock_token::{MockIdentity, MockSessionToken, SessionToken, HANDSHAKE_HARD_TIMEOUT};

#[tokio::main]
async fn main() {
    let endpoint_a = b"saga-test-endpoint-aaaaaaaa";
    let endpoint_b = b"saga-test-endpoint-bbbbbbbb";
    let alice = MockIdentity::from_label("vector-alice");
    let bob = MockIdentity::from_label("vector-bob");
    let init_token = MockSessionToken::with_ephemeral_seed(alice.clone(), 0xC1);
    let resp_token = MockSessionToken::with_ephemeral_seed(bob.clone(), 0xD1);
    let (init, pending) = init_token
        .initiator_offer(SessionCurve::BrainpoolP384r1, HANDSHAKE_HARD_TIMEOUT)
        .await
        .unwrap();
    let init_bytes = init.serialise().unwrap().as_slice().to_vec();
    let (resp, _out_r) = resp_token
        .responder_reply(
            InitMessage::parse(&init_bytes).unwrap(),
            alice.long_term_cert(),
            endpoint_b,
            endpoint_a,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .unwrap();
    let resp_bytes = resp.serialise().unwrap().as_slice().to_vec();
    let out_i = init_token
        .initiator_finish(
            pending,
            ResponseMessage::parse(&resp_bytes).unwrap(),
            bob.long_term_cert(),
            endpoint_a,
            endpoint_b,
            HANDSHAKE_HARD_TIMEOUT,
        )
        .await
        .unwrap();
    println!(
        "{{\n  \"description\": \"MockIdentity vector-alice/vector-bob P384, FakeTrng 0xC1/0xD1, pin b781bf6f\",\n  \"init_message_hex\": \"{}\",\n  \"response_message_hex\": \"{}\",\n  \"shared_secret_hex\": \"{}\",\n  \"transcript_hash_hex\": \"{}\",\n  \"saga_session_key_hex\": \"{}\"\n}}",
        hex::encode(&init_bytes),
        hex::encode(&resp_bytes),
        hex::encode(out_i.shared_secret.as_slice()),
        hex::encode(out_i.transcript_hash),
        hex::encode(out_i.saga_session_key.as_slice()),
    );
}
