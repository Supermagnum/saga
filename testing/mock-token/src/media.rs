//! Synthetic media frame probe (Phase 1.5 stand-in until SRTP).

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Nonce};

pub const SYNTHETIC_PLAINTEXT: &[u8] = b"saga-synthetic-audio-v1";

pub fn encrypt_frame(key: &[u8; 32], plaintext: &[u8]) -> Vec<u8> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(b"saga-frame00");
    cipher.encrypt(nonce, plaintext).expect("encrypt frame")
}

pub fn decrypt_frame(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = ChaCha20Poly1305::new(key.into());
    let nonce = Nonce::from_slice(b"saga-frame00");
    cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("decrypt: {e}"))
}
