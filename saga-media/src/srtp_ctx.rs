//! SRTP AEAD_AES_256_GCM contexts via webrtc-srtp.

use bytes::Bytes;
use webrtc_srtp::context::Context;
use webrtc_srtp::option::srtp_replay_protection;
use webrtc_srtp::protection_profile::ProtectionProfile;

use crate::error::Result;
use crate::keys::SrtpMaster;

/// Default replay window (matches webrtc-srtp Session default).
const REPLAY_WINDOW: usize = 64;

pub struct SrtpEncrypt {
    ctx: Context,
}

pub struct SrtpDecrypt {
    ctx: Context,
}

impl SrtpEncrypt {
    pub fn new(master: &SrtpMaster) -> Result<Self> {
        let ctx = Context::new(
            master.key.as_slice(),
            master.salt.as_slice(),
            ProtectionProfile::AeadAes256Gcm,
            // Encrypt path does not need replay detection.
            None,
            None,
        )?;
        Ok(Self { ctx })
    }

    /// Encrypt a fully marshalled RTP packet (header + payload).
    pub fn protect(&mut self, rtp_packet: &[u8]) -> Result<Bytes> {
        Ok(self.ctx.encrypt_rtp(rtp_packet)?)
    }
}

impl SrtpDecrypt {
    pub fn new(master: &SrtpMaster) -> Result<Self> {
        let ctx = Context::new(
            master.key.as_slice(),
            master.salt.as_slice(),
            ProtectionProfile::AeadAes256Gcm,
            Some(srtp_replay_protection(REPLAY_WINDOW)),
            None,
        )?;
        Ok(Self { ctx })
    }

    pub fn unprotect(&mut self, srtp_packet: &[u8]) -> Result<Bytes> {
        Ok(self.ctx.decrypt_rtp(srtp_packet)?)
    }
}

/// Conservative max ciphertext size for one Opus frame + RTP + SRTP tag.
/// Opus @ 32 kbps / 20 ms ≈ 80 bytes; RTP header 12; AEAD tag 16; margin.
pub const MAX_MEDIA_DATAGRAM_BYTES: usize = 12 + 200 + 16 + 8;
