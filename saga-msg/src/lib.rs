//! Encrypted Saga-to-Saga text messaging primitives (issue #41 / Part 3).
//!
//! Message keys are derived from [`saga_session::HandshakeOutput`] with dedicated
//! HKDF labels (never media or confirmation keys). Ciphertexts use AES-256-GCM
//! from the existing dependency set; plaintext is padded to fixed size buckets
//! before sealing so length does not reveal content.

mod keys;
mod pad;
mod replay;
mod seal;

pub use keys::{derive_msg_keys, MsgKeyPair, MsgRole, MSG_KEY_LEN};
pub use pad::{pad_plaintext, unpad_plaintext, PAD_BUCKETS, PAD_MAX_PLAINTEXT};
pub use replay::{ReplayWindow, REPLAY_WINDOW_SIZE};
pub use seal::{
    open_message, seal_message, MsgError, SealedMessage, FRAME_DATA, FRAME_DELIVERED, MSG_ID_LEN,
    WIRE_VERSION,
};

/// Iroh ALPN for Saga messaging (separate from voice / mesh-join / RoQ).
pub const SAGA_MSG_ALPN: &[u8] = b"saga/msg/1";

/// Max UTF-8 text length accepted before padding (issue #41 v1 limit).
pub const MAX_TEXT_BYTES: usize = 4096;
