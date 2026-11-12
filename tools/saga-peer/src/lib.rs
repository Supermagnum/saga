//! Linux reference peer for Saga voice calls (dev/test tool).
//!
//! Reuses `saga-session` / `mock-token` handshake and `saga-media` Opus+SRTP
//! over the pinned `iroh-roq` fork. No duplicated crypto or media code.

pub mod audio;
pub mod call;
pub mod identity;

pub use audio::{load_wav_mono_48k, write_wav_mono_48k};
pub use call::{run_call, run_listen, CallOpts, PeerOutcome};
pub use identity::{
    known_dev_labels, resolve_peer_label, FINGERPRINT_LAPTOP, LABEL_ALICE, LABEL_BOB, LABEL_LAPTOP,
};
