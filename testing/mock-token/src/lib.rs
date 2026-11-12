//! Mock Galdralag token: real `ephemeral-session` wire types over software Brainpool keys.
//!
//! Pinned firmware rev: see [`token::EPHEMERAL_SESSION_GIT_REV`].

mod channel_bind;
mod confirm;
mod handshake_io;
mod identity;
mod media;
mod mock_session;
mod token;

pub use channel_bind::{bind_iroh_endpoints, transcript_hash};
pub use confirm::{
    derive_confirmation_key, initiator_mac, responder_mac, verify_mac, ConfirmationKey,
    CONFIRM_MAC_LEN,
};
pub use handshake_io::{
    perturb_endpoint_id, run_initiator, run_initiator_with_options, run_responder,
    run_responder_with_options, HandshakeOutcome,
};
pub use identity::MockIdentity;
pub use media::{decrypt_frame, encrypt_frame, SYNTHETIC_PLAINTEXT};
pub use mock_session::MockSessionToken;
pub use token::{
    HandshakeOutput, InitiatorPending, SessionToken, TokenError, DEFAULT_SESSION_CURVE,
    EPHEMERAL_SESSION_GIT_DATE, EPHEMERAL_SESSION_GIT_REV, EPHEMERAL_SESSION_LICENCE,
    HANDSHAKE_HARD_TIMEOUT, HANDSHAKE_TARGET_TIMEOUT, MOCK_TOKEN_MARKER,
};

pub use ephemeral_session::{
    InitMessage, LongTermCert, ResponseMessage, SessionCurve, MAX_HANDSHAKE_BYTES,
};

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn handshake_round_trip_p384_default() {
        let alice = MockIdentity::from_label("15550100011");
        let bob = MockIdentity::from_label("15550100010");
        let local_a = b"endpoint-alice-aaaaaaaa";
        let local_b = b"endpoint-bob-bbbbbbbbbb";

        let (io_a, io_b) = duplex(4096);
        let (mut a_read, mut a_write) = tokio::io::split(io_a);
        let (mut b_read, mut b_write) = tokio::io::split(io_b);

        let (a_out, b_out) = tokio::join!(
            run_initiator(
                &alice,
                "15550100010",
                local_a,
                local_b,
                false,
                &mut a_read,
                &mut a_write
            ),
            run_responder(&bob, local_b, local_a, false, &mut b_read, &mut b_write),
        );

        match (&a_out, &b_out) {
            (
                HandshakeOutcome::Encrypted { output: oa },
                HandshakeOutcome::Encrypted { output: ob },
            ) => {
                assert_eq!(
                    oa.saga_session_key.as_slice(),
                    ob.saga_session_key.as_slice()
                );
                assert_eq!(oa.transcript_hash, ob.transcript_hash);
                assert_eq!(oa.shared_secret.as_slice(), ob.shared_secret.as_slice());
            }
            _ => panic!("handshake failed: {a_out:?} {b_out:?}"),
        }
    }

    #[tokio::test]
    async fn handshake_round_trip_p256() {
        let curve = SessionCurve::BrainpoolP256r1;
        let alice = MockIdentity::from_label_curve("15550100011", curve);
        let bob = MockIdentity::from_label_curve("15550100010", curve);
        let local_a = b"endpoint-alice-aaaaaaaa";
        let local_b = b"endpoint-bob-bbbbbbbbbb";

        let (io_a, io_b) = duplex(4096);
        let (mut a_read, mut a_write) = tokio::io::split(io_a);
        let (mut b_read, mut b_write) = tokio::io::split(io_b);

        let (a_out, b_out) = tokio::join!(
            run_initiator(
                &alice,
                "15550100010",
                local_a,
                local_b,
                false,
                &mut a_read,
                &mut a_write
            ),
            run_responder(&bob, local_b, local_a, false, &mut b_read, &mut b_write),
        );

        match (&a_out, &b_out) {
            (
                HandshakeOutcome::Encrypted { output: oa },
                HandshakeOutcome::Encrypted { output: ob },
            ) => assert_eq!(
                oa.saga_session_key.as_slice(),
                ob.saga_session_key.as_slice()
            ),
            _ => panic!("p256 handshake failed: {a_out:?} {b_out:?}"),
        }
    }
}
