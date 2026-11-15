//! Production USB / OpenPGP card SessionToken backends for Saga.
//!
//! # Nitrokey design
//!
//! Galdralag on-token ephemeral ECDH is blocked (xous-core#937). Nitrokey is the
//! first **production** [`saga_session::SessionToken`]:
//!
//! 1. Phone generates the ephemeral Brainpool keypair ([`saga_session::OsTrng`]).
//! 2. Phone hashes the Galdralag handshake preimage with SHA-256.
//! 3. Card signs that digest with the **AUT** key via INTERNAL AUTHENTICATE.
//! 4. Phone performs ECDH and derives the same [`saga_session::HandshakeOutput`]
//!    path as the mock / future Galdralag backends.
//!
//! This is **weaker than Galdralag**: the ephemeral private key lives in phone
//! RAM until ECDH completes. Per-call forward secrecy still holds after zeroise.
//!
//! # Fingerprint
//!
//! Saga contacts store the Galdralag wire fingerprint: **SHA-256 of the AUT
//! public key SEC1 bytes**, hex-encoded as 64 lowercase ASCII. This is **not**
//! the OpenPGP v4 fingerprint printed by GnuPG.
//!
//! # Hash / signature encoding
//!
//! - Preimage: same as Galdralag `ephemeral-session` (`version || curve_id || …`).
//! - Digest: SHA-256(preimage).
//! - Card: ECDSA over that digest (OpenPGP ECC INTERNAL AUTHENTICATE); wire
//!   signature is **DER** ECDSA (same as Galdralag verify).
//!
//! # NFC
//!
//! OpenPGP-over-NFC is **not** implemented. Nitrokey 3 NFC is reported as
//! FIDO2-only on Android for OpenPGP; Saga speaks USB CCID Host only.
//!
//! # Supported cards
//!
//! AUT algorithm attributes are read from the card (never assumed). Supported
//! only when AUT is ECDSA on Brainpool P-256r1 or P-384r1. See
//! `docs/nitrokey.md`.

#![deny(unsafe_code)]

/// Literal retained in release `.so` so hygiene scripts can confirm the production
/// token backend is linked (must never appear under `testing/`).
pub const NITROKEY_BACKEND_MARKER: &str = "saga-nitrokey-session-token-v1";

mod algo;
mod error;
mod nitrokey;
mod pin;
mod transport;

#[cfg(feature = "test-card")]
pub mod test_card;

#[cfg(feature = "pcsc")]
pub mod pcsc;

pub use algo::{brainpool_oid, curve_from_algorithm_attributes, BrainpoolAutCurve};
pub use error::UsbTokenError;
pub use nitrokey::{AutSigner, NitrokeySessionToken, OpenPgpAutSigner, PinBytes};
pub use pin::{ChannelPinProvider, PinProvider, StaticPinProvider};
pub use transport::{
    supported_usb_devices, ApduCardBackend, ApduCardFactory, ApduTransport, CallbackTransport,
    CardFactory, OpenPgpAutCard, SharedCardFactory, UsbDeviceIds, GALDRALAG_DABAO, NITROKEY_3,
    NITROKEY_PRO2,
};
