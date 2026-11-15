//! Host CI mesh session: N in-process endpoints, a real mock handshake per
//! pair, authenticated roster exchange, and [`MeshMixer`] media.
//!
//! Full mesh with pairwise keys (3 peers = 3 links, 4 peers = 6 links). Each
//! pair runs the voice-ALPN handshake with the smaller EndpointId (byte-wise)
//! as initiator, exchanges a roster on a second bi-stream of the same voice
//! connection, then opens RoQ for media. Every peer encodes its tone once and
//! encrypts it per link; every receiver mixes the remote streams locally.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context, Result};
use iroh::endpoint::{presets, Connection};
use iroh::{Endpoint, EndpointAddr, RelayMode};
use iroh_roq::{Session as RoqSession, ALPN as ROQ_ALPN};
use mock_token::{run_initiator, run_responder, HandshakeOutcome, MockIdentity};
use saga_media::{
    mesh_link_plain, mesh_link_srtp, signal_correlation, CallRole, MediaCounters,
    MediaCountersSnapshot, MediaPipeline, MeshMixer, PeerId, FRAME_SAMPLES, MESH_MAX_PARTICIPANTS,
    SAMPLE_RATE,
};
use saga_session::{read_roster, write_roster, HandshakeOutput, Roster, RosterEntry};

use crate::call::SAGA_VOICE_ALPN;
use crate::identity::dev_secret_key;

/// Distinct test tone per participant index (Hz).
pub const MESH_TONES_HZ: [f64; MESH_MAX_PARTICIPANTS] = [440.0, 660.0, 880.0, 1100.0];
/// Per-source amplitude (fraction of full scale). Four sources never clip.
pub const MESH_TONE_AMPLITUDE: f64 = 0.2;
/// Default clip length for CI (~1.2 s).
pub const MESH_DEFAULT_FRAMES: usize = 60;

const RECV_TIMEOUT: Duration = Duration::from_secs(5);
const PRIME_FRAMES: usize = 10;

/// The mesh already holds the maximum number of participants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshFullError {
    pub requested: usize,
    pub max: usize,
}

impl std::fmt::Display for MeshFullError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "mesh call is limited to {} participants (requested {})",
            self.max, self.requested
        )
    }
}

impl std::error::Error for MeshFullError {}

/// Mono 48 kHz sine, `frames` x 20 ms.
pub fn tone_pcm(frames: usize, freq_hz: f64, amplitude: f64) -> Vec<i16> {
    (0..frames * FRAME_SAMPLES)
        .map(|n| {
            let t = n as f64 / SAMPLE_RATE as f64;
            ((2.0 * std::f64::consts::PI * freq_hz * t).sin() * amplitude * 32767.0) as i16
        })
        .collect()
}

/// Normalized Goertzel power at `freq_hz` (a sine of amplitude A gives A^2 / 4).
pub fn tone_power(pcm: &[i16], freq_hz: f64) -> f64 {
    if pcm.is_empty() {
        return 0.0;
    }
    let w = 2.0 * std::f64::consts::PI * freq_hz / SAMPLE_RATE as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in pcm {
        let s0 = x as f64 + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (pcm.len() as f64).powi(2)
}

fn reference_power() -> f64 {
    let a = MESH_TONE_AMPLITUDE * 32767.0;
    a * a / 4.0
}

/// Spectral and waveform check of one receiver's mixed output.
#[derive(Debug, Clone)]
pub struct MixCheck {
    /// Power at each expected source tone, relative to a clean tone (1.0).
    pub present_rel: Vec<f64>,
    /// Power at each tone that must be absent (own tone, departed peers).
    pub absent_rel: Vec<f64>,
    /// Correlation of the mix with the sum of expected source tones.
    pub correlation: f64,
}

impl MixCheck {
    pub const PRESENT_MIN_REL: f64 = 0.2;
    pub const ABSENT_MAX_REL: f64 = 0.02;
    pub const CORR_MIN: f64 = 0.85;

    pub fn verify(&self) -> Result<()> {
        for (i, p) in self.present_rel.iter().enumerate() {
            if *p < Self::PRESENT_MIN_REL {
                bail!("source tone {i} too weak in mix: rel power {p:.3}");
            }
        }
        for (i, p) in self.absent_rel.iter().enumerate() {
            if *p > Self::ABSENT_MAX_REL {
                bail!("tone {i} must be absent from mix: rel power {p:.3}");
            }
        }
        if self.correlation < Self::CORR_MIN {
            bail!(
                "mix correlation {:.3} < {}",
                self.correlation,
                Self::CORR_MIN
            );
        }
        Ok(())
    }
}

/// Analyse `mixed` (starting after `skip_frames` of priming) against the
/// `present` source tones and the `absent` tones.
pub fn check_mix(mixed: &[i16], present: &[f64], absent: &[f64], skip_frames: usize) -> MixCheck {
    let skip = (skip_frames * FRAME_SAMPLES).min(mixed.len());
    let tail = &mixed[skip..];
    let refp = reference_power();
    let present_rel = present
        .iter()
        .map(|f| tone_power(tail, *f) / refp)
        .collect();
    let absent_rel = absent.iter().map(|f| tone_power(tail, *f) / refp).collect();

    let n = tail.len() / FRAME_SAMPLES;
    let mut expected = vec![0i16; tail.len()];
    for f in present {
        let src = tone_pcm(skip_frames + n, *f, MESH_TONE_AMPLITUDE);
        saga_media::mix_saturate_add(&mut expected, &src[skip..skip + tail.len()]);
    }
    let correlation = signal_correlation(&expected, tail, FRAME_SAMPLES * 4);
    MixCheck {
        present_rel,
        absent_rel,
        correlation,
    }
}

/// Session key recorded per link (test visibility only).
pub struct LinkKey {
    pub a: usize,
    pub b: usize,
    pub key: [u8; 32],
}

struct Node {
    #[allow(dead_code)] // keeps the endpoint (and its UDP socket) alive
    ep: Endpoint,
    mixer: MeshMixer,
    counters: Vec<Arc<MediaCounters>>,
    /// RoQ sessions and voice connections must outlive the media loop.
    _sessions: Vec<RoqSession>,
    _conns: Vec<Connection>,
}

/// Established in-process mesh.
pub struct HostMesh {
    pub labels: Vec<String>,
    pub ids: Vec<PeerId>,
    pub encrypted: bool,
    pub link_keys: Vec<LinkKey>,
    /// Number of authenticated rosters received (2 per encrypted link).
    pub rosters_verified: usize,
    nodes: Vec<Option<Node>>,
}

struct SideSetup {
    hs: Option<HandshakeOutput>,
    voice: Connection,
    roq: Connection,
    roster: Option<Roster>,
}

fn fingerprint_bytes(label: &str) -> Result<[u8; 32]> {
    let hex = MockIdentity::from_label(label).fingerprint_hex_string();
    if hex.len() != 64 {
        bail!("fingerprint for {label} is not 32 bytes");
    }
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).context("fingerprint hex")?;
    }
    Ok(out)
}

async fn bind(label: &str) -> Result<Endpoint> {
    Ok(Endpoint::builder(presets::Minimal)
        .alpns(vec![SAGA_VOICE_ALPN.to_vec(), ROQ_ALPN.to_vec()])
        .secret_key(dev_secret_key(label))
        .relay_mode(RelayMode::Disabled)
        .bind()
        .await?)
}

/// Establish one pairwise link; `init` is the initiator (smaller EndpointId).
async fn establish_link(
    ep_init: &Endpoint,
    ep_resp: &Endpoint,
    label_init: &str,
    label_resp: &str,
    encrypted: bool,
    roster: Roster,
) -> Result<(SideSetup, SideSetup)> {
    let resp_task = tokio::spawn({
        let ep = ep_resp.clone();
        let label = label_resp.to_string();
        let roster = roster.clone();
        async move {
            let voice = ep.accept().await.context("voice accept")?.await?;
            if voice.alpn() != SAGA_VOICE_ALPN {
                bail!("expected voice ALPN");
            }
            let identity = MockIdentity::from_label(&label);
            let local = *ep.id().as_bytes();
            let remote = *voice.remote_id().as_bytes();
            let (mut send, mut recv) = voice.accept_bi().await?;
            let outcome =
                run_responder(&identity, &local, &remote, !encrypted, &mut recv, &mut send).await;
            let hs = match outcome {
                HandshakeOutcome::Encrypted { output } => Some(output),
                HandshakeOutcome::Failed { .. } => None,
            };
            let mut got = None;
            if let Some(ref hs) = hs {
                let (mut s2, mut r2) = voice.accept_bi().await?;
                got = Some(read_roster(&mut r2, hs).await?);
                write_roster(&mut s2, hs, &roster).await?;
                let _ = s2.finish();
            }
            let remote_id = voice.remote_id();
            let roq = loop {
                let c = ep.accept().await.context("roq accept")?.await?;
                if c.alpn() == ROQ_ALPN && c.remote_id() == remote_id {
                    break c;
                }
            };
            Ok::<_, anyhow::Error>(SideSetup {
                hs,
                voice,
                roq,
                roster: got,
            })
        }
    });

    let voice = ep_init
        .connect(ep_resp.addr(), SAGA_VOICE_ALPN)
        .await
        .context("voice connect")?;
    let identity = MockIdentity::from_label(label_init);
    let local = *ep_init.id().as_bytes();
    let remote = *voice.remote_id().as_bytes();
    let (mut send, mut recv) = voice.open_bi().await?;
    let outcome = run_initiator(
        &identity, label_resp, &local, &remote, !encrypted, &mut recv, &mut send,
    )
    .await;
    let _ = send.finish();
    let hs = match outcome {
        HandshakeOutcome::Encrypted { output } => Some(output),
        HandshakeOutcome::Failed { .. } => None,
    };
    if encrypted && hs.is_none() {
        bail!("handshake failed for {label_init} -> {label_resp}");
    }
    let mut got = None;
    if let Some(ref hs) = hs {
        let (mut s2, mut r2) = voice.open_bi().await?;
        write_roster(&mut s2, hs, &roster).await?;
        got = Some(read_roster(&mut r2, hs).await?);
        let _ = s2.finish();
    }
    let roq = ep_init
        .connect(EndpointAddr::from(voice.remote_id()), ROQ_ALPN)
        .await
        .context("roq connect")?;
    let resp = resp_task.await??;
    Ok((
        SideSetup {
            hs,
            voice,
            roq,
            roster: got,
        },
        resp,
    ))
}

impl HostMesh {
    /// Bind endpoints, run every pairwise handshake, exchange rosters, and
    /// build one [`MeshMixer`] per node.
    pub async fn establish(labels: &[&str], encrypted: bool) -> Result<Self> {
        if labels.len() > MESH_MAX_PARTICIPANTS {
            return Err(MeshFullError {
                requested: labels.len(),
                max: MESH_MAX_PARTICIPANTS,
            }
            .into());
        }
        if labels.len() < 2 {
            bail!("a mesh call needs at least 2 participants");
        }
        for (i, l) in labels.iter().enumerate() {
            if labels[..i].contains(l) {
                bail!("duplicate participant label {l}");
            }
        }

        let mut eps = Vec::new();
        for l in labels {
            eps.push(bind(l).await?);
        }
        let ids: Vec<PeerId> = eps.iter().map(|e| *e.id().as_bytes()).collect();

        let mut entries = Vec::new();
        for (l, id) in labels.iter().zip(ids.iter()) {
            entries.push(RosterEntry {
                endpoint_id: *id,
                fingerprint: fingerprint_bytes(l)?,
            });
        }
        let roster = Roster::new(entries, None)?;

        let n = labels.len();
        let mut mixers: Vec<MeshMixer> =
            (0..n).map(|_| MeshMixer::new()).collect::<Result<_, _>>()?;
        let mut counters: Vec<Vec<Arc<MediaCounters>>> = vec![Vec::new(); n];
        let mut sessions: Vec<Vec<RoqSession>> = (0..n).map(|_| Vec::new()).collect();
        let mut conns: Vec<Vec<Connection>> = (0..n).map(|_| Vec::new()).collect();
        let mut link_keys = Vec::new();
        let mut rosters_verified = 0usize;
        let flow = MediaPipeline::flow_id_hint();

        for i in 0..n {
            for j in (i + 1)..n {
                // Deterministic initiator: smaller EndpointId, byte-wise.
                let (a, b) = if ids[i] <= ids[j] { (i, j) } else { (j, i) };
                let (init, resp) = establish_link(
                    &eps[a],
                    &eps[b],
                    labels[a],
                    labels[b],
                    encrypted,
                    roster.clone(),
                )
                .await
                .with_context(|| format!("link {} <-> {}", labels[a], labels[b]))?;

                if encrypted {
                    let ha = init.hs.as_ref().context("initiator hs")?;
                    let hb = resp.hs.as_ref().context("responder hs")?;
                    if ha.saga_session_key.as_slice() != hb.saga_session_key.as_slice() {
                        bail!(
                            "session keys differ on link {} <-> {}",
                            labels[a],
                            labels[b]
                        );
                    }
                    let mut key = [0u8; 32];
                    key.copy_from_slice(ha.saga_session_key.as_slice());
                    if link_keys.iter().any(|k: &LinkKey| k.key == key) {
                        bail!("session key reused across links");
                    }
                    link_keys.push(LinkKey { a, b, key });
                    for side in [&init, &resp] {
                        let r = side.roster.as_ref().context("roster missing")?;
                        if r != &roster {
                            bail!("roster mismatch on link {} <-> {}", labels[a], labels[b]);
                        }
                        rosters_verified += 1;
                    }
                }

                let sess_a = RoqSession::new(init.roq.clone());
                let sess_b = RoqSession::new(resp.roq.clone());
                let tx_a = sess_a.new_send_flow(flow).await?;
                let rx_a = sess_a.new_receive_flow(flow).await?;
                let tx_b = sess_b.new_send_flow(flow).await?;
                let rx_b = sess_b.new_receive_flow(flow).await?;
                let ca = Arc::new(MediaCounters::default());
                let cb = Arc::new(MediaCounters::default());
                let (link_a, link_b) = if encrypted {
                    (
                        mesh_link_srtp(
                            init.hs.as_ref().unwrap(),
                            CallRole::Caller,
                            tx_a,
                            rx_a,
                            ca.clone(),
                        )?,
                        mesh_link_srtp(
                            resp.hs.as_ref().unwrap(),
                            CallRole::Callee,
                            tx_b,
                            rx_b,
                            cb.clone(),
                        )?,
                    )
                } else {
                    // Explicit confirm is the test stand-in for the user's tap.
                    (
                        mesh_link_plain(true, tx_a, rx_a, ca.clone())?,
                        mesh_link_plain(true, tx_b, rx_b, cb.clone())?,
                    )
                };
                mixers[a].add_link(ids[b], link_a.0, link_a.1)?;
                mixers[b].add_link(ids[a], link_b.0, link_b.1)?;
                counters[a].push(ca);
                counters[b].push(cb);
                sessions[a].push(sess_a);
                sessions[b].push(sess_b);
                conns[a].extend([init.voice, init.roq]);
                conns[b].extend([resp.voice, resp.roq]);
            }
        }

        let nodes = eps
            .into_iter()
            .zip(mixers)
            .zip(counters)
            .zip(sessions)
            .zip(conns)
            .map(|((((ep, mixer), counters), s), c)| {
                Some(Node {
                    ep,
                    mixer,
                    counters,
                    _sessions: s,
                    _conns: c,
                })
            })
            .collect();

        Ok(Self {
            labels: labels.iter().map(|s| s.to_string()).collect(),
            ids,
            encrypted,
            link_keys,
            rosters_verified,
            nodes,
        })
    }

    pub fn is_live(&self, idx: usize) -> bool {
        self.nodes.get(idx).is_some_and(|n| n.is_some())
    }

    pub fn live_indices(&self) -> Vec<usize> {
        (0..self.nodes.len()).filter(|i| self.is_live(*i)).collect()
    }

    pub fn tone_hz(&self, idx: usize) -> f64 {
        MESH_TONES_HZ[idx]
    }

    /// Aggregate counters over all links of one node.
    pub fn counters(&self, idx: usize) -> Option<MediaCountersSnapshot> {
        let node = self.nodes.get(idx)?.as_ref()?;
        let mut sum = MediaCountersSnapshot::default();
        for c in &node.counters {
            let s = c.snapshot();
            sum.sent += s.sent;
            sum.played += s.played;
            sum.tamper_or_auth_fail += s.tamper_or_auth_fail;
            sum.replay += s.replay;
            sum.wrong_media_mode += s.wrong_media_mode;
            sum.lost_packets += s.lost_packets;
            sum.plc_frames += s.plc_frames;
        }
        Some(sum)
    }

    pub fn link_count(&self, idx: usize) -> usize {
        self.nodes
            .get(idx)
            .and_then(|n| n.as_ref())
            .map_or(0, |n| n.mixer.link_count())
    }

    /// A participant leaves: its endpoint and sessions are dropped and every
    /// remaining node removes the link. The call continues for the others.
    pub fn leave(&mut self, idx: usize) -> Result<()> {
        if !self.is_live(idx) {
            bail!("participant {idx} is not in the call");
        }
        self.nodes[idx] = None;
        let gone = self.ids[idx];
        for node in self.nodes.iter_mut().flatten() {
            node.mixer.remove_link(&gone);
        }
        Ok(())
    }

    /// Run `frames` ticks. Every live peer encodes its tone once and sends it
    /// on every link; every live peer drains one datagram per link and plays
    /// one mixed frame. Returns the mixed playout per node (empty if left).
    pub async fn run(&mut self, frames: usize) -> Result<Vec<Vec<i16>>> {
        let n = self.nodes.len();
        let sources: Vec<Vec<i16>> = (0..n)
            .map(|i| tone_pcm(frames, MESH_TONES_HZ[i], MESH_TONE_AMPLITUDE))
            .collect();
        let mut mixed: Vec<Vec<i16>> = vec![Vec::new(); n];
        for f in 0..frames {
            for (node, src) in self.nodes.iter_mut().zip(sources.iter()) {
                if let Some(node) = node.as_mut() {
                    let mut frame = src[f * FRAME_SAMPLES..(f + 1) * FRAME_SAMPLES].to_vec();
                    node.mixer.send_pcm_frame(&mut frame)?;
                }
            }
            for (node, out_buf) in self.nodes.iter_mut().zip(mixed.iter_mut()) {
                if let Some(node) = node.as_mut() {
                    for peer in node.mixer.peers() {
                        tokio::time::timeout(RECV_TIMEOUT, node.mixer.recv_link_one(&peer))
                            .await
                            .context("mesh recv timeout")??;
                    }
                    let mut out = [0i16; FRAME_SAMPLES];
                    node.mixer.playout_mixed(&mut out)?;
                    out_buf.extend_from_slice(&out);
                }
            }
        }
        Ok(mixed)
    }

    /// Check one node's mix: all live others present, own and departed absent.
    pub fn check_node_mix(&self, idx: usize, mixed: &[i16], skip_frames: usize) -> MixCheck {
        let mut present = Vec::new();
        let mut absent = Vec::new();
        for (j, hz) in MESH_TONES_HZ.iter().enumerate().take(self.nodes.len()) {
            if j == idx || !self.is_live(j) {
                absent.push(*hz);
            } else {
                present.push(*hz);
            }
        }
        check_mix(mixed, &present, &absent, skip_frames)
    }
}

/// Result of [`host_mesh_roundtrip`].
pub struct MeshOutcome {
    pub labels: Vec<String>,
    pub links: usize,
    pub link_keys: Vec<LinkKey>,
    pub rosters_verified: usize,
    /// Mixed playout per node.
    pub mixed: Vec<Vec<i16>>,
    pub checks: Vec<MixCheck>,
    pub counters: Vec<MediaCountersSnapshot>,
}

/// In-process host test: full mesh over `labels` (max 4), each peer sends a
/// distinct tone, every receiver mixes the others and the mix is verified.
pub async fn host_mesh_roundtrip(labels: &[&str], encrypted: bool) -> Result<MeshOutcome> {
    let mut mesh = HostMesh::establish(labels, encrypted).await?;
    let mixed = mesh.run(MESH_DEFAULT_FRAMES).await?;
    let mut checks = Vec::new();
    let mut counters = Vec::new();
    for (i, m) in mixed.iter().enumerate() {
        let check = mesh.check_node_mix(i, m, PRIME_FRAMES);
        check
            .verify()
            .with_context(|| format!("mix at {}", mesh.labels[i]))?;
        checks.push(check);
        counters.push(mesh.counters(i).context("counters")?);
    }
    let n = labels.len();
    Ok(MeshOutcome {
        labels: mesh.labels.clone(),
        links: n * (n - 1) / 2,
        link_keys: std::mem::take(&mut mesh.link_keys),
        rosters_verified: mesh.rosters_verified,
        mixed,
        checks,
        counters,
    })
}
