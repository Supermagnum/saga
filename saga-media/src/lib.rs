//! Saga host media pipeline: Opus over `iroh-roq` datagrams.
//!
//! Two-party calls use [`MediaPipeline`]; multi-peer mesh calls (up to 4
//! participants, pairwise keys, encode once / encrypt per link) use
//! [`MeshMixer`].
//!
//! - **Encrypted:** SRTP AEAD_AES_256_GCM (token-authenticated session keys).
//! - **Unsecured Iroh:** plain RTP inside RoQ after security resolves to
//!   not-encrypted (QUIC still encrypts the hop; no SRTP / no token auth).
//!   Previously-encrypted contacts require an explicit §5a continue; mid-call
//!   encrypted failure never silent-switches to plain RTP (§5b).

mod audio_metrics;
mod codec;
pub mod disguise;
mod error;
mod jitter;
mod keys;
mod mesh;
mod metrics;
mod pipeline;
mod policy;
mod srtp_ctx;

#[cfg(test)]
pub use audio_metrics::{best_correlation, snr_db};
pub use audio_metrics::{best_correlation as signal_correlation, snr_db as signal_snr_db};

pub use codec::{
    disguise_pcm_inplace, OpusDecoder, OpusEncoder, FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE,
    TARGET_BITRATE,
};
pub use disguise::{
    disguise_begin_call, disguise_end_call, disguise_is_active, disguise_last_alpha_for_test,
    disguise_last_pitch_for_test, disguise_process_buffer, disguise_settings,
    set_disguise_settings, DisguiseMode, DisguiseSettings, FormantStrength,
};
pub use error::{MediaError, Result};
pub use jitter::{JitterBuffer, CAP_MS as JB_CAP_MS, TARGET_MS_HIGH, TARGET_MS_MIN};
pub use keys::{CallRole, SrtpKeyPair, SrtpMaster, SRTP_MASTER_KEY_LEN, SRTP_MASTER_SALT_LEN};
pub use mesh::{
    mesh_link_plain, mesh_link_srtp, mix_saturate_add, LinkIngest, LinkReceiver, LinkSender,
    MeshEncoder, MeshMixer, PeerId, MESH_MAX_LINKS, MESH_MAX_PARTICIPANTS,
};
pub use metrics::{MediaCounters, MediaCountersSnapshot};
pub use pipeline::MediaPipeline;
pub use policy::{media_permit, refuse_silent_downgrade, MediaWireMode, SecurityResolution};
pub use srtp_ctx::{
    SrtpDecrypt, SrtpEncrypt, ANSWERED_SIGNAL_PAYLOAD, DISGUISE_SIGNAL_PREFIX,
    MAX_MEDIA_DATAGRAM_BYTES, MEDIA_TAG_ANSWERED, MEDIA_TAG_DISGUISE, MEDIA_TAG_PLAIN,
    MEDIA_TAG_SRTP,
};
