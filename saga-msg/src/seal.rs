//! AES-256-GCM seal / open for padded messaging payloads.

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::RngCore;
use thiserror::Error;
use zeroize::Zeroize;

use crate::keys::MsgKey;
use crate::pad::{pad_plaintext, unpad_plaintext};

pub const MSG_ID_LEN: usize = 16;
pub const WIRE_VERSION: u8 = 1;

/// Frame types on the messaging bi-stream (after handshake).
pub const FRAME_DATA: u8 = 1;
pub const FRAME_DELIVERED: u8 = 2;

const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum MsgError {
    #[error("plaintext too large")]
    TooLarge,
    #[error("bad padding")]
    BadPadding,
    #[error("decrypt failed")]
    DecryptFailed,
    #[error("malformed sealed message")]
    Malformed,
    #[error("unsupported wire version")]
    BadVersion,
}

/// Sealed wire body (no length prefix — callers frame with u32 BE length).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SealedMessage {
    pub version: u8,
    pub frame_type: u8,
    pub seq: u64,
    pub msg_id: [u8; MSG_ID_LEN],
    /// AES-GCM ciphertext including tag; nonce is derived from seq (not on wire as raw key).
    pub ciphertext: Vec<u8>,
}

impl SealedMessage {
    /// Encode to bytes: version | type | seq(8) | msg_id(16) | ciphertext.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(1 + 1 + 8 + MSG_ID_LEN + self.ciphertext.len());
        out.push(self.version);
        out.push(self.frame_type);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.msg_id);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MsgError> {
        if bytes.len() < 1 + 1 + 8 + MSG_ID_LEN + TAG_LEN {
            return Err(MsgError::Malformed);
        }
        let version = bytes[0];
        if version != WIRE_VERSION {
            return Err(MsgError::BadVersion);
        }
        let frame_type = bytes[1];
        let mut seq_buf = [0u8; 8];
        seq_buf.copy_from_slice(&bytes[2..10]);
        let seq = u64::from_be_bytes(seq_buf);
        let mut msg_id = [0u8; MSG_ID_LEN];
        msg_id.copy_from_slice(&bytes[10..10 + MSG_ID_LEN]);
        let ciphertext = bytes[10 + MSG_ID_LEN..].to_vec();
        Ok(Self {
            version,
            frame_type,
            seq,
            msg_id,
            ciphertext,
        })
    }
}

fn nonce_for_seq(seq: u64) -> [u8; NONCE_LEN] {
    // 4 zero bytes || seq BE — unique per direction under monotonic seq.
    let mut nonce = [0u8; NONCE_LEN];
    nonce[4..12].copy_from_slice(&seq.to_be_bytes());
    nonce
}

/// Seal UTF-8 text (or delivery-ack payload) under `key`.
pub fn seal_message(
    key: &MsgKey,
    seq: u64,
    frame_type: u8,
    plaintext: &[u8],
) -> Result<SealedMessage, MsgError> {
    let padded = pad_plaintext(plaintext)?;
    let mut msg_id = [0u8; MSG_ID_LEN];
    rand::thread_rng().fill_bytes(&mut msg_id);

    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).expect("32-byte key");
    let nonce = Nonce::from(nonce_for_seq(seq));
    // AAD binds version, type, seq, msg_id so header tampering fails decrypt.
    let mut aad = Vec::with_capacity(1 + 1 + 8 + MSG_ID_LEN);
    aad.push(WIRE_VERSION);
    aad.push(frame_type);
    aad.extend_from_slice(&seq.to_be_bytes());
    aad.extend_from_slice(&msg_id);

    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &padded,
                aad: &aad,
            },
        )
        .map_err(|_| MsgError::DecryptFailed)?;

    let mut padded_z = padded;
    padded_z.zeroize();

    Ok(SealedMessage {
        version: WIRE_VERSION,
        frame_type,
        seq,
        msg_id,
        ciphertext,
    })
}

/// Open a sealed message; returns unpadded plaintext.
pub fn open_message(key: &MsgKey, sealed: &SealedMessage) -> Result<Vec<u8>, MsgError> {
    if sealed.version != WIRE_VERSION {
        return Err(MsgError::BadVersion);
    }
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).expect("32-byte key");
    let nonce = Nonce::from(nonce_for_seq(sealed.seq));
    let mut aad = Vec::with_capacity(1 + 1 + 8 + MSG_ID_LEN);
    aad.push(sealed.version);
    aad.push(sealed.frame_type);
    aad.extend_from_slice(&sealed.seq.to_be_bytes());
    aad.extend_from_slice(&sealed.msg_id);

    let mut padded = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &sealed.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| MsgError::DecryptFailed)?;
    let plain = unpad_plaintext(&padded)?;
    padded.zeroize();
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{MsgKeyPair, MsgRole};
    use saga_session::HandshakeOutput;
    use zeroize::Zeroizing;

    fn output(session: u8) -> HandshakeOutput {
        HandshakeOutput {
            shared_secret: Zeroizing::new(vec![1u8; 48]),
            transcript_hash: [2u8; 32],
            profile_prk: Zeroizing::new([3u8; 32]),
            saga_session_key: Zeroizing::new([session; 32]),
            confirmation_key: Zeroizing::new([5u8; 32]),
        }
    }

    #[test]
    fn round_trip() {
        let pair = MsgKeyPair::derive(&output(6));
        let (send_key, _) = pair.for_role(MsgRole::Initiator);
        let text = b"hello saga messaging";
        let sealed = seal_message(send_key, 1, FRAME_DATA, text).unwrap();
        let opened = open_message(send_key, &sealed).unwrap();
        assert_eq!(opened, text);
        let encoded = sealed.encode();
        let decoded = SealedMessage::decode(&encoded).unwrap();
        assert_eq!(open_message(send_key, &decoded).unwrap(), text);
        let pad_len = crate::pad::pad_plaintext(text).unwrap().len();
        assert_eq!(sealed.ciphertext.len(), pad_len + TAG_LEN);
        assert!(crate::PAD_BUCKETS.contains(&pad_len));
    }

    #[test]
    fn tamper_rejected() {
        let pair = MsgKeyPair::derive(&output(4));
        let (send_key, _) = pair.for_role(MsgRole::Initiator);
        let mut sealed = seal_message(send_key, 3, FRAME_DATA, b"secret").unwrap();
        sealed.ciphertext[0] ^= 0xff;
        assert_eq!(
            open_message(send_key, &sealed),
            Err(MsgError::DecryptFailed)
        );
    }

    #[test]
    fn wrong_key_rejected() {
        let ka = MsgKeyPair::derive(&output(4));
        let kb = MsgKeyPair::derive(&output(9));
        let (send_a, _) = ka.for_role(MsgRole::Initiator);
        let (send_b, _) = kb.for_role(MsgRole::Initiator);
        let sealed = seal_message(send_a, 1, FRAME_DATA, b"x").unwrap();
        assert_eq!(open_message(send_b, &sealed), Err(MsgError::DecryptFailed));
    }
}
