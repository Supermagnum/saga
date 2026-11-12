//! Opus encode/decode at 48 kHz, 20 ms, VBR ~24–32 kbps, in-band FEC on.
//! DTX is off for v1 (simpler loss tests; avoid silence-burst edge cases).

use crate::error::{MediaError, Result};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: opus::Channels = opus::Channels::Mono;
pub const FRAME_MS: u32 = 20;
/// Mono 20 ms at 48 kHz.
pub const FRAME_SAMPLES: usize = SAMPLE_RATE as usize * FRAME_MS as usize / 1000;
/// Target bitrate at the top of the 24–32 kbps VBR band (host quality).
pub const TARGET_BITRATE: i32 = 32_000;

/// §16 voice-disguise hook — no-op until Phase 5.
#[inline]
pub fn disguise_pcm_inplace(_pcm: &mut [i16]) {}

pub struct OpusEncoder {
    inner: opus::Encoder,
}

pub struct OpusDecoder {
    inner: opus::Decoder,
}

impl OpusEncoder {
    pub fn new() -> Result<Self> {
        let mut inner = opus::Encoder::new(SAMPLE_RATE, CHANNELS, opus::Application::Voip)?;
        inner.set_bitrate(opus::Bitrate::Bits(TARGET_BITRATE))?;
        inner.set_vbr(true)?;
        let _ = inner.set_complexity(10);
        inner.set_inband_fec(true)?;
        inner.set_packet_loss_perc(5)?;
        // DTX off (v1).
        let _ = inner.set_dtx(false);
        Ok(Self { inner })
    }

    pub fn set_packet_loss_hint(&mut self, percent: i32) -> Result<()> {
        self.inner.set_packet_loss_perc(percent.clamp(0, 100))?;
        Ok(())
    }

    /// Encode one 20 ms mono frame. Returns encoded length into `out`.
    pub fn encode_frame(&mut self, pcm: &mut [i16], out: &mut [u8]) -> Result<usize> {
        if pcm.len() < FRAME_SAMPLES {
            return Err(MediaError::Failed(format!(
                "short PCM frame: {} < {}",
                pcm.len(),
                FRAME_SAMPLES
            )));
        }
        disguise_pcm_inplace(&mut pcm[..FRAME_SAMPLES]);
        Ok(self.inner.encode(&pcm[..FRAME_SAMPLES], out)?)
    }
}

impl OpusDecoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            inner: opus::Decoder::new(SAMPLE_RATE, CHANNELS)?,
        })
    }

    /// Decode one packet into `out` (must hold ≥ FRAME_SAMPLES).
    pub fn decode_frame(&mut self, packet: &[u8], out: &mut [i16], fec: bool) -> Result<usize> {
        Ok(self.inner.decode(packet, out, fec)?)
    }

    /// PLC: synthesize concealment for a missing frame.
    pub fn conceal(&mut self, out: &mut [i16]) -> Result<usize> {
        Ok(self.inner.decode(&[], out, false)?)
    }
}
