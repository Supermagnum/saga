//! Token abstraction for authenticated ephemeral-session handshakes.
//!
//! The mock implements this now. Future backends (Galdralag USB CCID, Nitrokey
//! USB CCID) must not assume an in-process vault: every call is fallible,
//! async-safe, and accepts a timeout because a real token can be slow,
//! unplugged, or wait for a PIN.

use std::time::Duration;

use async_trait::async_trait;
use ephemeral_session::{InitMessage, LongTermCert, ResponseMessage, SessionCurve};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// Log marker for production session handshake lines (debug builds).
pub const SESSION_LOG_MARKER: &str = "SAGA_SESSION";

/// Galdralag-firmware pin used by this crate (git rev, never a branch).
pub const EPHEMERAL_SESSION_GIT_REV: &str = "b781bf6fa5ccf8700a5fb061583c31cfa00241a1";
/// Commit date of [`EPHEMERAL_SESSION_GIT_REV`] (UTC).
pub const EPHEMERAL_SESSION_GIT_DATE: &str = "2026-10-22";
/// Upstream crate licence (GPL-3.0); compatible with Saga AGPL-3.0.
pub const EPHEMERAL_SESSION_LICENCE: &str = "GPL-3.0";

/// Default curve: firmware / CESS preference is Brainpool P-384r1.
pub const DEFAULT_SESSION_CURVE: SessionCurve = SessionCurve::BrainpoolP384r1;

/// Spec §8 hard timeout for token handshake steps (generate / sign / verify / ECDH).
pub const HANDSHAKE_HARD_TIMEOUT: Duration = Duration::from_secs(4);
/// Spec §8 target latency budget.
pub const HANDSHAKE_TARGET_TIMEOUT: Duration = Duration::from_secs(2);

/// Typed failures from a [`SessionToken`] backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenError {
    /// Step exceeded the caller-supplied timeout (USB stall, PIN wait, etc.).
    Timeout,
    /// Token reported it is not present / unplugged.
    Unavailable,
    /// PIN required or rejected (reserved for real USB backends).
    PinRequired,
    /// Peer signature did not verify under the expected long-term key.
    BadSignature,
    /// Signer fingerprint is not in the local trust material.
    UnknownSigner,
    /// Curve id in the peer message does not match the negotiated curve.
    CurveMismatch,
    /// Handshake bytes were truncated or otherwise malformed.
    TruncatedMessage,
    /// The same InitMessage serialisation was seen again (replay).
    Replay,
    /// Saga confirmation MAC mismatch (channel-binding / EndpointId disagreement).
    ConfirmationFailed,
    /// Underlying ephemeral-session / vault failure.
    Protocol(String),
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenError::Timeout => write!(f, "token handshake timed out"),
            TokenError::Unavailable => write!(f, "token unavailable"),
            TokenError::PinRequired => write!(f, "token PIN required"),
            TokenError::BadSignature => write!(f, "bad handshake signature"),
            TokenError::UnknownSigner => write!(f, "unknown handshake signer"),
            TokenError::CurveMismatch => write!(f, "handshake curve mismatch"),
            TokenError::TruncatedMessage => write!(f, "truncated handshake message"),
            TokenError::Replay => write!(f, "replayed InitMessage"),
            TokenError::ConfirmationFailed => write!(f, "confirmation MAC failed"),
            TokenError::Protocol(s) => write!(f, "protocol error: {s}"),
        }
    }
}

impl std::error::Error for TokenError {}

impl From<ephemeral_session::EphemeralSessionError> for TokenError {
    fn from(e: ephemeral_session::EphemeralSessionError) -> Self {
        use ephemeral_session::EphemeralSessionError as E;
        match e {
            E::InvalidPeerSignature => TokenError::BadSignature,
            E::FingerprintMismatch => TokenError::UnknownSigner,
            E::CurveMismatch => TokenError::CurveMismatch,
            E::MalformedHandshake | E::RemovedBrainpoolP512Curve => TokenError::TruncatedMessage,
            other => TokenError::Protocol(other.to_string()),
        }
    }
}

/// Handshake output consumed by Phase 2 (SRTP key derivation).
///
/// Secrets implement [`ZeroizeOnDrop`]. Do not log secret bytes;
/// `#[cfg(debug_assertions)]` builds may log a short hash prefix of
/// [`Self::saga_session_key`] only.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct HandshakeOutput {
    /// Classical ECDH shared secret (packed x-coordinate) from ephemeral-session.
    pub shared_secret: Zeroizing<Vec<u8>>,
    /// Transcript hash: SHA-256 of ordered (min||max) ephemeral public keys
    /// (same salt material ephemeral-session feeds to HKDF-Extract).
    pub transcript_hash: [u8; 32],
    /// HKDF-Extract PRK from ephemeral-session (`SessionKeys::profile_prk`).
    pub profile_prk: Zeroizing<[u8; 32]>,
    /// Saga media/session key after Iroh EndpointId channel binding.
    pub saga_session_key: Zeroizing<[u8; 32]>,
    /// Dedicated confirmation HMAC key (not the media key); zeroised on drop.
    pub confirmation_key: Zeroizing<[u8; 32]>,
}

impl HandshakeOutput {
    /// First two bytes of SHA-256(session key) for debug log correlation only.
    pub fn session_key_debug_prefix(&self) -> (u8, u8) {
        use sha2::{Digest, Sha256};
        let dig = Sha256::digest(self.saga_session_key.as_slice());
        (dig[0], dig[1])
    }
}

/// Opaque initiator state between offer generation and response completion.
///
/// Backends that keep ephemeral private keys on a USB device may store only a
/// handle here; the software mock keeps an in-process
/// [`ephemeral_session::InitiatorSession`].
pub struct InitiatorPending {
    pub(crate) inner: ephemeral_session::InitiatorSession,
    #[allow(dead_code)] // retained for USB backends that re-check curve on finish
    pub(crate) curve: SessionCurve,
    pub(crate) init_ephemeral_public_key: Vec<u8>,
}

impl InitiatorPending {
    /// Construct pending state from an in-progress [`ephemeral_session::InitiatorSession`].
    pub fn from_session(
        inner: ephemeral_session::InitiatorSession,
        curve: SessionCurve,
        init_ephemeral_public_key: Vec<u8>,
    ) -> Self {
        Self {
            inner,
            curve,
            init_ephemeral_public_key,
        }
    }

    pub fn init_ephemeral_public_key(&self) -> &[u8] {
        &self.init_ephemeral_public_key
    }

    pub fn take_inner(self) -> ephemeral_session::InitiatorSession {
        self.inner
    }
}

/// Produce / verify signed ephemeral offers and derive the shared secret.
///
/// No method may assume an in-process mock. USB backends will block on CCID
/// and must honour `timeout`.
#[async_trait]
pub trait SessionToken: Send + Sync {
    /// Long-term verifying certificate for this token (SEC1 + fingerprint).
    fn long_term_cert(&self) -> LongTermCert;

    /// Wire fingerprint: 64 lowercase hex ASCII bytes (ephemeral-session format).
    fn fingerprint_hex(&self) -> [u8; 64];

    /// Generate a signed [`InitMessage`] (ephemeral sk retained in the pending handle).
    async fn initiator_offer(
        &self,
        curve: SessionCurve,
        timeout: Duration,
    ) -> Result<(InitMessage, InitiatorPending), TokenError>;

    /// Verify the peer [`ResponseMessage`] and derive [`HandshakeOutput`].
    async fn initiator_finish(
        &self,
        pending: InitiatorPending,
        response: ResponseMessage,
        peer_cert: LongTermCert,
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
        timeout: Duration,
    ) -> Result<HandshakeOutput, TokenError>;

    /// Verify [`InitMessage`], produce [`ResponseMessage`], derive [`HandshakeOutput`].
    async fn responder_reply(
        &self,
        init: InitMessage,
        peer_cert: LongTermCert,
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
        timeout: Duration,
    ) -> Result<(ResponseMessage, HandshakeOutput), TokenError>;
}
