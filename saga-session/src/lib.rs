//! Saga authenticated ephemeral-session handshake (production crate).
//!
//! Holds [`SessionToken`], [`HandshakeOutput`], EndpointId channel binding,
//! confirmation MACs, and the Init/Response stream driver. Software mock
//! backends live in `testing/mock-token`.

mod channel_bind;
mod confirm;
mod handshake_io;
mod token;

pub use channel_bind::{bind_iroh_endpoints, transcript_hash};
pub use confirm::{
    derive_confirmation_key, initiator_mac, responder_mac, verify_mac, ConfirmationKey,
    CONFIRM_MAC_LEN,
};
pub use handshake_io::{
    perturb_endpoint_id, run_initiator_session, run_responder_session, HandshakeOutcome,
};
pub use token::{
    HandshakeOutput, InitiatorPending, SessionToken, TokenError, DEFAULT_SESSION_CURVE,
    EPHEMERAL_SESSION_GIT_DATE, EPHEMERAL_SESSION_GIT_REV, EPHEMERAL_SESSION_LICENCE,
    HANDSHAKE_HARD_TIMEOUT, HANDSHAKE_TARGET_TIMEOUT, SESSION_LOG_MARKER,
};

pub use ephemeral_session::{
    InitMessage, LongTermCert, ResponseMessage, SessionCurve, MAX_HANDSHAKE_BYTES,
};
