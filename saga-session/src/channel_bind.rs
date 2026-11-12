//! Saga-side channel binding of Iroh EndpointIds into the session key.
//!
//! ephemeral-session's HKDF salt binds only the ordered ephemeral public keys.
//! It does **not** include Iroh EndpointIds. Without a further binding, a
//! confused-deputy / endpoint-swap scenario (same long-term keys presented on
//! a different Iroh path) could reuse handshake-derived key material on the
//! wrong QUIC identities.
//!
//! Saga derives `saga_session_key` as HKDF-Expand over `profile_prk` with info
//! `saga-iroh-channel-v1 || min(eid_a, eid_b) || max(eid_a, eid_b)`. This does
//! not change the InitMessage / ResponseMessage wire format.

use hkdf::Hkdf;
use sha2::Sha256;
use zeroize::Zeroizing;

const INFO_PREFIX: &[u8] = b"saga-iroh-channel-v1";

/// Derive the Saga session key from ephemeral-session `profile_prk` + EndpointIds.
pub fn bind_iroh_endpoints(
    profile_prk: &[u8; 32],
    endpoint_a: &[u8],
    endpoint_b: &[u8],
) -> Zeroizing<[u8; 32]> {
    let (lo, hi) = if endpoint_a <= endpoint_b {
        (endpoint_a, endpoint_b)
    } else {
        (endpoint_b, endpoint_a)
    };
    let mut info = Vec::with_capacity(INFO_PREFIX.len() + lo.len() + hi.len());
    info.extend_from_slice(INFO_PREFIX);
    info.extend_from_slice(lo);
    info.extend_from_slice(hi);
    let hk = Hkdf::<Sha256>::from_prk(profile_prk).expect("32-byte prk");
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("hkdf expand 32 bytes");
    out
}

/// Transcript hash matching ephemeral-session salt: SHA-256(min(epk)||max(epk)).
pub fn transcript_hash(epk_initiator: &[u8], epk_responder: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    if epk_initiator <= epk_responder {
        hasher.update(epk_initiator);
        hasher.update(epk_responder);
    } else {
        hasher.update(epk_responder);
        hasher.update(epk_initiator);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_is_order_independent() {
        let prk = [7u8; 32];
        let a = b"endpoint-aaa";
        let b = b"endpoint-bbb";
        let k1 = bind_iroh_endpoints(&prk, a, b);
        let k2 = bind_iroh_endpoints(&prk, b, a);
        assert_eq!(k1.as_slice(), k2.as_slice());
    }

    #[test]
    fn different_endpoints_differ() {
        let prk = [7u8; 32];
        let k1 = bind_iroh_endpoints(&prk, b"a", b"b");
        let k2 = bind_iroh_endpoints(&prk, b"a", b"c");
        assert_ne!(k1.as_slice(), k2.as_slice());
    }
}
