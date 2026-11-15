//! Multi-peer mesh media: encode once, encrypt per link, mix locally.
//!
//! Topology is a full mesh with pairwise keys (3 participants = 3 links,
//! 4 participants = 6 links, i.e. at most [`MESH_MAX_LINKS`] links per node).
//! There is no group key: each link carries its own SRTP contexts derived from
//! that pair's [`HandshakeOutput`] with the same HKDF labels as the 1:1 path.
//!
//! Send path: [`MeshEncoder`] runs Opus and builds the RTP packet once per PCM
//! frame; every [`LinkSender`] protects that same plain RTP with its own key
//! and sends it over its own RoQ flow.
//!
//! Receive path: every [`LinkReceiver`] owns an SRTP context, a jitter buffer
//! and an Opus decoder for one remote peer; [`MeshMixer::playout_mixed`] pulls
//! one frame from each and saturate-adds them into one output frame.
//!
//! The 1:1 [`crate::MediaPipeline`] is untouched and remains the path for
//! two-party calls.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use bytes::{Bytes, BytesMut};
use iroh_roq::{ReceiveFlow, SendFlow};
use rtp::packet::Packet as RtpWire;
use saga_session::HandshakeOutput;
use tracing::error;
use webrtc_util::marshal::{Marshal, MarshalSize};
use webrtc_util::Unmarshal;

use crate::codec::{OpusDecoder, OpusEncoder, FRAME_SAMPLES};
use crate::error::{MediaError, Result};
use crate::jitter::{JitterBuffer, PopOutcome};
use crate::keys::{CallRole, SrtpKeyPair};
use crate::metrics::MediaCounters;
use crate::pipeline::FLOW_ID_MARGIN;
use crate::policy::{media_permit, MediaWireMode, SecurityResolution};
use crate::srtp_ctx::{
    SrtpDecrypt, SrtpEncrypt, ANSWERED_SIGNAL_PAYLOAD, MAX_MEDIA_DATAGRAM_BYTES,
    MEDIA_TAG_ANSWERED, MEDIA_TAG_PLAIN, MEDIA_TAG_SRTP,
};

/// Maximum participants in one mesh call (including the local peer).
pub const MESH_MAX_PARTICIPANTS: usize = 4;
/// Maximum number of remote links one node holds.
pub const MESH_MAX_LINKS: usize = MESH_MAX_PARTICIPANTS - 1;

const RTP_PAYLOAD_TYPE: u8 = 111;

/// Remote peer identifier (Iroh EndpointId bytes).
pub type PeerId = [u8; 32];

/// Saturating add of `src` into `dst` (i16 PCM mix).
pub fn mix_saturate_add(dst: &mut [i16], src: &[i16]) {
    for (d, s) in dst.iter_mut().zip(src.iter()) {
        *d = d.saturating_add(*s);
    }
}

fn check_datagram_limit(send: &SendFlow) -> Result<()> {
    match send.max_datagram_size() {
        None => {
            error!("mesh: connection reports no datagram support");
            Err(MediaError::DatagramsUnsupported)
        }
        Some(limit) => {
            let need = MAX_MEDIA_DATAGRAM_BYTES + FLOW_ID_MARGIN;
            if limit < need {
                error!("mesh: max_datagram_size {limit} < required {need}");
                return Err(MediaError::DatagramTooLarge { need, limit });
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// MeshEncoder
// ---------------------------------------------------------------------------

/// One Opus encoder plus the shared RTP header state for all links.
pub struct MeshEncoder {
    encoder: OpusEncoder,
    ssrc: u32,
    seq: u16,
    timestamp: u32,
}

impl MeshEncoder {
    pub fn new() -> Result<Self> {
        Ok(Self {
            encoder: OpusEncoder::new()?,
            ssrc: rand::random::<u32>(),
            seq: 0,
            timestamp: 0,
        })
    }

    pub fn set_packet_loss_hint(&mut self, percent: i32) -> Result<()> {
        self.encoder.set_packet_loss_hint(percent)
    }

    pub fn ssrc(&self) -> u32 {
        self.ssrc
    }

    /// Encode one PCM frame and return the marshalled plain RTP packet.
    /// Called once per frame regardless of the number of links.
    pub fn encode_rtp(&mut self, pcm: &mut [i16]) -> Result<Bytes> {
        let mut opus_buf = [0u8; 400];
        let n = self.encoder.encode_frame(pcm, &mut opus_buf)?;
        self.build_rtp(&opus_buf[..n])
    }

    fn build_rtp(&mut self, opus: &[u8]) -> Result<Bytes> {
        let packet = RtpWire {
            header: rtp::header::Header {
                version: 2,
                sequence_number: self.seq,
                timestamp: self.timestamp,
                ssrc: self.ssrc,
                payload_type: RTP_PAYLOAD_TYPE,
                marker: false,
                ..Default::default()
            },
            payload: Bytes::copy_from_slice(opus),
        };
        self.seq = self.seq.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(FRAME_SAMPLES as u32);
        let mut plain = BytesMut::new();
        plain.resize(packet.marshal_size(), 0);
        packet.marshal_to(&mut plain)?;
        Ok(plain.freeze())
    }
}

// ---------------------------------------------------------------------------
// LinkWire (per-link protect step, no I/O)
// ---------------------------------------------------------------------------

enum SendWire {
    Srtp(Box<SrtpEncrypt>),
    Plain,
}

/// Per-link protect step: plain RTP in, tagged datagram payload out.
struct LinkWire {
    wire: SendWire,
}

impl LinkWire {
    fn tag(&self) -> u8 {
        match self.wire {
            SendWire::Srtp(_) => MEDIA_TAG_SRTP,
            SendWire::Plain => MEDIA_TAG_PLAIN,
        }
    }

    fn mode(&self) -> MediaWireMode {
        match self.wire {
            SendWire::Srtp(_) => MediaWireMode::Srtp,
            SendWire::Plain => MediaWireMode::PlainRtp,
        }
    }

    fn frame(&mut self, rtp: &[u8]) -> Result<Bytes> {
        let body = match &mut self.wire {
            SendWire::Srtp(enc) => enc.protect(rtp)?,
            SendWire::Plain => Bytes::copy_from_slice(rtp),
        };
        let mut framed = BytesMut::with_capacity(1 + body.len());
        framed.extend_from_slice(&[self.tag()]);
        framed.extend_from_slice(&body);
        Ok(framed.freeze())
    }
}

// ---------------------------------------------------------------------------
// LinkSender
// ---------------------------------------------------------------------------

/// Per-link outbound half: protect (SRTP or plain) and send over RoQ.
pub struct LinkSender {
    wire: LinkWire,
    send: SendFlow,
    pub counters: Arc<MediaCounters>,
    /// Pre-answer gate: unarmed links drop all capture.
    armed: bool,
    muted: bool,
    held: bool,
    gated_drops: u64,
    failed: bool,
}

impl LinkSender {
    /// SRTP link sender from one directional master.
    pub fn new_srtp(
        master: &crate::keys::SrtpMaster,
        send: SendFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        check_datagram_limit(&send)?;
        Ok(Self::finish(
            SendWire::Srtp(Box::new(SrtpEncrypt::new(master)?)),
            send,
            counters,
        ))
    }

    /// Plain RTP link sender (QUIC-only protection). Prefer
    /// [`mesh_link_plain`], which enforces the explicit-confirm policy.
    fn new_plain(send: SendFlow, counters: Arc<MediaCounters>) -> Result<Self> {
        check_datagram_limit(&send)?;
        Ok(Self::finish(SendWire::Plain, send, counters))
    }

    fn finish(wire: SendWire, send: SendFlow, counters: Arc<MediaCounters>) -> Self {
        Self {
            wire: LinkWire { wire },
            send,
            counters,
            armed: true,
            muted: false,
            held: false,
            gated_drops: 0,
            failed: false,
        }
    }

    pub fn wire_mode(&self) -> MediaWireMode {
        self.wire.mode()
    }

    pub fn is_armed(&self) -> bool {
        self.armed
    }

    /// Arm or disarm this link (pre-answer gate).
    pub fn set_armed(&mut self, armed: bool) {
        self.armed = armed;
    }

    pub fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
    }

    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// Capture frames dropped because the link was not armed.
    pub fn gated_drops(&self) -> u64 {
        self.gated_drops
    }

    /// True when this link would send a capture frame right now.
    pub fn can_send(&self) -> bool {
        self.armed && !self.muted && !self.held && !self.failed
    }

    /// Protect an already-built plain RTP packet and return the tagged payload
    /// without sending (used by tests and fault injectors).
    pub fn frame_rtp(&mut self, rtp: &[u8]) -> Result<Bytes> {
        self.wire.frame(rtp)
    }

    /// Protect and send plain RTP. Returns `Ok(true)` when queued on the wire,
    /// `Ok(false)` when the frame was dropped (gate, mute, hold, or full buffer).
    pub fn send_rtp(&mut self, rtp: &[u8]) -> Result<bool> {
        if self.failed {
            return Err(MediaError::Failed("mesh link already failed".into()));
        }
        if !self.armed {
            self.gated_drops += 1;
            return Ok(false);
        }
        if self.muted || self.held {
            return Ok(false);
        }
        let framed = self.wire.frame(rtp)?;
        let need = framed.len() + FLOW_ID_MARGIN;
        let limit = self
            .send
            .max_datagram_size()
            .ok_or(MediaError::DatagramsUnsupported)?;
        if need > limit {
            error!("mesh: packet {need} exceeds datagram limit {limit}");
            self.failed = true;
            return Err(MediaError::DatagramTooLarge { need, limit });
        }
        match self.send.try_send_payload(&framed) {
            Ok(true) => {
                self.counters.sent.fetch_add(1, Ordering::Relaxed);
                Ok(true)
            }
            Ok(false) => {
                self.counters
                    .datagram_blocked_drops
                    .fetch_add(1, Ordering::Relaxed);
                Ok(false)
            }
            Err(e) => Err(MediaError::Other(e)),
        }
    }

    /// One-shot control datagram: remote peer arms its send/playout on receipt.
    /// Sent regardless of the armed state.
    pub fn send_answered_signal(&mut self) -> Result<()> {
        let mut framed = BytesMut::with_capacity(1 + ANSWERED_SIGNAL_PAYLOAD.len());
        framed.extend_from_slice(&[MEDIA_TAG_ANSWERED]);
        framed.extend_from_slice(ANSWERED_SIGNAL_PAYLOAD);
        match self.send.try_send_payload(&framed) {
            Ok(_) => Ok(()),
            Err(e) => Err(MediaError::Other(e)),
        }
    }
}

// ---------------------------------------------------------------------------
// LinkReceiver
// ---------------------------------------------------------------------------

enum RecvWire {
    Srtp(Box<SrtpDecrypt>),
    Plain,
}

/// Result of ingesting one datagram on a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkIngest {
    /// RTP media accepted into the jitter buffer (or dropped and counted).
    Media,
    /// Explicit answered control datagram.
    Answered,
}

/// Per-link inbound half: SRTP + own jitter buffer + own Opus decoder.
pub struct LinkReceiver {
    wire: RecvWire,
    recv: ReceiveFlow,
    decoder: OpusDecoder,
    jb: JitterBuffer,
    pub counters: Arc<MediaCounters>,
    /// Pre-answer gate: media is discarded and playout is silent while true.
    gated: bool,
    held: bool,
    gated_drops: u64,
}

impl LinkReceiver {
    pub fn new_srtp(
        master: &crate::keys::SrtpMaster,
        recv: ReceiveFlow,
        counters: Arc<MediaCounters>,
    ) -> Result<Self> {
        Self::finish(
            RecvWire::Srtp(Box::new(SrtpDecrypt::new(master)?)),
            recv,
            counters,
        )
    }

    fn new_plain(recv: ReceiveFlow, counters: Arc<MediaCounters>) -> Result<Self> {
        Self::finish(RecvWire::Plain, recv, counters)
    }

    fn finish(wire: RecvWire, recv: ReceiveFlow, counters: Arc<MediaCounters>) -> Result<Self> {
        Ok(Self {
            wire,
            recv,
            decoder: OpusDecoder::new()?,
            jb: JitterBuffer::new(),
            counters,
            gated: false,
            held: false,
            gated_drops: 0,
        })
    }

    fn expected_tag(&self) -> u8 {
        match self.wire {
            RecvWire::Srtp(_) => MEDIA_TAG_SRTP,
            RecvWire::Plain => MEDIA_TAG_PLAIN,
        }
    }

    pub fn wire_mode(&self) -> MediaWireMode {
        match self.wire {
            RecvWire::Srtp(_) => MediaWireMode::Srtp,
            RecvWire::Plain => MediaWireMode::PlainRtp,
        }
    }

    pub fn set_gated(&mut self, gated: bool) {
        self.gated = gated;
    }

    pub fn is_gated(&self) -> bool {
        self.gated
    }

    pub fn set_held(&mut self, held: bool) {
        self.held = held;
    }

    /// Media datagrams discarded because the link was still pre-answer.
    pub fn gated_drops(&self) -> u64 {
        self.gated_drops
    }

    pub fn jitter_len(&self) -> usize {
        self.jb.len()
    }

    pub fn jitter_cap(&self) -> usize {
        self.jb.max_packets()
    }

    /// Discard buffered pre-answer media.
    pub fn flush_pre_answer(&mut self) {
        self.jb.flush_pre_answer();
    }

    /// True when `bytes` is the explicit callee-answered control datagram.
    pub fn is_answered_signal(bytes: &[u8]) -> bool {
        bytes.first() == Some(&MEDIA_TAG_ANSWERED)
            && bytes.get(1..) == Some(ANSWERED_SIGNAL_PAYLOAD)
    }

    /// Read one RoQ datagram without decoding.
    pub async fn read_datagram_bytes(&mut self) -> Result<Bytes> {
        self.recv.read_bytes().await.map_err(MediaError::Other)
    }

    /// Await and ingest one datagram.
    pub async fn recv_one_datagram(&mut self) -> Result<LinkIngest> {
        let bytes = self.read_datagram_bytes().await?;
        Ok(self.ingest_datagram(&bytes))
    }

    /// Non-blocking drain of already-queued datagrams, bounded by `max` and by
    /// jitter-buffer headroom. Returns how many datagrams were ingested and
    /// whether an answered signal was among them.
    pub async fn drain(&mut self, max: u32) -> (u32, bool) {
        let mut n = 0u32;
        let mut answered = false;
        while n < max {
            if self.jb.len() + 1 >= self.jb.max_packets() {
                break;
            }
            match tokio::time::timeout(Duration::ZERO, self.read_datagram_bytes()).await {
                Ok(Ok(bytes)) => {
                    if self.ingest_datagram(&bytes) == LinkIngest::Answered {
                        answered = true;
                    }
                    n += 1;
                }
                _ => break,
            }
        }
        (n, answered)
    }

    /// Push a framed RoQ payload into SRTP / the jitter buffer.
    pub fn ingest_datagram(&mut self, framed: &[u8]) -> LinkIngest {
        if Self::is_answered_signal(framed) {
            return LinkIngest::Answered;
        }
        if framed.is_empty() {
            self.counters
                .wrong_media_mode
                .fetch_add(1, Ordering::Relaxed);
            return LinkIngest::Media;
        }
        if framed[0] != self.expected_tag() {
            self.counters
                .wrong_media_mode
                .fetch_add(1, Ordering::Relaxed);
            return LinkIngest::Media;
        }
        if self.gated {
            self.gated_drops += 1;
            return LinkIngest::Media;
        }
        let body = &framed[1..];
        let plain = match &mut self.wire {
            RecvWire::Srtp(dec) => match dec.unprotect(body) {
                Ok(p) => p,
                Err(MediaError::Srtp(webrtc_srtp::Error::SrtpSsrcDuplicated(_, _)))
                | Err(MediaError::Srtp(webrtc_srtp::Error::ErrDuplicated)) => {
                    self.counters.replay.fetch_add(1, Ordering::Relaxed);
                    return LinkIngest::Media;
                }
                Err(_) => {
                    self.counters
                        .tamper_or_auth_fail
                        .fetch_add(1, Ordering::Relaxed);
                    return LinkIngest::Media;
                }
            },
            RecvWire::Plain => Bytes::copy_from_slice(body),
        };
        self.push_rtp_plain(&plain);
        LinkIngest::Media
    }

    fn push_rtp_plain(&mut self, plain: &[u8]) {
        let mut buf = plain;
        match rtp::header::Header::unmarshal(&mut buf) {
            Ok(hdr) => {
                if !self.jb.push(hdr.sequence_number, buf.to_vec()) {
                    self.counters.late_arrivals.fetch_add(1, Ordering::Relaxed);
                }
                self.counters.record_jb_depth(self.jb.depth());
            }
            Err(_) => {
                self.counters
                    .tamper_or_auth_fail
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Decode one 20 ms frame from this link's jitter buffer into `out`.
    /// Silence while priming, gated, or held.
    pub fn playout_frame(&mut self, out: &mut [i16; FRAME_SAMPLES]) -> Result<usize> {
        if self.gated || self.held {
            out.fill(0);
            return Ok(FRAME_SAMPLES);
        }
        self.counters.record_jb_target_ms(self.jb.target_ms());
        match self.jb.pop_ready() {
            PopOutcome::Packet(pkt) => {
                let n = self.decoder.decode_frame(&pkt.payload, out, false)?;
                self.counters.played.fetch_add(1, Ordering::Relaxed);
                self.counters.record_jb_depth(self.jb.depth());
                Ok(n)
            }
            PopOutcome::Wait => {
                out.fill(0);
                self.counters.record_jb_depth(self.jb.depth());
                Ok(FRAME_SAMPLES)
            }
            PopOutcome::Lost => {
                self.counters.lost_packets.fetch_add(1, Ordering::Relaxed);
                if !self.jb.playout_started() {
                    out.fill(0);
                    self.counters.record_jb_depth(self.jb.depth());
                    return Ok(FRAME_SAMPLES);
                }
                let n = self.decoder.conceal(out)?;
                self.counters.plc_frames.fetch_add(1, Ordering::Relaxed);
                self.counters.record_jb_depth(self.jb.depth());
                Ok(n)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Link construction helpers
// ---------------------------------------------------------------------------

/// Build the SRTP sender/receiver pair for one pairwise link.
///
/// `role` is the handshake role on this link (initiator = [`CallRole::Caller`]).
pub fn mesh_link_srtp(
    output: &HandshakeOutput,
    role: CallRole,
    send: SendFlow,
    recv: ReceiveFlow,
    counters: Arc<MediaCounters>,
) -> Result<(LinkSender, LinkReceiver)> {
    let keys = SrtpKeyPair::derive(output);
    let (out_m, in_m) = keys.for_role(role);
    let sender = LinkSender::new_srtp(out_m, send, counters.clone())?;
    let receiver = LinkReceiver::new_srtp(in_m, recv, counters)?;
    Ok((sender, receiver))
}

/// Build a plain-RTP link (QUIC-protected only). Fails with
/// [`MediaError::MediaDeclined`] unless `explicit_confirm` is true: an
/// unencrypted join always requires an explicit user confirmation.
pub fn mesh_link_plain(
    explicit_confirm: bool,
    send: SendFlow,
    recv: ReceiveFlow,
    counters: Arc<MediaCounters>,
) -> Result<(LinkSender, LinkReceiver)> {
    media_permit(SecurityResolution::NotEncrypted {
        prior_encrypted_history: true,
        user_continued_past_5a: explicit_confirm,
    })?;
    let sender = LinkSender::new_plain(send, counters.clone())?;
    let receiver = LinkReceiver::new_plain(recv, counters)?;
    Ok((sender, receiver))
}

// ---------------------------------------------------------------------------
// MeshMixer
// ---------------------------------------------------------------------------

struct MeshLink {
    peer: PeerId,
    sender: LinkSender,
    receiver: LinkReceiver,
}

/// Encode once, fan out to every armed link, mix every link on playout.
pub struct MeshMixer {
    encoder: MeshEncoder,
    links: Vec<MeshLink>,
}

impl MeshMixer {
    pub fn new() -> Result<Self> {
        Ok(Self {
            encoder: MeshEncoder::new()?,
            links: Vec::with_capacity(MESH_MAX_LINKS),
        })
    }

    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    pub fn peers(&self) -> Vec<PeerId> {
        self.links.iter().map(|l| l.peer).collect()
    }

    pub fn encoder(&self) -> &MeshEncoder {
        &self.encoder
    }

    pub fn set_packet_loss_hint(&mut self, percent: i32) -> Result<()> {
        self.encoder.set_packet_loss_hint(percent)
    }

    /// Add a link. Fails when [`MESH_MAX_LINKS`] links already exist or the
    /// peer is already linked.
    pub fn add_link(
        &mut self,
        peer: PeerId,
        sender: LinkSender,
        receiver: LinkReceiver,
    ) -> Result<()> {
        if self.links.len() >= MESH_MAX_LINKS {
            return Err(MediaError::MeshFull {
                max: MESH_MAX_PARTICIPANTS,
            });
        }
        if self.links.iter().any(|l| l.peer == peer) {
            return Err(MediaError::Failed("mesh peer already linked".into()));
        }
        self.links.push(MeshLink {
            peer,
            sender,
            receiver,
        });
        Ok(())
    }

    /// Remove a link (peer left). Returns true if it existed. Remaining links
    /// are unaffected and keep playing.
    pub fn remove_link(&mut self, peer: &PeerId) -> bool {
        let before = self.links.len();
        self.links.retain(|l| &l.peer != peer);
        self.links.len() != before
    }

    fn link_mut(&mut self, peer: &PeerId) -> Option<&mut MeshLink> {
        self.links.iter_mut().find(|l| &l.peer == peer)
    }

    pub fn sender(&self, peer: &PeerId) -> Option<&LinkSender> {
        self.links
            .iter()
            .find(|l| &l.peer == peer)
            .map(|l| &l.sender)
    }

    pub fn receiver_mut(&mut self, peer: &PeerId) -> Option<&mut LinkReceiver> {
        self.link_mut(peer).map(|l| &mut l.receiver)
    }

    pub fn receiver(&self, peer: &PeerId) -> Option<&LinkReceiver> {
        self.links
            .iter()
            .find(|l| &l.peer == peer)
            .map(|l| &l.receiver)
    }

    /// Put a link into (or out of) the pre-answer state: send disarmed,
    /// receive gated. Returns false if the peer is unknown.
    pub fn set_link_pre_answer(&mut self, peer: &PeerId, pre_answer: bool) -> bool {
        match self.link_mut(peer) {
            Some(l) => {
                l.sender.set_armed(!pre_answer);
                l.receiver.set_gated(pre_answer);
                if pre_answer {
                    l.receiver.flush_pre_answer();
                }
                true
            }
            None => false,
        }
    }

    /// The remote peer answered this link: arm send, open playout, drop any
    /// media buffered before the answer.
    pub fn mark_answered(&mut self, peer: &PeerId) -> bool {
        match self.link_mut(peer) {
            Some(l) => {
                l.receiver.flush_pre_answer();
                l.receiver.set_gated(false);
                l.sender.set_armed(true);
                true
            }
            None => false,
        }
    }

    /// Send the answered control datagram on one link.
    pub fn send_answered_signal(&mut self, peer: &PeerId) -> Result<()> {
        match self.link_mut(peer) {
            Some(l) => l.sender.send_answered_signal(),
            None => Err(MediaError::Failed("unknown mesh peer".into())),
        }
    }

    /// Mute microphone on all links.
    pub fn set_muted(&mut self, muted: bool) {
        for l in &mut self.links {
            l.sender.set_muted(muted);
        }
    }

    /// Mute microphone on one link only. Returns false if the peer is unknown.
    pub fn set_link_muted(&mut self, peer: &PeerId, muted: bool) -> bool {
        match self.link_mut(peer) {
            Some(l) => {
                l.sender.set_muted(muted);
                true
            }
            None => false,
        }
    }

    /// Hold one link: no capture sent, remote audio silent. Returns false if
    /// the peer is unknown.
    pub fn set_link_held(&mut self, peer: &PeerId, held: bool) -> bool {
        match self.link_mut(peer) {
            Some(l) => {
                l.sender.set_held(held);
                l.receiver.set_held(held);
                true
            }
            None => false,
        }
    }

    /// Hold on all links: no capture sent, remote audio silent.
    pub fn set_held(&mut self, held: bool) {
        for l in &mut self.links {
            l.sender.set_held(held);
            l.receiver.set_held(held);
        }
    }

    /// Encode `pcm` once and send it to every link that can send.
    /// Returns the number of links the frame was queued on. Links that are not
    /// armed count a gated drop. When no link can send, Opus is skipped.
    pub fn send_pcm_frame(&mut self, pcm: &mut [i16]) -> Result<usize> {
        if !self.links.iter().any(|l| l.sender.can_send()) {
            for l in &mut self.links {
                if !l.sender.is_armed() {
                    l.sender.gated_drops += 1;
                }
            }
            return Ok(0);
        }
        let rtp = self.encoder.encode_rtp(pcm)?;
        let mut sent = 0usize;
        let mut first_err: Option<MediaError> = None;
        for l in &mut self.links {
            match l.sender.send_rtp(&rtp) {
                Ok(true) => sent += 1,
                Ok(false) => {}
                Err(e) => {
                    // One failed link must not silence the others.
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        match first_err {
            Some(e) if sent == 0 => Err(e),
            _ => Ok(sent),
        }
    }

    /// Ingest a raw datagram for one link. An answered signal arms that link.
    pub fn ingest_link(&mut self, peer: &PeerId, framed: &[u8]) -> Option<LinkIngest> {
        let ingest = self.link_mut(peer)?.receiver.ingest_datagram(framed);
        if ingest == LinkIngest::Answered {
            self.mark_answered(peer);
        }
        Some(ingest)
    }

    /// Await one datagram on one link and ingest it.
    pub async fn recv_link_one(&mut self, peer: &PeerId) -> Result<LinkIngest> {
        let l = self
            .link_mut(peer)
            .ok_or_else(|| MediaError::Failed("unknown mesh peer".into()))?;
        let ingest = l.receiver.recv_one_datagram().await?;
        if ingest == LinkIngest::Answered {
            self.mark_answered(peer);
        }
        Ok(ingest)
    }

    /// Non-blocking drain of every link (at most `max_per_link` datagrams each).
    /// Returns the total number of datagrams ingested.
    pub async fn recv_drain_all(&mut self, max_per_link: u32) -> u32 {
        let mut total = 0;
        let mut answered = Vec::new();
        for l in &mut self.links {
            let (n, ans) = l.receiver.drain(max_per_link).await;
            total += n;
            if ans {
                answered.push(l.peer);
            }
        }
        for p in answered {
            self.mark_answered(&p);
        }
        total
    }

    /// Play out one frame from every link and saturate-add into `out`
    /// (`out` is zeroed first).
    pub fn playout_mixed(&mut self, out: &mut [i16; FRAME_SAMPLES]) -> Result<()> {
        out.fill(0);
        let mut tmp = [0i16; FRAME_SAMPLES];
        for l in &mut self.links {
            l.receiver.playout_frame(&mut tmp)?;
            mix_saturate_add(out, &tmp);
        }
        Ok(())
    }

    /// Play out one frame per link without mixing (per-source analysis).
    pub fn playout_each(&mut self) -> Result<Vec<(PeerId, [i16; FRAME_SAMPLES])>> {
        let mut v = Vec::with_capacity(self.links.len());
        for l in &mut self.links {
            let mut frame = [0i16; FRAME_SAMPLES];
            l.receiver.playout_frame(&mut frame)?;
            v.push((l.peer, frame));
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::SAMPLE_RATE;
    use crate::keys::SrtpKeyPair;
    use zeroize::Zeroizing;

    fn hs(seed: u8) -> HandshakeOutput {
        HandshakeOutput {
            shared_secret: Zeroizing::new(vec![seed; 48]),
            transcript_hash: [seed.wrapping_add(1); 32],
            profile_prk: Zeroizing::new([seed.wrapping_add(2); 32]),
            saga_session_key: Zeroizing::new([seed.wrapping_add(3); 32]),
            confirmation_key: Zeroizing::new([seed.wrapping_add(4); 32]),
        }
    }

    fn tone(freq: f64, amp: f64) -> Vec<i16> {
        (0..FRAME_SAMPLES)
            .map(|n| {
                let t = n as f64 / SAMPLE_RATE as f64;
                ((2.0 * std::f64::consts::PI * freq * t).sin() * amp * 32767.0) as i16
            })
            .collect()
    }

    fn srtp_wire(seed: u8) -> LinkWire {
        let pair = SrtpKeyPair::derive(&hs(seed));
        let (out_m, _) = pair.for_role(CallRole::Caller);
        LinkWire {
            wire: SendWire::Srtp(Box::new(SrtpEncrypt::new(out_m).unwrap())),
        }
    }

    #[test]
    fn encode_once_fans_out_with_different_ciphertext() {
        let mut enc = MeshEncoder::new().unwrap();
        let mut pcm = tone(440.0, 0.4);
        let rtp = enc.encode_rtp(&mut pcm).unwrap();

        let mut a = srtp_wire(10);
        let mut b = srtp_wire(20);
        let fa = a.frame(&rtp).unwrap();
        let fb = b.frame(&rtp).unwrap();

        assert_eq!(fa[0], MEDIA_TAG_SRTP);
        assert_eq!(fb[0], MEDIA_TAG_SRTP);
        assert_eq!(fa.len(), fb.len());
        // Same RTP header (cleartext, same seq/ssrc), different protected payload.
        assert_eq!(&fa[1..13], &fb[1..13]);
        assert_ne!(&fa[13..], &fb[13..]);
    }

    #[test]
    fn same_key_same_rtp_is_deterministic() {
        // Sanity: ciphertext differs because of the key, not randomness.
        let mut enc = MeshEncoder::new().unwrap();
        let mut pcm = tone(440.0, 0.4);
        let rtp = enc.encode_rtp(&mut pcm).unwrap();
        let mut a = srtp_wire(10);
        let mut b = srtp_wire(10);
        assert_eq!(a.frame(&rtp).unwrap(), b.frame(&rtp).unwrap());
    }

    #[test]
    fn encoder_advances_seq_and_timestamp_once_per_frame() {
        let mut enc = MeshEncoder::new().unwrap();
        let mut pcm = tone(440.0, 0.4);
        let r0 = enc.encode_rtp(&mut pcm).unwrap();
        let r1 = enc.encode_rtp(&mut pcm).unwrap();
        let mut b0 = &r0[..];
        let mut b1 = &r1[..];
        let h0 = rtp::header::Header::unmarshal(&mut b0).unwrap();
        let h1 = rtp::header::Header::unmarshal(&mut b1).unwrap();
        assert_eq!(h1.sequence_number, h0.sequence_number.wrapping_add(1));
        assert_eq!(
            h1.timestamp,
            h0.timestamp.wrapping_add(FRAME_SAMPLES as u32)
        );
        assert_eq!(h0.ssrc, h1.ssrc);
    }

    #[test]
    fn saturating_mix_clips_instead_of_wrapping() {
        let mut dst = [i16::MAX - 10, i16::MIN + 10, 100];
        mix_saturate_add(&mut dst, &[100, -100, 200]);
        assert_eq!(dst, [i16::MAX, i16::MIN, 300]);
    }
}
