//! Saga authenticated ephemeral-session handshake (production crate).
//!
//! Holds [`SessionToken`], [`HandshakeOutput`], EndpointId channel binding,
//! confirmation MACs, and the Init/Response stream driver. Software mock
//! backends (`FakeTrng` / `FakeVaultStorage`) live only in `testing/mock-token`.

mod channel_bind;
mod confirm;
mod hal_os;
mod handshake_io;
mod phone_handshake;
mod roster;
mod token;

pub use channel_bind::{bind_iroh_endpoints, transcript_hash};
pub use confirm::{
    derive_confirmation_key, initiator_mac, responder_mac, verify_mac, ConfirmationKey,
    CONFIRM_MAC_LEN,
};
pub use hal_os::{MemoryVaultStorage, OsTrng};
pub use handshake_io::{
    perturb_endpoint_id, run_initiator_session, run_responder_session, HandshakeOutcome,
};
pub use phone_handshake::{
    build_init_message, build_response_message, fingerprint_hex_from_sec1, handshake_digest,
    init_sign_preimage, initiator_finish_phone, long_term_cert_from_sec1, responder_ecdh_output,
    response_sign_preimage, verify_handshake_signature, verify_init_message, PhoneEphemeral,
};
pub use roster::{
    read_roster, write_roster, Roster, RosterEntry, RosterError, ROSTER_MAX_PARTICIPANTS,
    ROSTER_TAG_LEN, ROSTER_VERSION,
};
pub use token::{
    HandshakeOutput, InitiatorPending, SessionToken, TokenError, DEFAULT_SESSION_CURVE,
    EPHEMERAL_SESSION_GIT_DATE, EPHEMERAL_SESSION_GIT_REV, EPHEMERAL_SESSION_LICENCE,
    HANDSHAKE_HARD_TIMEOUT, HANDSHAKE_TARGET_TIMEOUT, SESSION_LOG_MARKER,
};

pub use ephemeral_session::{
    InitMessage, LongTermCert, ResponseMessage, SessionCurve, INIT_PROTOCOL_VERSION,
    MAX_HANDSHAKE_BYTES, RESP_PROTOCOL_VERSION,
};
