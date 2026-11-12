//! End-to-end media path over RoQ datagrams (SRTP or plain RTP).

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::{Bytes, BytesMut};
use iroh_roq::{ReceiveFlow, SendFlow, VarInt};
use rtp::packet::Packet as RtpWire;
use tracing::error;
use webrtc_util::marshal::{Marshal, MarshalSize};
use webrtc_util::Unmarshal;

use crate::codec::{OpusDecoder, OpusEncoder, FRAME_SAMPLES};
use crate::error::{MediaError, Result};
use crate::jitter::{JitterBuffer, PopOutcome};
use crate::keys::{CallRole, SrtpKeyPair};
use crate::metrics::MediaCounters;
use crate::policy::{media_permit, refuse_silent_downgrade, MediaWireMode, SecurityResolution};
use crate::srtp_ctx::{
    SrtpDecrypt, SrtpEncrypt, MAX_MEDIA_DATAGRAM_BYTES, MEDIA_TAG_PLAIN, MEDIA_TAG_SRTP,
};
use saga_session::HandshakeOutput;

/// Extra bytes for RoQ flow-id varint (usually 1).
const FLOW_ID_MARGIN: usize = 8;

enum Wire {
    Srtp {
        encrypt: Box<SrtpEncrypt>,
        decrypt: Box<SrtpDecrypt>,
        /// Previous decrypt kept briefly across §5b re-key for in-flight packets.
        decrypt_prev: Option<Box<SrtpDecrypt>>,
    },
    Plain,
}

pub struct MediaPipeline {
    encoder: OpusEncoder,
    decoder: OpusDecoder,
    wire: Wire,
    send: SendFlow,
    recv: ReceiveFlow,
    jb: JitterBuffer,
    ssrc: u32,
    seq: u16,
    timestamp: u32,
    last_opus: Option<Vec<u8>>,
    pub counters: Arc<MediaCounters>,
    max_datagram_override: Option<usize>,
    media_failed: bool,
    /// True if this pipeline was started in SRTP mode (blocks silent downgrade).
    started_encrypted: bool,
}

impl MediaPipeline {
    /// Start media after security resolution (§5 step 4 / §5a).
    pub fn start(
        resolution: SecurityResolution,
        handshake: Option<&HandshakeOutput>,
        role: CallRole,
        send: SendFlow,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        let mode = media_permit(resolution)?;
        match mode {
            MediaWireMode::Srtp => {
                let hs = handshake.ok_or_else(|| {
                    MediaError::Failed("encrypted media requires HandshakeOutput".into())
                })?;
                Self::new_srtp(hs, role, send, recv, counters)
            }
            MediaWireMode::PlainRtp => Self::new_plain(send, recv, counters),
        }
    }

    /// Encrypted path (SRTP). Prefer [`Self::start`] from call UI code.
    pub fn new(
        output: &HandshakeOutput,
        role: CallRole,
        send: SendFlow,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        Self::new_srtp(output, role, send, recv, counters)
    }

    fn new_srtp(
        output: &HandshakeOutput,
        role: CallRole,
        send: SendFlow,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        let keys = SrtpKeyPair::derive(output);
        let (out_m, in_m) = keys.for_role(role);
        let encrypt = SrtpEncrypt::new(out_m)?;
        let decrypt = SrtpDecrypt::new(in_m)?;
        Self::finish_new(
            Wire::Srtp {
                encrypt: Box::new(encrypt),
                decrypt: Box::new(decrypt),
                decrypt_prev: None,
            },
            send,
            recv,
            counters,
            true,
        )
    }

    /// §5b re-key: install new SRTP contexts; keep previous receive context briefly.
    /// Returns elapsed microseconds for the swap (audible-gap budget ~100 ms).
    pub fn rekey_srtp(&mut self, output: &HandshakeOutput, role: CallRole) -> Result<u128> {
        if !self.started_encrypted {
            return Err(MediaError::Failed(
                "rekey only valid on encrypted media".into(),
            ));
        }
        let t0 = std::time::Instant::now();
        let keys = SrtpKeyPair::derive(output);
        let (out_m, in_m) = keys.for_role(role);
        let encrypt = Box::new(SrtpEncrypt::new(out_m)?);
        let decrypt = Box::new(SrtpDecrypt::new(in_m)?);
        match &mut self.wire {
            Wire::Srtp {
                encrypt: e,
                decrypt: d,
                decrypt_prev,
            } => {
                *decrypt_prev = Some(std::mem::replace(d, decrypt));
                *e = encrypt;
            }
            Wire::Plain => {
                return Err(MediaError::NoSilentDowngrade);
            }
        }
        Ok(t0.elapsed().as_micros())
    }

    /// Drop the previous decrypt context after the in-flight window.
    pub fn clear_prev_decrypt(&mut self) {
        if let Wire::Srtp { decrypt_prev, .. } = &mut self.wire {
            *decrypt_prev = None;
        }
    }

    /// Unsecured Iroh: plain RTP inside RoQ (after [`media_permit`] allows it).
    pub fn new_plain(
        send: SendFlow,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        Self::finish_new(Wire::Plain, send, recv, counters, false)
    }

    fn finish_new(
        wire: Wire,
        send: SendFlow,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
        started_encrypted: bool,
    ) -> Result<Self> {
        match send.max_datagram_size() {
            None => {
                error!("media: connection reports no datagram support");
                return Err(MediaError::DatagramsUnsupported);
            }
            Some(limit) => {
                let need = MAX_MEDIA_DATAGRAM_BYTES + FLOW_ID_MARGIN;
                if limit < need {
                    error!("media: max_datagram_size {limit} < required {need}");
                    return Err(MediaError::DatagramTooLarge { need, limit });
                }
            }
        }

        Ok(Self {
            encoder: OpusEncoder::new()?,
            decoder: OpusDecoder::new()?,
            wire,
            send,
            recv,
            jb: JitterBuffer::new(),
            ssrc: rand::random::<u32>(),
            seq: 0,
            timestamp: 0,
            last_opus: None,
            counters,
            max_datagram_override: None,
            media_failed: false,
            started_encrypted,
        })
    }

    pub fn wire_mode(&self) -> MediaWireMode {
        match self.wire {
            Wire::Srtp { .. } => MediaWireMode::Srtp,
            Wire::Plain => MediaWireMode::PlainRtp,
        }
    }

    pub fn started_encrypted(&self) -> bool {
        self.started_encrypted
    }

    /// §5b: refuse constructing a plain pipeline after an encrypted call.
    pub fn assert_no_silent_downgrade(was_encrypted: bool, mode: MediaWireMode) -> Result<()> {
        refuse_silent_downgrade(was_encrypted, mode).map(|_| ())
    }

    pub fn flow_id_hint() -> VarInt {
        VarInt::from_u32(0)
    }

    pub fn set_max_datagram_override_for_test(&mut self, limit: Option<usize>) {
        self.max_datagram_override = limit;
    }

    pub fn media_failed(&self) -> bool {
        self.media_failed
    }

    pub fn jitter_len(&self) -> usize {
        self.jb.len()
    }

    pub fn jitter_cap(&self) -> usize {
        self.jb.max_packets()
    }

    pub fn set_packet_loss_hint(&mut self, percent: i32) -> Result<()> {
        self.encoder.set_packet_loss_hint(percent)
    }

    fn effective_max_datagram(&self) -> Result<usize> {
        if let Some(o) = self.max_datagram_override {
            return Ok(o);
        }
        self.send
            .max_datagram_size()
            .ok_or(MediaError::DatagramsUnsupported)
    }

    fn tag(&self) -> u8 {
        match self.wire {
            Wire::Srtp { .. } => MEDIA_TAG_SRTP,
            Wire::Plain => MEDIA_TAG_PLAIN,
        }
    }

    fn build_rtp(&mut self, opus: &[u8]) -> Result<BytesMut> {
        let packet = RtpWire {
            header: rtp::header::Header {
                version: 2,
                sequence_number: self.seq,
                timestamp: self.timestamp,
                ssrc: self.ssrc,
                payload_type: 111,
                marker: false,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(opus),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        let mut plain = BytesMut::new();
        let sz = packet.marshal_size();
        plain.resize(sz, 0);
        packet.marshal_to(&mut plain)?;
        Ok(plain)
    }

    /// Encode one PCM frame and try to send (non-blocking).
    pub fn send_pcm_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
        if self.media_failed {
            return Err(MediaError::Failed("media already failed".into()));
        }
        let mut opus_buf = [0u8; 400];
        let n = self.encoder.encode_frame(pcm, &mut opus_buf)?;
        let rtp = self.build_rtp(&opus_buf[..n])?;

        let body = match &mut self.wire {
            Wire::Srtp { encrypt, .. } => encrypt.protect(&rtp)?,
            Wire::Plain => Bytes::copy_from_slice(&rtp),
        };

        let mut framed = BytesMut::with_capacity(1 + body.len());
        framed.extend_from_slice(&[self.tag()]);
        framed.extend_from_slice(&body);

        let need = framed.len() + FLOW_ID_MARGIN;
        let limit = self.effective_max_datagram()?;
        if need > limit {
            error!("media: packet {need} exceeds datagram limit {limit}");
            self.media_failed = true;
            return Err(MediaError::DatagramTooLarge { need, limit });
        }

        match self.send.try_send_payload(&framed) {
            Ok(true) => {
                self.counters.sent.fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Ok(false) => {
                self.counters
                    .datagram_blocked_drops
                    .fetch_add(1, Ordering::Relaxed);
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("too large") || msg.contains("TooLarge") {
                    error!("media: datagram TooLarge: {msg}");
                    self.media_failed = true;
                    Err(MediaError::DatagramTooLarge { need, limit })
                } else if msg.contains("not supported") || msg.contains("disabled") {
                    error!("media: datagrams unsupported: {msg}");
                    self.media_failed = true;
                    Err(MediaError::DatagramsUnsupported)
                } else {
                    Err(MediaError::Other(e))
                }
            }
        }
    }

    /// Test helper: inject a tagged media payload into the receive path.
    pub fn inject_framed_for_test(&mut self, framed: &[u8]) {
        self.handle_incoming(framed);
    }

    /// Test helper: inject raw SRTP (no tag) — used by older security tests via tag wrap.
    pub fn inject_srtp_for_test(&mut self, srtp: &[u8]) {
        let mut framed = Vec::with_capacity(1 + srtp.len());
        framed.push(MEDIA_TAG_SRTP);
        framed.extend_from_slice(srtp);
        self.handle_incoming(&framed);
    }

    /// Test helper: build tagged outbound bytes for a PCM frame without sending.
    pub fn protect_pcm_for_test(&mut self, pcm: &mut [i16]) -> Result<Bytes> {
        let mut opus_buf = [0u8; 400];
        let n = self.encoder.encode_frame(pcm, &mut opus_buf)?;
        let rtp = self.build_rtp(&opus_buf[..n])?;
        let body = match &mut self.wire {
            Wire::Srtp { encrypt, .. } => encrypt.protect(&rtp)?,
            Wire::Plain => Bytes::copy_from_slice(&rtp),
        };
        let mut framed = BytesMut::with_capacity(1 + body.len());
        framed.extend_from_slice(&[self.tag()]);
        framed.extend_from_slice(&body);
        Ok(framed.freeze())
    }

    /// Untagged SRTP ciphertext only (for cross-mode injection tests).
    pub fn srtp_body_for_test(&mut self, pcm: &mut [i16]) -> Result<Bytes> {
        let framed = self.protect_pcm_for_test(pcm)?;
        Ok(Bytes::copy_from_slice(&framed[1..]))
    }

    fn handle_incoming(&mut self, framed: &[u8]) {
        if framed.is_empty() {
            self.counters
                .wrong_media_mode
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        let tag = framed[0];
        let body = &framed[1..];
        let expect = self.tag();
        if tag != expect {
            self.counters
                .wrong_media_mode
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        match &mut self.wire {
            Wire::Srtp {
                decrypt,
                decrypt_prev,
                ..
            } => match decrypt.unprotect(body) {
                Ok(plain) => self.push_rtp_plain(&plain),
                Err(MediaError::Srtp(webrtc_srtp::Error::SrtpSsrcDuplicated(_, _)))
                | Err(MediaError::Srtp(webrtc_srtp::Error::ErrDuplicated)) => {
                    self.counters.replay.fetch_add(1, Ordering::Relaxed);
                }
                Err(_) => {
                    // §5b: try previous receive context for in-flight packets.
                    if let Some(prev) = decrypt_prev.as_mut() {
                        match prev.unprotect(body) {
                            Ok(plain) => {
                                self.push_rtp_plain(&plain);
                                return;
                            }
                            Err(MediaError::Srtp(webrtc_srtp::Error::SrtpSsrcDuplicated(_, _)))
                            | Err(MediaError::Srtp(webrtc_srtp::Error::ErrDuplicated)) => {
                                self.counters.replay.fetch_add(1, Ordering::Relaxed);
                                return;
                            }
                            Err(_) => {}
                        }
                    }
                    self.counters
                        .tamper_or_auth_fail
                        .fetch_add(1, Ordering::Relaxed);
                }
            },
            Wire::Plain => self.push_rtp_plain(body),
        }
    }

    fn push_rtp_plain(&mut self, plain: &[u8]) {
        let mut buf = plain;
        match rtp::header::Header::unmarshal(&mut buf) {
            Ok(hdr) => {
                let payload = buf.to_vec();
                self.jb.push(hdr.sequence_number, payload);
            }
            Err(_) => {
                self.counters
                    .tamper_or_auth_fail
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub async fn recv_one_datagram(&mut self) -> Result<()> {
        let bytes = self.recv.read_bytes().await.map_err(MediaError::Other)?;
        self.handle_incoming(&bytes);
        Ok(())
    }

    pub fn playout_frame(&mut self, out: &mut [i16]) -> Result<usize> {
        if out.len() < FRAME_SAMPLES {
            return Err(MediaError::Failed("short playout buffer".into()));
        }
        match self.jb.pop_ready() {
            PopOutcome::Packet(pkt) => {
                let n = self.decoder.decode_frame(&pkt.payload, out, false)?;
                self.last_opus = Some(pkt.payload);
                self.counters.played.fetch_add(1, Ordering::Relaxed);
                Ok(n)
            }
            PopOutcome::Wait => {
                // Startup / reorder window / empty: silence, do not advance.
                out[..FRAME_SAMPLES].fill(0);
                Ok(FRAME_SAMPLES)
            }
            PopOutcome::Lost => {
                // Confirmed gap (wait_ticks / ahead threshold). Conceal once.
                if !self.jb.playout_started() {
                    out[..FRAME_SAMPLES].fill(0);
                    return Ok(FRAME_SAMPLES);
                }
                let n = self.decoder.conceal(out)?;
                self.counters.plc_frames.fetch_add(1, Ordering::Relaxed);
                Ok(n)
            }
        }
    }

    pub fn record_blocked_drop_for_test(&self) {
        self.counters
            .datagram_blocked_drops
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Test helper: whether a real packet has been played out yet.
    pub fn jb_playout_started_for_test(&self) -> bool {
        self.jb.playout_started()
    }
}
