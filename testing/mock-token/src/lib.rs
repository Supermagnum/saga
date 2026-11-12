//! Software mock Galdralag token for Saga E2E.
//!
//! Production handshake types live in [`saga_session`]. This crate keeps
//! [`MockSessionToken`], [`MockIdentity`], and the synthetic media probe
//! (until Phase 2 real media lands).

mod handshake_io;
mod identity;
mod media;
mod mock_session;

/// Log marker for mock-token handshake lines (E2E / debug).
pub const MOCK_TOKEN_MARKER: &str = "MOCK_TOKEN";

pub use handshake_io::{
    perturb_endpoint_id, run_initiator, run_initiator_with_options, run_responder,
    run_responder_with_options, HandshakeOutcome,
};
pub use identity::MockIdentity;
pub use media::{decrypt_frame, encrypt_frame, SYNTHETIC_PLAINTEXT};
pub use mock_session::MockSessionToken;

// Re-export production API so existing `use mock_token::{SessionToken, ...}` keeps working.
pub use saga_session::{
    bind_iroh_endpoints, derive_confirmation_key, initiator_mac, responder_mac, transcript_hash,
    verify_mac, ConfirmationKey, HandshakeOutput, InitiatorPending, SessionToken, TokenError,
    CONFIRM_MAC_LEN, DEFAULT_SESSION_CURVE, EPHEMERAL_SESSION_GIT_DATE, EPHEMERAL_SESSION_GIT_REV,
    EPHEMERAL_SESSION_LICENCE, HANDSHAKE_HARD_TIMEOUT, HANDSHAKE_TARGET_TIMEOUT,
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
