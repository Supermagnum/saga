//! SRTP master key/salt derivation from [`saga_session::HandshakeOutput`].

use hkdf::Hkdf;
use saga_session::HandshakeOutput;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

/// AEAD_AES_256_GCM master key length (RFC 7714).
pub const SRTP_MASTER_KEY_LEN: usize = 32;
/// AEAD_AES_256_GCM master salt length (RFC 7714).
pub const SRTP_MASTER_SALT_LEN: usize = 12;

const LABEL_CALLER_TO_CALLEE: &[u8] = b"saga-srtp-caller-to-callee-v1";
const LABEL_CALLEE_TO_CALLER: &[u8] = b"saga-srtp-callee-to-caller-v1";

/// One direction's SRTP master key + salt.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SrtpMaster {
    pub key: Zeroizing<[u8; SRTP_MASTER_KEY_LEN]>,
    pub salt: Zeroizing<[u8; SRTP_MASTER_SALT_LEN]>,
}

/// Directional pair for a call.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct SrtpKeyPair {
    pub caller_to_callee: SrtpMaster,
    pub callee_to_caller: SrtpMaster,
}

/// Whether this endpoint originated the call (caller) or answered (callee).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallRole {
    Caller,
    Callee,
}

impl SrtpKeyPair {
    /// Derive both directions from handshake output.
    ///
    /// IKM = `saga_session_key` (already EndpointId-bound). Info includes the
    /// direction label and `transcript_hash` so keys differ from confirmation
    /// (`saga-iroh-confirm-v1`) and from each other.
    pub fn derive(output: &HandshakeOutput) -> Self {
        Self {
            caller_to_callee: expand(output, LABEL_CALLER_TO_CALLEE),
            callee_to_caller: expand(output, LABEL_CALLEE_TO_CALLER),
        }
    }

    /// Outbound / inbound masters for this role.
    pub fn for_role(&self, role: CallRole) -> (&SrtpMaster, &SrtpMaster) {
        match role {
            CallRole::Caller => (&self.caller_to_callee, &self.callee_to_caller),
            CallRole::Callee => (&self.callee_to_caller, &self.caller_to_callee),
        }
    }
}

fn expand(output: &HandshakeOutput, label: &[u8]) -> SrtpMaster {
    // IKM = saga_session_key; direction label + transcript_hash in info
    // (distinct from confirmation label saga-iroh-confirm-v1).
    let hk = Hkdf::<Sha256>::new(None, output.saga_session_key.as_slice());
    let mut info = Vec::with_capacity(label.len() + output.transcript_hash.len());
    info.extend_from_slice(label);
    info.extend_from_slice(&output.transcript_hash);
    let mut out = [0u8; SRTP_MASTER_KEY_LEN + SRTP_MASTER_SALT_LEN];
    hk.expand(&info, &mut out)
        .expect("HKDF-Expand length fits SHA-256");
    let mut key = [0u8; SRTP_MASTER_KEY_LEN];
    let mut salt = [0u8; SRTP_MASTER_SALT_LEN];
    key.copy_from_slice(&out[..SRTP_MASTER_KEY_LEN]);
    salt.copy_from_slice(&out[SRTP_MASTER_KEY_LEN..]);
    out.zeroize();
    SrtpMaster {
        key: Zeroizing::new(key),
        salt: Zeroizing::new(salt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    fn dummy_output() -> HandshakeOutput {
        HandshakeOutput {
            shared_secret: Zeroizing::new(vec![1u8; 48]),
            transcript_hash: [2u8; 32],
            profile_prk: Zeroizing::new([3u8; 32]),
            saga_session_key: Zeroizing::new([4u8; 32]),
            confirmation_key: Zeroizing::new([5u8; 32]),
        }
    }

    #[test]
    fn directions_differ() {
        let pair = SrtpKeyPair::derive(&dummy_output());
        assert_ne!(
            pair.caller_to_callee.key.as_slice(),
            pair.callee_to_caller.key.as_slice()
        );
        assert_ne!(
            pair.caller_to_callee.salt.as_slice(),
            pair.callee_to_caller.salt.as_slice()
        );
    }

    #[test]
    fn role_swap_is_consistent() {
        let pair = SrtpKeyPair::derive(&dummy_output());
        let (c_out, c_in) = pair.for_role(CallRole::Caller);
        let (s_out, s_in) = pair.for_role(CallRole::Callee);
        assert_eq!(c_out.key.as_slice(), s_in.key.as_slice());
        assert_eq!(c_in.key.as_slice(), s_out.key.as_slice());
    }
}
