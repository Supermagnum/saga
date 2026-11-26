//! Directional messaging keys from [`HandshakeOutput`].

use hkdf::Hkdf;
use saga_session::HandshakeOutput;
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

pub const MSG_KEY_LEN: usize = 32;

const LABEL_INIT_TO_RESP: &[u8] = b"saga-msg-initiator-to-responder-v1";
const LABEL_RESP_TO_INIT: &[u8] = b"saga-msg-responder-to-initiator-v1";

/// Whether this endpoint opened the messaging session (initiator) or accepted it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgRole {
    Initiator,
    Responder,
}

/// One direction's AES-256-GCM key.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MsgKey(Zeroizing<[u8; MSG_KEY_LEN]>);

impl MsgKey {
    pub fn as_bytes(&self) -> &[u8; MSG_KEY_LEN] {
        &self.0
    }
}

/// Directional pair for a messaging session.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct MsgKeyPair {
    pub initiator_to_responder: MsgKey,
    pub responder_to_initiator: MsgKey,
}

impl MsgKeyPair {
    /// Derive both directions from handshake output.
    ///
    /// IKM = `saga_session_key` (EndpointId-bound). Info = direction label ||
    /// `transcript_hash`, distinct from SRTP and confirmation labels.
    pub fn derive(output: &HandshakeOutput) -> Self {
        Self {
            initiator_to_responder: expand(output, LABEL_INIT_TO_RESP),
            responder_to_initiator: expand(output, LABEL_RESP_TO_INIT),
        }
    }

    /// Outbound / inbound keys for this role.
    pub fn for_role(&self, role: MsgRole) -> (&MsgKey, &MsgKey) {
        match role {
            MsgRole::Initiator => (&self.initiator_to_responder, &self.responder_to_initiator),
            MsgRole::Responder => (&self.responder_to_initiator, &self.initiator_to_responder),
        }
    }
}

/// Convenience: derive and return the pair.
pub fn derive_msg_keys(output: &HandshakeOutput) -> MsgKeyPair {
    MsgKeyPair::derive(output)
}

fn expand(output: &HandshakeOutput, label: &[u8]) -> MsgKey {
    let hk = Hkdf::<Sha256>::new(None, output.saga_session_key.as_slice());
    let mut info = Vec::with_capacity(label.len() + output.transcript_hash.len());
    info.extend_from_slice(label);
    info.extend_from_slice(&output.transcript_hash);
    let mut out = Zeroizing::new([0u8; MSG_KEY_LEN]);
    hk.expand(&info, out.as_mut())
        .expect("HKDF-Expand 32 bytes");
    MsgKey(out)
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
        let pair = MsgKeyPair::derive(&dummy_output());
        assert_ne!(
            pair.initiator_to_responder.as_bytes(),
            pair.responder_to_initiator.as_bytes()
        );
    }

    #[test]
    fn role_swap_is_consistent() {
        let pair = MsgKeyPair::derive(&dummy_output());
        let (i_out, i_in) = pair.for_role(MsgRole::Initiator);
        let (r_out, r_in) = pair.for_role(MsgRole::Responder);
        assert_eq!(i_out.as_bytes(), r_in.as_bytes());
        assert_eq!(i_in.as_bytes(), r_out.as_bytes());
    }

    #[test]
    fn differs_from_session_key() {
        let out = dummy_output();
        let pair = MsgKeyPair::derive(&out);
        assert_ne!(
            pair.initiator_to_responder.as_bytes().as_slice(),
            out.saga_session_key.as_slice()
        );
    }
}
