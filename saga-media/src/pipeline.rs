//! End-to-end encrypted media path over RoQ datagrams.

use std::sync::atomic::Ordering;
use std::sync::Arc;

use bytes::BytesMut;
use iroh_roq::{ReceiveFlow, SendFlow, VarInt};
use rtp::packet::Packet as RtpWire;
use tracing::error;
use webrtc_util::marshal::{Marshal, MarshalSize};
use webrtc_util::Unmarshal;

use crate::codec::{OpusDecoder, OpusEncoder, FRAME_SAMPLES};
use crate::error::{MediaError, Result};
use crate::jitter::JitterBuffer;
use crate::keys::{CallRole, SrtpKeyPair};
use crate::metrics::MediaCounters;
use crate::srtp_ctx::{SrtpDecrypt, SrtpEncrypt, MAX_MEDIA_DATAGRAM_BYTES};
use saga_session::HandshakeOutput;

/// Extra bytes for RoQ flow-id varint (usually 1).
const FLOW_ID_MARGIN: usize = 8;

pub struct MediaPipeline {
    encoder: OpusEncoder,
    decoder: OpusDecoder,
    encrypt: SrtpEncrypt,
    decrypt: SrtpDecrypt,
    send: SendFlow,
    recv: ReceiveFlow,
    jb: JitterBuffer,
    ssrc: u32,
    seq: u16,
    timestamp: u32,
    /// Last received Opus payload (for FEC decode).
    last_opus: Option<Vec<u8>>,
    pub counters: Arc<MediaCounters>,
    /// Override for tests (artificial small limit).
    max_datagram_override: Option<usize>,
    media_failed: bool,
}

impl MediaPipeline {
    pub fn new(
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
            encrypt,
            decrypt,
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
        })
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

    /// Encode one PCM frame and try to send as SRTP over RoQ (non-blocking).
    pub fn send_pcm_frame(&mut self, pcm: &mut [i16]) -> Result<()> {
        if self.media_failed {
            return Err(MediaError::Failed("media already failed".into()));
        }
        let mut opus_buf = [0u8; 400];
        let n = self.encoder.encode_frame(pcm, &mut opus_buf)?;
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
            payload: bytes::Bytes::copy_from_slice(&opus_buf[..n]),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);

        let mut plain = BytesMut::new();
        let sz = packet.marshal_size();
        plain.resize(sz, 0);
        packet.marshal_to(&mut plain)?;
        let protected = self.encrypt.protect(&plain)?;

        let need = protected.len() + FLOW_ID_MARGIN;
        let limit = self.effective_max_datagram()?;
        if need > limit {
            error!("media: SRTP packet {need} exceeds datagram limit {limit}");
            self.media_failed = true;
            return Err(MediaError::DatagramTooLarge { need, limit });
        }

        match self.send.try_send_payload(&protected) {
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

    /// Test helper: inject a raw SRTP (or tampered) packet into the decrypt/JB path.
    pub fn inject_srtp_for_test(&mut self, srtp: &[u8]) {
        self.handle_incoming_srtp(srtp);
    }

    /// Test helper: build protected bytes for a PCM frame without sending.
    pub fn protect_pcm_for_test(&mut self, pcm: &mut [i16]) -> Result<bytes::Bytes> {
        let mut opus_buf = [0u8; 400];
        let n = self.encoder.encode_frame(pcm, &mut opus_buf)?;
        let packet = RtpWire {
            header: rtp::header::Header {
                version: 2,
                sequence_number: self.seq,
                timestamp: self.timestamp,
                ssrc: self.ssrc,
                payload_type: 111,
                ..Default::default()
            },
            payload: bytes::Bytes::copy_from_slice(&opus_buf[..n]),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        let mut plain = BytesMut::new();
        let sz = packet.marshal_size();
        plain.resize(sz, 0);
        packet.marshal_to(&mut plain)?;
        self.encrypt.protect(&plain)
    }

    fn handle_incoming_srtp(&mut self, srtp: &[u8]) {
        match self.decrypt.unprotect(srtp) {
            Ok(plain) => {
                let mut buf = plain.as_ref();
                match rtp::header::Header::unmarshal(&mut buf) {
                    Ok(hdr) => {
                        // `buf` advanced past header; remainder is Opus payload.
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
            Err(MediaError::Srtp(webrtc_srtp::Error::SrtpSsrcDuplicated(_, _)))
            | Err(MediaError::Srtp(webrtc_srtp::Error::ErrDuplicated)) => {
                self.counters.replay.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {
                self.counters
                    .tamper_or_auth_fail
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Pull from RoQ (non-blocking try via timeout 0 is caller's job — this awaits one packet).
    pub async fn recv_one_datagram(&mut self) -> Result<()> {
        let bytes = self.recv.read_bytes().await.map_err(MediaError::Other)?;
        self.handle_incoming_srtp(&bytes);
        Ok(())
    }

    /// Playout one frame into `out` (≥ FRAME_SAMPLES). Uses PLC/FEC on gaps.
    pub fn playout_frame(&mut self, out: &mut [i16]) -> Result<usize> {
        if out.len() < FRAME_SAMPLES {
            return Err(MediaError::Failed("short playout buffer".into()));
        }
        match self.jb.pop_ready() {
            Some(pkt) => {
                // If we had a previous packet, try FEC decode of previous using this as next.
                let n = self.decoder.decode_frame(&pkt.payload, out, false)?;
                self.last_opus = Some(pkt.payload);
                self.counters.played.fetch_add(1, Ordering::Relaxed);
                Ok(n)
            }
            None => {
                // Gap: attempt FEC with last, else PLC.
                if let Some(ref last) = self.last_opus {
                    // fec=true asks decoder to reconstruct prior missing frame using this...
                    // Without the next packet FEC needs the next; for underrun use PLC.
                    let _ = last;
                }
                let n = self.decoder.conceal(out)?;
                self.counters.plc_frames.fetch_add(1, Ordering::Relaxed);
                self.jb.advance_on_underrun();
                Ok(n)
            }
        }
    }

    /// Force a blocked-drop increment (unit tests without filling QUIC buffers).
    pub fn record_blocked_drop_for_test(&self) {
        self.counters
            .datagram_blocked_drops
            .fetch_add(1, Ordering::Relaxed);
    }
}
