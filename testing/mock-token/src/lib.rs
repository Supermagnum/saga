//! Mock Galdralag token implementing ephemeral-session handshake primitives (saga-spec.md section 15.1).

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};
use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};
use hkdf::Hkdf;
use log::info;
use sha2::{Digest, Sha256};
use x25519_dalek::{PublicKey as X25519PublicKey, SharedSecret, StaticSecret};

pub const MOCK_TOKEN_MARKER: &str = "MOCK_TOKEN";

const HELLO_LEN: usize = 32 + 32 + 64;
const CONFIRM_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandshakeOutcome {
    Encrypted { session_key: [u8; 32] },
    Failed { reason: String },
}

#[derive(Clone)]
pub struct MockIdentity {
    pub label: String,
    pub x25519: StaticSecret,
    pub ed25519: SigningKey,
}

impl MockIdentity {
    pub fn from_label(label: &str) -> Self {
        let material = format!("saga-mock-token-v1:{label}");
        let hash = blake3::hash(material.as_bytes());
        let seed: [u8; 32] = *hash.as_bytes();
        Self {
            label: label.to_string(),
            x25519: StaticSecret::from(seed),
            ed25519: SigningKey::from_bytes(&seed),
        }
    }

    pub fn x25519_public(&self) -> X25519PublicKey {
        X25519PublicKey::from(&self.x25519)
    }

    pub fn ed25519_public(&self) -> VerifyingKey {
        self.ed25519.verifying_key()
    }
}

fn transcript_hash(initiator_pk: &[u8; 32], responder_pk: &[u8; 32]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"saga-ephemeral-session-v1");
    hasher.update(initiator_pk);
    hasher.update(responder_pk);
    hasher.finalize().into()
}

fn derive_session_key(shared: &SharedSecret, transcript: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(b"saga-mock-session"), shared.as_bytes());
    let mut okm = [0u8; 32];
    hk.expand(transcript, &mut okm)
        .expect("hkdf expand 32 bytes");
    okm
}

fn pack_hello(identity: &MockIdentity) -> [u8; HELLO_LEN] {
    let mut buf = [0u8; HELLO_LEN];
    let x_pk = identity.x25519_public().to_bytes();
    let e_pk = identity.ed25519_public().to_bytes();
    buf[..32].copy_from_slice(&x_pk);
    buf[32..64].copy_from_slice(&e_pk);
    let sig = identity.ed25519.sign(&buf[..64]);
    buf[64..].copy_from_slice(&sig.to_bytes());
    buf
}

fn parse_hello(buf: &[u8; HELLO_LEN]) -> Result<(X25519PublicKey, VerifyingKey), String> {
    let mut x_bytes = [0u8; 32];
    let mut e_bytes = [0u8; 32];
    let mut sig_bytes = [0u8; 64];
    x_bytes.copy_from_slice(&buf[..32]);
    e_bytes.copy_from_slice(&buf[32..64]);
    sig_bytes.copy_from_slice(&buf[64..]);
    let verifying_key =
        VerifyingKey::from_bytes(&e_bytes).map_err(|e| format!("invalid ed25519 pk: {e}"))?;
    verifying_key
        .verify_strict(&buf[..64], &ed25519_dalek::Signature::from_bytes(&sig_bytes))
        .map_err(|e| format!("hello signature invalid: {e}"))?;
    Ok((X25519PublicKey::from(x_bytes), verifying_key))
}

/// Initiator side of ephemeral-session over a byte stream.
pub async fn run_initiator<R, W>(
    identity: &MockIdentity,
    peer_label: &str,
    force_fail: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "mock-token force_fail".into(),
        };
    }

    let peer = MockIdentity::from_label(peer_label);
    let hello = pack_hello(identity);
    info!(
        "[{MOCK_TOKEN_MARKER}] initiator [{}] sending hello",
        identity.label
    );
    if writer.write_all(&hello).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "write hello failed".into(),
        };
    }

    let mut resp = [0u8; HELLO_LEN];
    if reader.read_exact(&mut resp).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "read hello response failed".into(),
        };
    }
    let (resp_x_pk, _) = match parse_hello(&resp) {
        Ok(v) => v,
        Err(e) => return HandshakeOutcome::Failed { reason: e },
    };
    if resp_x_pk.to_bytes() != peer.x25519_public().to_bytes() {
        return HandshakeOutcome::Failed {
            reason: "responder x25519 pubkey mismatch".into(),
        };
    }

    let init_x_bytes = identity.x25519_public().to_bytes();
    let transcript = transcript_hash(&init_x_bytes, &resp_x_pk.to_bytes());
    let shared = identity.x25519.diffie_hellman(&resp_x_pk);
    let session_key = derive_session_key(&shared, &transcript);

    let confirm = identity.ed25519.sign(&transcript);
    if writer.write_all(&confirm.to_bytes()).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "write confirm failed".into(),
        };
    }
    let _ = writer.flush().await;

    info!(
        "[{MOCK_TOKEN_MARKER}] initiator handshake complete session_key_prefix={:02x}{:02x}",
        session_key[0], session_key[1]
    );
    HandshakeOutcome::Encrypted { session_key }
}

/// Responder side of ephemeral-session over a byte stream.
pub async fn run_responder<R, W>(
    identity: &MockIdentity,
    force_fail: bool,
    reader: &mut R,
    writer: &mut W,
) -> HandshakeOutcome
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    if force_fail {
        return HandshakeOutcome::Failed {
            reason: "mock-token force_fail".into(),
        };
    }

    let mut hello = [0u8; HELLO_LEN];
    if reader.read_exact(&mut hello).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "read hello failed".into(),
        };
    }
    let (init_x_pk, init_ed_pk) = match parse_hello(&hello) {
        Ok(v) => v,
        Err(e) => return HandshakeOutcome::Failed { reason: e },
    };

    let resp = pack_hello(identity);
    if writer.write_all(&resp).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "write hello response failed".into(),
        };
    }

    let mut confirm = [0u8; CONFIRM_LEN];
    if reader.read_exact(&mut confirm).await.is_err() {
        return HandshakeOutcome::Failed {
            reason: "read confirm failed".into(),
        };
    }
    let transcript = transcript_hash(&init_x_pk.to_bytes(), &identity.x25519_public().to_bytes());
    if init_ed_pk
        .verify_strict(&transcript, &ed25519_dalek::Signature::from_bytes(&confirm))
        .is_err()
    {
        return HandshakeOutcome::Failed {
            reason: "confirm signature invalid".into(),
        };
    }

    let shared = identity.x25519.diffie_hellman(&init_x_pk);
    let session_key = derive_session_key(&shared, &transcript);

    info!(
        "[{MOCK_TOKEN_MARKER}] responder [{}] handshake complete session_key_prefix={:02x}{:02x}",
        identity.label, session_key[0], session_key[1]
    );
    HandshakeOutcome::Encrypted { session_key }
}

pub fn encrypt_frame(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(b"saga-frame00");
    cipher.encrypt(nonce, plaintext).expect("encrypt frame")
}

pub fn decrypt_frame(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(b"saga-frame00");
    cipher.decrypt(nonce, ciphertext).map_err(|e| format!("decrypt: {e}"))
}

pub const SYNTHETIC_PLAINTEXT: &[u8] = b"saga-synthetic-audio-v1";

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn handshake_round_trip() {
        let alice = MockIdentity::from_label("alicepeer1");
        let bob = MockIdentity::from_label("bobpeer12");

        let (io_a, io_b) = duplex(4096);
        let (mut a_read, mut a_write) = tokio::io::split(io_a);
        let (mut b_read, mut b_write) = tokio::io::split(io_b);

        let (a_out, b_out) = tokio::join!(
            run_initiator(&alice, "bobpeer12", false, &mut a_read, &mut a_write),
            run_responder(&bob, false, &mut b_read, &mut b_write),
        );

        match (&a_out, &b_out) {
            (
                HandshakeOutcome::Encrypted { session_key: ka },
                HandshakeOutcome::Encrypted { session_key: kb },
            ) => assert_eq!(ka, kb),
            _ => panic!("handshake failed: {a_out:?} {b_out:?}"),
        }
    }
}
