//! Saga host media pipeline: Opus + SRTP AEAD_AES_256_GCM over `iroh-roq` datagrams.
//!
//! Encrypted path only. Unsecured Iroh media is intentionally unimplemented
//! until product decides (plain RTP vs no-media vs cellular).

mod audio_metrics;
mod codec;
mod error;
mod jitter;
mod keys;
mod metrics;
mod pipeline;
mod srtp_ctx;

#[cfg(test)]
pub use audio_metrics::{best_correlation, snr_db};
pub use audio_metrics::{best_correlation as signal_correlation, snr_db as signal_snr_db};

pub use codec::{
    disguise_pcm_inplace, OpusDecoder, OpusEncoder, FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE,
    TARGET_BITRATE,
};
pub use error::{MediaError, Result};
pub use jitter::JitterBuffer;
pub use keys::{CallRole, SrtpKeyPair, SrtpMaster, SRTP_MASTER_KEY_LEN, SRTP_MASTER_SALT_LEN};
pub use metrics::{MediaCounters, MediaCountersSnapshot};
pub use pipeline::MediaPipeline;
pub use srtp_ctx::{SrtpDecrypt, SrtpEncrypt, MAX_MEDIA_DATAGRAM_BYTES};
