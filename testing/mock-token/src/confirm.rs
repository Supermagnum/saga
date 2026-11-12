//! Saga-side mutual confirmation MAC after ephemeral-session key derivation.
//!
//! Channel binding alone changes derived keys when EndpointIds disagree, but
//! without an explicit check both sides would still report "Encrypted" until
//! media fails. Spec §7.4 / §12: after deriving keys, each peer sends an HMAC
//! over the Iroh stream keyed from a dedicated HKDF output (not the media key),
//! with distinct initiator/responder role labels. Constant-time compare
//! (`subtle`). Security state may only resolve to Encrypted after both MACs
//! verify (inside the §8 2 s / 4 s budget).

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::token::TokenError;

/// Wire size of one confirmation MAC (HMAC-SHA256).
pub const CONFIRM_MAC_LEN: usize = 32;

const CONFIRM_KEY_INFO: &[u8] = b"saga-iroh-confirm-v1";
const ROLE_INITIATOR: &[u8] = b"saga-confirm-initiator-v1";
const ROLE_RESPONDER: &[u8] = b"saga-confirm-responder-v1";

/// Dedicated confirmation key (ZeroizeOnDrop). Not the media/session key.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct ConfirmationKey(Zeroizing<[u8; 32]>);

impl ConfirmationKey {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Rebuild from stored key bytes (e.g. after HKDF in [`HandshakeOutput`]).
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        ConfirmationKey(Zeroizing::new(bytes))
    }
}

fn ordered_eids<'a>(a: &'a [u8], b: &'a [u8]) -> (&'a [u8], &'a [u8]) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Derive the confirmation key from `profile_prk` + sorted EndpointIds.
pub fn derive_confirmation_key(
    profile_prk: &[u8; 32],
    endpoint_a: &[u8],
    endpoint_b: &[u8],
) -> ConfirmationKey {
    let (lo, hi) = ordered_eids(endpoint_a, endpoint_b);
    let mut info = Vec::with_capacity(CONFIRM_KEY_INFO.len() + lo.len() + hi.len());
    info.extend_from_slice(CONFIRM_KEY_INFO);
    info.extend_from_slice(lo);
    info.extend_from_slice(hi);
    let hk = Hkdf::<Sha256>::from_prk(profile_prk).expect("32-byte prk");
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("hkdf expand 32 bytes");
    ConfirmationKey(out)
}

fn mac_for_role(key: &ConfirmationKey, role: &[u8], transcript_hash: &[u8; 32]) -> [u8; 32] {
    type HmacSha256 = Hmac<Sha256>;
    let mut mac = HmacSha256::new_from_slice(key.as_bytes()).expect("hmac key length");
    mac.update(role);
    mac.update(transcript_hash);
    let result = mac.finalize().into_bytes();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

/// Initiator (caller) confirmation MAC.
pub fn initiator_mac(key: &ConfirmationKey, transcript_hash: &[u8; 32]) -> [u8; CONFIRM_MAC_LEN] {
    mac_for_role(key, ROLE_INITIATOR, transcript_hash)
}

/// Responder (callee) confirmation MAC.
pub fn responder_mac(key: &ConfirmationKey, transcript_hash: &[u8; 32]) -> [u8; CONFIRM_MAC_LEN] {
    mac_for_role(key, ROLE_RESPONDER, transcript_hash)
}

/// Constant-time MAC equality. Returns [`TokenError::ConfirmationFailed`] on mismatch.
pub fn verify_mac(expected: &[u8; CONFIRM_MAC_LEN], received: &[u8]) -> Result<(), TokenError> {
    if received.len() != CONFIRM_MAC_LEN {
        return Err(TokenError::ConfirmationFailed);
    }
    let mut got = [0u8; CONFIRM_MAC_LEN];
    got.copy_from_slice(received);
    if bool::from(expected.ct_eq(&got)) {
        Ok(())
    } else {
        Err(TokenError::ConfirmationFailed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_labels_differ() {
        let key = derive_confirmation_key(&[9u8; 32], b"a", b"b");
        let th = [1u8; 32];
        assert_ne!(initiator_mac(&key, &th), responder_mac(&key, &th));
    }

    #[test]
    fn endpoint_order_independent() {
        let prk = [3u8; 32];
        let k1 = derive_confirmation_key(&prk, b"aa", b"bb");
        let k2 = derive_confirmation_key(&prk, b"bb", b"aa");
        assert_eq!(k1.as_bytes(), k2.as_bytes());
    }
}
