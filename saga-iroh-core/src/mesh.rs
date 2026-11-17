//! Native mesh call (Android JNI + host tests): full mesh of pairwise links, up
//! to [`MESH_MAX_PARTICIPANTS`] participants.
//!
//! Each link is one QUIC connection on the dedicated `saga/mesh-join/1` ALPN
//! (never routed into the Telecom ring path) plus one RoQ connection for media:
//!
//! 1. bi-stream 1: `hello` (group id) + mock-token handshake (initiator dials);
//! 2. bi-stream 2: authenticated roster exchange (`saga-mesh-roster-v1` HMAC);
//! 3. RoQ connection (initiator connects, responder is woken through
//!    `pending_roq`), then one SRTP `LinkSender`/`LinkReceiver` pair is added to
//!    the group's single `MeshMixer` (encode once, encrypt per link).
//!
//! When A (talking to B) adds C: A dials C, the rosters tell C about B and B
//! (through an update roster pushed on the A-B link) about C, and the peer with
//! the smaller EndpointId dials the other directly. Leaving closes the local
//! links only; remaining peers keep their mutual links (no host).
//!
//! Media is keyed by id: a group id addresses the group mixer directly; a link
//! id resolves to its group (the oldest attached link is the group's audio
//! leader and is the only link id whose capture/playback is serviced).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use iroh::endpoint::Connection;
use iroh::{Endpoint, EndpointAddr, PublicKey};
use iroh_roq::{Session as RoqSession, ALPN as ROQ_ALPN};
use log::{info, warn};
use mock_token::{
    run_initiator_with_options, run_responder_with_options, HandshakeOutcome, MockIdentity,
};
use saga_media::{
    mesh_link_srtp, CallRole, MediaCounters, MediaPipeline, MeshMixer, PeerId, FRAME_SAMPLES,
    MESH_MAX_LINKS, MESH_MAX_PARTICIPANTS,
};
use saga_session::{read_roster, write_roster, HandshakeOutput, Roster, RosterEntry};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Handle;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};

use crate::iroh_transport::{
    dev_label_for_endpoint, dev_secret_key, dial_with_retry, endpoint_addr_for_peer,
    force_binding_perturb, force_handshake_fail, parse_peer_endpoint_id, sample_relay_connected,
    RELAY_ONLINE_WAIT, SAGA_MESH_JOIN_ALPN,
};

/// Link state values reported through `notifyMeshPeerState` (match Kotlin).
pub const MESH_PEER_PENDING: i32 = 0;
pub const MESH_PEER_ENCRYPTED: i32 = 1;
pub const MESH_PEER_FAILED: i32 = 2;
pub const MESH_PEER_LEFT: i32 = 3;

const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
const RING_FRAMES: usize = 50;
const MAX_GROUP_ID_LEN: usize = 128;
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);
const ROQ_WAIT: Duration = Duration::from_secs(15);
const INBOUND_AUTH_WAIT: Duration = Duration::from_secs(5);

/// Labels whose token identity fingerprint is known to this build (dev/E2E).
const FINGERPRINT_LABELS: [&str; 11] = [
    "15550100010",
    "15550100011",
    "15550100020",
    "15550100030",
    "alicepeer1",
    "bobpeer12",
    "vector-alice",
    "vector-bob",
    "neg-alice",
    "neg-bob",
    "neg-impostor",
];

type NotifyFn = Arc<dyn Fn(&str, &str, i32) + Send + Sync>;

/// True when the local node must dial `remote` for a roster-introduced link
/// (the smaller EndpointId, byte-wise, is the initiator).
pub fn local_dials(local: &PublicKey, remote: &PublicKey) -> bool {
    local.as_bytes() < remote.as_bytes()
}

fn identity_fingerprint(label: &str) -> [u8; 32] {
    let hex = MockIdentity::from_label(label).fingerprint_hex_string();
    let mut out = [0u8; 32];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap_or(0);
    }
    out
}

fn fingerprint_hex(fp: &[u8; 32]) -> String {
    fp.iter().map(|b| format!("{b:02x}")).collect()
}

fn label_for_endpoint(pk: &PublicKey) -> Option<String> {
    FINGERPRINT_LABELS
        .iter()
        .find(|l| dev_secret_key(l).public() == *pk)
        .map(|l| l.to_string())
}

/// Stable per-peer key used in link ids (matches Kotlin `GroupCallState.legKey`).
fn lookup_for_endpoint(pk: &PublicKey) -> String {
    dev_label_for_endpoint(pk).unwrap_or_else(|| pk.to_string())
}

fn link_id_for(group_id: &str, pk: &PublicKey) -> String {
    format!("{group_id}/{}", lookup_for_endpoint(pk))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LinkState {
    Pending,
    Encrypted,
}

/// Lock-free per-link control surface (JNI threads write, the media tick reads).
#[derive(Default)]
struct LinkFlags {
    muted: AtomicBool,
    held: AtomicBool,
    answer_pending: AtomicBool,
    /// Link is in the group mixer (send armed).
    attached: AtomicBool,
}

struct LinkInfo {
    link_id: String,
    peer: PublicKey,
    state: LinkState,
    fingerprint: Option<[u8; 32]>,
    hs: Option<Arc<HandshakeOutput>>,
    voice: Option<Connection>,
    roq: Option<Connection>,
    /// Keeps the RoQ session (and its flows) alive for media.
    roq_session: Option<RoqSession>,
    flags: Arc<LinkFlags>,
    task: Option<tokio::task::AbortHandle>,
}

struct GroupMedia {
    capture_tx: mpsc::Sender<Vec<i16>>,
    ring: Arc<StdMutex<VecDeque<Vec<u8>>>>,
    _shutdown: mpsc::Sender<()>,
}

struct MeshGroup {
    id: String,
    links: StdMutex<Vec<LinkInfo>>,
    mixer: TokioMutex<MeshMixer>,
    /// Peers announced by a verified roster, with the attested fingerprint.
    expected: StdMutex<HashMap<PublicKey, [u8; 32]>>,
    /// Mixer links to drop on the next media tick (or before the next attach).
    removals: StdMutex<Vec<PeerId>>,
    media: StdMutex<Option<GroupMedia>>,
    closed: AtomicBool,
}

impl MeshGroup {
    fn new(id: &str) -> Result<Self, String> {
        Ok(Self {
            id: id.to_string(),
            links: StdMutex::new(Vec::new()),
            mixer: TokioMutex::new(MeshMixer::new().map_err(|e| format!("mixer: {e}"))?),
            expected: StdMutex::new(HashMap::new()),
            removals: StdMutex::new(Vec::new()),
            media: StdMutex::new(None),
            closed: AtomicBool::new(false),
        })
    }

    fn attached_flags(&self) -> Vec<(PeerId, Arc<LinkFlags>)> {
        let Ok(links) = self.links.lock() else {
            return Vec::new();
        };
        links
            .iter()
            .filter(|l| l.flags.attached.load(Ordering::Relaxed))
            .map(|l| (*l.peer.as_bytes(), l.flags.clone()))
            .collect()
    }

    /// Oldest attached link: the only link id whose audio I/O is serviced.
    fn leader_link_id(&self) -> Option<String> {
        let links = self.links.lock().ok()?;
        links
            .iter()
            .find(|l| l.flags.attached.load(Ordering::Relaxed))
            .map(|l| l.link_id.clone())
    }

    async fn apply_removals(&self, mixer: &mut MeshMixer) {
        let peers: Vec<PeerId> = match self.removals.lock() {
            Ok(mut r) => std::mem::take(&mut *r),
            Err(_) => return,
        };
        for p in peers {
            mixer.remove_link(&p);
        }
    }

    fn close_all_links(&self) {
        let links: Vec<LinkInfo> = match self.links.lock() {
            Ok(mut l) => std::mem::take(&mut *l),
            Err(_) => return,
        };
        for l in links {
            close_link_info(&l);
            if let Ok(mut r) = self.removals.lock() {
                r.push(*l.peer.as_bytes());
            }
        }
    }
}

fn close_link_info(l: &LinkInfo) {
    l.flags.attached.store(false, Ordering::Relaxed);
    if let Some(t) = &l.task {
        t.abort();
    }
    if let Some(c) = &l.roq {
        c.close(0u32.into(), b"saga mesh leave");
    }
    if let Some(c) = &l.voice {
        c.close(0u32.into(), b"saga mesh leave");
    }
}

/// One local mesh participant (process-global on Android, per-instance in tests).
pub struct MeshNode {
    endpoint: Endpoint,
    label: String,
    local_id: PublicKey,
    own_fp: [u8; 32],
    rt: Handle,
    /// Wait for the relay before dialing (production); off for direct-address tests.
    wait_online: bool,
    notify: NotifyFn,
    groups: StdMutex<HashMap<String, Arc<MeshGroup>>>,
    pending_roq: StdMutex<HashMap<PublicKey, oneshot::Sender<Connection>>>,
    /// Direct-address overrides (host tests).
    addr_book: StdMutex<HashMap<PublicKey, EndpointAddr>>,
}

static GLOBAL: OnceLock<Arc<MeshNode>> = OnceLock::new();

/// Process-wide node on the shared endpoint. Never binds; `None` until the
/// endpoint exists (see `ensure_global_node`).
pub(crate) fn global_node() -> Option<Arc<MeshNode>> {
    if let Some(n) = GLOBAL.get() {
        return Some(n.clone());
    }
    let endpoint = crate::iroh_transport::shared_endpoint()?.clone();
    let node = MeshNode::new(
        endpoint,
        crate::iroh_transport::local_identity_label(),
        crate::iroh_transport::runtime().handle().clone(),
        true,
        Arc::new(crate::bridge_notify_mesh_peer_state),
    );
    let _ = GLOBAL.set(node);
    GLOBAL.get().cloned()
}

/// Bind the shared endpoint if needed (JNI threads only; blocks on the runtime).
pub(crate) fn ensure_global_node() -> Result<Arc<MeshNode>, String> {
    crate::iroh_transport::ensure_listening()?;
    global_node().ok_or_else(|| "mesh node unavailable".to_string())
}

impl MeshNode {
    pub fn new(
        endpoint: Endpoint,
        label: String,
        rt: Handle,
        wait_online: bool,
        notify: NotifyFn,
    ) -> Arc<Self> {
        let local_id = endpoint.id();
        let own_fp = identity_fingerprint(&label);
        Arc::new(Self {
            endpoint,
            label,
            local_id,
            own_fp,
            rt,
            wait_online,
            notify,
            groups: StdMutex::new(HashMap::new()),
            pending_roq: StdMutex::new(HashMap::new()),
            addr_book: StdMutex::new(HashMap::new()),
        })
    }

    #[cfg(test)]
    pub fn local_id(&self) -> PublicKey {
        self.local_id
    }

    #[cfg(test)]
    /// Register a direct address for `peer` (tests; production uses relay hints).
    pub fn set_peer_addr(&self, peer: PublicKey, addr: EndpointAddr) {
        if let Ok(mut b) = self.addr_book.lock() {
            b.insert(peer, addr);
        }
    }

    fn emit(&self, group_id: &str, link_id: &str, state: i32) {
        (self.notify)(group_id, link_id, state);
    }

    fn group(&self, id: &str) -> Option<Arc<MeshGroup>> {
        self.groups.lock().ok()?.get(id).cloned()
    }

    fn group_or_create(&self, id: &str) -> Result<Arc<MeshGroup>, String> {
        if id.is_empty() || id.len() > MAX_GROUP_ID_LEN {
            return Err("invalid group id".into());
        }
        let mut groups = self.groups.lock().map_err(|_| "groups poisoned")?;
        if let Some(g) = groups.get(id) {
            return Ok(g.clone());
        }
        let g = Arc::new(MeshGroup::new(id)?);
        groups.insert(id.to_string(), g.clone());
        Ok(g)
    }

    /// `nativeMeshOpen`: create the group (idempotent).
    pub fn open(&self, group_id: &str) -> bool {
        match self.group_or_create(group_id) {
            Ok(_) => {
                info!("[Saga Mesh] open group=[{group_id}]");
                true
            }
            Err(e) => {
                warn!("[Saga Mesh] open group=[{group_id}] failed: {e}");
                false
            }
        }
    }

    /// `nativeMeshAddPeer`: start dialing `peer_id` (the caller is the initiator).
    pub fn add_peer(self: &Arc<Self>, group_id: &str, link_id: &str, peer_id: &str) -> bool {
        let remote = match parse_peer_endpoint_id(peer_id) {
            Ok(pk) => pk,
            Err(e) => {
                warn!("[Saga Mesh] add_peer bad peer [{peer_id}]: {e}");
                return false;
            }
        };
        if remote == self.local_id {
            warn!("[Saga Mesh] add_peer refuses self");
            return false;
        }
        let group = match self.group_or_create(group_id) {
            Ok(g) => g,
            Err(e) => {
                warn!("[Saga Mesh] add_peer group error: {e}");
                return false;
            }
        };
        let flags = match self.insert_link(&group, link_id, remote) {
            Ok(f) => f,
            Err(e) => {
                warn!("[Saga Mesh] add_peer refused group=[{group_id}] link=[{link_id}]: {e}");
                return false;
            }
        };
        let peer_label = label_for_endpoint(&remote).unwrap_or_else(|| peer_id.to_string());
        self.spawn_outbound(&group, link_id, remote, peer_label, flags);
        true
    }

    fn spawn_outbound(
        self: &Arc<Self>,
        group: &Arc<MeshGroup>,
        link_id: &str,
        remote: PublicKey,
        peer_label: String,
        flags: Arc<LinkFlags>,
    ) {
        let node = self.clone();
        let g = group.clone();
        let lid = link_id.to_string();
        let handle = self.rt.spawn(async move {
            node.emit(&g.id, &lid, MESH_PEER_PENDING);
            if let Err(e) = node.outbound(&g, &lid, remote, &peer_label, flags).await {
                warn!(
                    "[Saga Mesh] outbound link failed group=[{}] link=[{lid}]: {e}",
                    g.id
                );
                node.fail_link(&g, &lid);
            }
        });
        if let Ok(mut links) = group.links.lock() {
            if let Some(l) = links.iter_mut().find(|l| l.link_id == link_id) {
                l.task = Some(handle.abort_handle());
            }
        }
    }

    /// Reserve a link slot: enforces the participant cap and one link per peer.
    fn insert_link(
        &self,
        group: &Arc<MeshGroup>,
        link_id: &str,
        peer: PublicKey,
    ) -> Result<Arc<LinkFlags>, String> {
        let mut links = group
            .links
            .lock()
            .map_err(|_| "links poisoned".to_string())?;
        if group.closed.load(Ordering::Relaxed) {
            return Err("group closed".into());
        }
        if links.iter().any(|l| l.peer == peer) {
            return Err("peer already linked".into());
        }
        if links.iter().any(|l| l.link_id == link_id) {
            return Err("link id in use".into());
        }
        if links.len() >= MESH_MAX_LINKS {
            return Err(format!(
                "mesh limited to {MESH_MAX_PARTICIPANTS} participants"
            ));
        }
        let flags = Arc::new(LinkFlags::default());
        links.push(LinkInfo {
            link_id: link_id.to_string(),
            peer,
            state: LinkState::Pending,
            fingerprint: None,
            hs: None,
            voice: None,
            roq: None,
            roq_session: None,
            flags: flags.clone(),
            task: None,
        });
        Ok(flags)
    }

    /// Drop a link that never became (or stopped being) usable and report FAILED.
    fn fail_link(&self, group: &Arc<MeshGroup>, link_id: &str) {
        let removed = {
            let Ok(mut links) = group.links.lock() else {
                return;
            };
            links
                .iter()
                .position(|l| l.link_id == link_id)
                .map(|i| links.remove(i))
        };
        if let Some(l) = removed {
            close_link_info(&l);
            if let Ok(mut r) = group.removals.lock() {
                r.push(*l.peer.as_bytes());
            }
            self.emit(&group.id, link_id, MESH_PEER_FAILED);
        }
    }

    /// `nativeMeshRemovePeer`: close one link only. No callback (caller-initiated).
    pub fn remove_peer(&self, group_id: &str, link_id: &str) {
        let Some(group) = self.group(group_id) else {
            return;
        };
        let removed = {
            let Ok(mut links) = group.links.lock() else {
                return;
            };
            links
                .iter()
                .position(|l| l.link_id == link_id)
                .map(|i| links.remove(i))
        };
        if let Some(l) = removed {
            close_link_info(&l);
            if let Ok(mut r) = group.removals.lock() {
                r.push(*l.peer.as_bytes());
            }
            info!("[Saga Mesh] remove_peer group=[{group_id}] link=[{link_id}]");
        }
    }

    /// `nativeMeshClose`: the local participant leaves. Only local links close;
    /// the remaining peers keep their mutual links.
    pub fn close(&self, group_id: &str) {
        let group = self.groups.lock().ok().and_then(|mut g| g.remove(group_id));
        let Some(group) = group else {
            return;
        };
        group.closed.store(true, Ordering::Relaxed);
        group.close_all_links();
        if let Ok(mut m) = group.media.lock() {
            *m = None;
        }
        info!("[Saga Mesh] close group=[{group_id}] (local links only)");
    }

    /// `nativeMeshPeerFingerprint`: 64 lowercase hex, only after an encrypted handshake.
    pub fn peer_fingerprint(&self, group_id: &str, link_id: &str) -> Option<String> {
        let group = self.group(group_id)?;
        let links = group.links.lock().ok()?;
        let l = links.iter().find(|l| l.link_id == link_id)?;
        if l.state != LinkState::Encrypted {
            return None;
        }
        l.fingerprint.as_ref().map(fingerprint_hex)
    }

    #[cfg(test)]
    /// Encrypted links in a group (diagnostics / tests).
    pub fn encrypted_peers(&self, group_id: &str) -> Vec<PublicKey> {
        let Some(group) = self.group(group_id) else {
            return Vec::new();
        };
        let Ok(links) = group.links.lock() else {
            return Vec::new();
        };
        links
            .iter()
            .filter(|l| l.state == LinkState::Encrypted)
            .map(|l| l.peer)
            .collect()
    }

    #[cfg(test)]
    /// Link ids currently in the group (any state).
    pub fn link_ids(&self, group_id: &str) -> Vec<String> {
        let Some(group) = self.group(group_id) else {
            return Vec::new();
        };
        let Ok(links) = group.links.lock() else {
            return Vec::new();
        };
        links.iter().map(|l| l.link_id.clone()).collect()
    }

    /// Hand a RoQ connection to the link waiting for it. `Err` returns it unchanged.
    pub fn offer_roq(&self, conn: Connection) -> Result<(), Connection> {
        let remote = conn.remote_id();
        let tx = self
            .pending_roq
            .lock()
            .ok()
            .and_then(|mut p| p.remove(&remote));
        match tx {
            Some(tx) => tx.send(conn),
            None => Err(conn),
        }
    }

    // ----- roster -------------------------------------------------------

    fn build_roster(
        &self,
        group: &MeshGroup,
        extra: Option<([u8; 32], [u8; 32])>,
    ) -> Result<Roster, String> {
        let mut entries = vec![RosterEntry {
            endpoint_id: *self.local_id.as_bytes(),
            fingerprint: self.own_fp,
        }];
        if let Ok(links) = group.links.lock() {
            for l in links.iter().filter(|l| l.state == LinkState::Encrypted) {
                if let Some(fp) = l.fingerprint {
                    entries.push(RosterEntry {
                        endpoint_id: *l.peer.as_bytes(),
                        fingerprint: fp,
                    });
                }
            }
        }
        if let Some((id, fp)) = extra {
            if !entries.iter().any(|e| e.endpoint_id == id) {
                entries.push(RosterEntry {
                    endpoint_id: id,
                    fingerprint: fp,
                });
            }
        }
        Roster::new(entries, None).map_err(|e| format!("roster: {e}"))
    }

    /// The sender must be consistent with this link, and our own entry must be ours.
    fn validate_roster(
        &self,
        roster: &Roster,
        from: &PublicKey,
        from_fp: Option<[u8; 32]>,
    ) -> Result<(), String> {
        for p in &roster.participants {
            if p.endpoint_id == *self.local_id.as_bytes() && p.fingerprint != self.own_fp {
                return Err("roster lists a wrong fingerprint for the local peer".into());
            }
            if p.endpoint_id == *from.as_bytes() {
                if let Some(fp) = from_fp {
                    if p.fingerprint != fp {
                        return Err("roster sender fingerprint differs from link handshake".into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Learn about peers from a verified roster; dial those we must initiate to.
    fn apply_roster(self: &Arc<Self>, group: &Arc<MeshGroup>, from: &PublicKey, roster: &Roster) {
        for p in &roster.participants {
            if p.endpoint_id == *self.local_id.as_bytes() || p.endpoint_id == *from.as_bytes() {
                continue;
            }
            let Ok(pk) = PublicKey::from_bytes(&p.endpoint_id) else {
                continue;
            };
            if let Ok(mut exp) = group.expected.lock() {
                exp.entry(pk).or_insert(p.fingerprint);
            }
            if !local_dials(&self.local_id, &pk) {
                // The other side initiates; its inbound is authorized via `expected`.
                continue;
            }
            let link_id = link_id_for(&group.id, &pk);
            match self.insert_link(group, &link_id, pk) {
                Ok(flags) => {
                    info!(
                        "[Saga Mesh] roster introduces peer, dialing group=[{}] link=[{link_id}]",
                        group.id
                    );
                    let peer_label = label_for_endpoint(&pk).unwrap_or_else(|| pk.to_string());
                    self.spawn_outbound(group, &link_id, pk, peer_label, flags);
                }
                Err(e) => info!("[Saga Mesh] roster peer not dialed: {e}"),
            }
        }
    }

    /// Push the current roster to every encrypted link except `exclude`.
    fn broadcast_roster(self: &Arc<Self>, group: &Arc<MeshGroup>, exclude: &PublicKey) {
        let Ok(roster) = self.build_roster(group, None) else {
            return;
        };
        let targets: Vec<(Connection, Arc<HandshakeOutput>)> = {
            let Ok(links) = group.links.lock() else {
                return;
            };
            links
                .iter()
                .filter(|l| l.state == LinkState::Encrypted && l.peer != *exclude)
                .filter_map(|l| {
                    let hs = group_hs(l)?;
                    Some((l.voice.clone()?, hs))
                })
                .collect()
        };
        for (conn, hs) in targets {
            let roster = roster.clone();
            self.rt.spawn(async move {
                let Ok((mut send, _recv)) = conn.open_bi().await else {
                    return;
                };
                if let Err(e) = write_roster(&mut send, &hs, &roster).await {
                    warn!("[Saga Mesh] roster update write failed: {e}");
                }
                let _ = send.finish();
            });
        }
    }

    // ----- link establishment -------------------------------------------

    async fn outbound(
        self: &Arc<Self>,
        group: &Arc<MeshGroup>,
        link_id: &str,
        remote: PublicKey,
        peer_label: &str,
        flags: Arc<LinkFlags>,
    ) -> Result<(), String> {
        let addr = self
            .addr_book
            .lock()
            .ok()
            .and_then(|b| b.get(&remote).cloned());
        if self.wait_online {
            // Same warm-up policy as 1:1 connect_async (issue #49).
            if sample_relay_connected(&self.endpoint) {
                info!("[Saga Mesh] relay already connected before dialing [{peer_label}]");
            } else if tokio::time::timeout(RELAY_ONLINE_WAIT, self.endpoint.online())
                .await
                .is_ok()
                && sample_relay_connected(&self.endpoint)
            {
                info!("[Saga Mesh] relay online before dialing [{peer_label}]");
            } else {
                warn!(
                    "[Saga Mesh] relay online wait timed out before dialing [{peer_label}] — dialling with available hints"
                );
            }
        }
        let conn = dial_with_retry(
            &self.endpoint,
            remote,
            peer_label,
            SAGA_MESH_JOIN_ALPN,
            addr.as_ref(),
        )
        .await?;
        let (mut send, mut recv) = conn
            .open_bi()
            .await
            .map_err(|e| format!("mesh open_bi: {e}"))?;
        write_hello(&mut send, &group.id).await?;
        let identity = MockIdentity::from_label(&self.label);
        let local_eid = *self.local_id.as_bytes();
        let remote_eid = *conn.remote_id().as_bytes();
        let outcome = run_initiator_with_options(
            &identity,
            peer_label,
            &local_eid,
            &remote_eid,
            force_handshake_fail(),
            force_binding_perturb(),
            &mut recv,
            &mut send,
        )
        .await;
        let _ = send.finish();
        let hs = match outcome {
            HandshakeOutcome::Encrypted { output } => Arc::new(output),
            HandshakeOutcome::Failed { reason } => {
                conn.close(0u32.into(), b"saga mesh handshake failed");
                return Err(format!("handshake failed: {reason}"));
            }
        };
        let peer_fp = label_for_endpoint(&remote)
            .map(|l| identity_fingerprint(&l))
            .or_else(|| Some(identity_fingerprint(peer_label)));

        // Roster exchange: initiator writes first.
        let (mut s2, mut r2) = conn
            .open_bi()
            .await
            .map_err(|e| format!("mesh roster open_bi: {e}"))?;
        let mine = self.build_roster(group, peer_fp.map(|fp| (remote_eid, fp)))?;
        write_roster(&mut s2, &hs, &mine)
            .await
            .map_err(|e| format!("roster write: {e}"))?;
        let theirs = read_roster(&mut r2, &hs)
            .await
            .map_err(|e| format!("roster read: {e}"))?;
        let _ = s2.finish();

        // RoQ: initiator connects.
        let roq_addr = addr.unwrap_or_else(|| endpoint_addr_for_peer(remote, &self.endpoint));
        let mut roq = None;
        for attempt in 1..=8 {
            match self.endpoint.connect(roq_addr.clone(), ROQ_ALPN).await {
                Ok(c) => {
                    roq = Some(c);
                    break;
                }
                Err(e) => {
                    warn!("[Saga Mesh] RoQ connect attempt {attempt}/8: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
        let roq = roq.ok_or_else(|| "RoQ connect failed".to_string())?;
        self.finalize(
            group,
            link_id,
            remote,
            conn,
            roq,
            hs,
            peer_fp,
            CallRole::Caller,
            theirs,
            flags,
        )
        .await
    }

    /// Accept path for `saga/mesh-join/1` (no Telecom ring, ever).
    pub async fn handle_inbound(self: &Arc<Self>, conn: Connection) {
        let remote = conn.remote_id();
        match self.inbound(&conn, remote).await {
            Ok(()) => {}
            Err((group, link_id, e)) => {
                warn!("[Saga Mesh] inbound link from [{remote}] failed: {e}");
                if let (Some(g), Some(l)) = (group, link_id) {
                    self.fail_link(&g, &l);
                }
                conn.close(0u32.into(), b"saga mesh inbound rejected");
            }
        }
    }

    #[allow(clippy::type_complexity)]
    async fn inbound(
        self: &Arc<Self>,
        conn: &Connection,
        remote: PublicKey,
    ) -> Result<(), (Option<Arc<MeshGroup>>, Option<String>, String)> {
        let early = |e: String| (None, None, e);
        let (mut send, mut recv) = tokio::time::timeout(HELLO_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| early("accept_bi timed out".into()))?
            .map_err(|e| early(format!("accept_bi: {e}")))?;
        let group_id = tokio::time::timeout(HELLO_TIMEOUT, read_hello(&mut recv))
            .await
            .map_err(|_| early("hello timed out".into()))?
            .map_err(early)?;

        // Authorization: roster-introduced peer, or a known dev identity. Unknown
        // peers never get silently attached to this node's audio.
        if !self.inbound_authorized(&group_id, &remote).await {
            return Err(early(format!(
                "peer [{remote}] not introduced for group [{group_id}]"
            )));
        }
        let group = self.group_or_create(&group_id).map_err(early)?;
        let link_id = link_id_for(&group_id, &remote);
        let flags = self.insert_link(&group, &link_id, remote).map_err(early)?;
        let fail = |e: String| (Some(group.clone()), Some(link_id.clone()), e);
        self.emit(&group_id, &link_id, MESH_PEER_PENDING);

        // Register before the handshake so the initiator's RoQ connect is not dropped.
        let (roq_tx, roq_rx) = oneshot::channel();
        if let Ok(mut p) = self.pending_roq.lock() {
            p.insert(remote, roq_tx);
        }
        let result = self
            .inbound_link(
                &group, &link_id, conn, remote, &mut send, &mut recv, roq_rx, flags,
            )
            .await;
        if result.is_err() {
            if let Ok(mut p) = self.pending_roq.lock() {
                p.remove(&remote);
            }
        }
        result.map_err(fail)
    }

    #[allow(clippy::too_many_arguments)]
    async fn inbound_link(
        self: &Arc<Self>,
        group: &Arc<MeshGroup>,
        link_id: &str,
        conn: &Connection,
        remote: PublicKey,
        send: &mut iroh::endpoint::SendStream,
        recv: &mut iroh::endpoint::RecvStream,
        roq_rx: oneshot::Receiver<Connection>,
        flags: Arc<LinkFlags>,
    ) -> Result<(), String> {
        let identity = MockIdentity::from_label(&self.label);
        let local_eid = *self.local_id.as_bytes();
        let remote_eid = *remote.as_bytes();
        let outcome = run_responder_with_options(
            &identity,
            &local_eid,
            &remote_eid,
            force_handshake_fail(),
            force_binding_perturb(),
            recv,
            send,
        )
        .await;
        let hs = match outcome {
            HandshakeOutcome::Encrypted { output } => Arc::new(output),
            HandshakeOutcome::Failed { reason } => {
                return Err(format!("handshake failed: {reason}"));
            }
        };
        let peer_fp = label_for_endpoint(&remote).map(|l| identity_fingerprint(&l));

        // Roster exchange: responder reads first.
        let (mut s2, mut r2) = tokio::time::timeout(HELLO_TIMEOUT, conn.accept_bi())
            .await
            .map_err(|_| "roster accept_bi timed out".to_string())?
            .map_err(|e| format!("roster accept_bi: {e}"))?;
        let theirs = read_roster(&mut r2, &hs)
            .await
            .map_err(|e| format!("roster read: {e}"))?;
        let mine = self.build_roster(group, peer_fp.map(|fp| (remote_eid, fp)))?;
        write_roster(&mut s2, &hs, &mine)
            .await
            .map_err(|e| format!("roster write: {e}"))?;
        let _ = s2.finish();

        let roq = match tokio::time::timeout(ROQ_WAIT, roq_rx).await {
            Ok(Ok(c)) => c,
            _ => return Err("RoQ accept timed out".into()),
        };
        self.finalize(
            group,
            link_id,
            remote,
            conn.clone(),
            roq,
            hs,
            peer_fp,
            CallRole::Callee,
            theirs,
            flags,
        )
        .await
    }

    async fn inbound_authorized(&self, group_id: &str, remote: &PublicKey) -> bool {
        if label_for_endpoint(remote).is_some() {
            return true;
        }
        // A roster may still be in flight from the introducer: wait briefly.
        let deadline = tokio::time::Instant::now() + INBOUND_AUTH_WAIT;
        loop {
            if let Some(g) = self.group(group_id) {
                if g.expected
                    .lock()
                    .map(|e| e.contains_key(remote))
                    .unwrap_or(false)
                {
                    return true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Verify, attach media, publish ENCRYPTED, spread rosters, start the control loop.
    #[allow(clippy::too_many_arguments)]
    async fn finalize(
        self: &Arc<Self>,
        group: &Arc<MeshGroup>,
        link_id: &str,
        remote: PublicKey,
        conn: Connection,
        roq: Connection,
        hs: Arc<HandshakeOutput>,
        peer_fp: Option<[u8; 32]>,
        role: CallRole,
        roster: Roster,
        flags: Arc<LinkFlags>,
    ) -> Result<(), String> {
        self.validate_roster(&roster, &remote, peer_fp)?;
        // A roster-introduced peer must present the attested key.
        let attested = group
            .expected
            .lock()
            .ok()
            .and_then(|e| e.get(&remote).copied());
        if let Some(att) = attested {
            match peer_fp {
                Some(fp) if fp == att => {}
                _ => return Err("peer key does not match roster attestation".into()),
            }
        }

        // Media link first, so Kotlin's startMedia finds it attached.
        let session = self.attach_link(group, &remote, &roq, &hs, role).await?;

        {
            let mut links = group
                .links
                .lock()
                .map_err(|_| "links poisoned".to_string())?;
            let Some(l) = links.iter_mut().find(|l| l.link_id == link_id) else {
                // Removed (local hang-up) while establishing.
                drop(links);
                group
                    .removals
                    .lock()
                    .map(|mut r| r.push(*remote.as_bytes()))
                    .ok();
                conn.close(0u32.into(), b"saga mesh link removed");
                roq.close(0u32.into(), b"saga mesh link removed");
                return Ok(());
            };
            l.state = LinkState::Encrypted;
            l.fingerprint = peer_fp;
            l.voice = Some(conn.clone());
            l.roq = Some(roq);
            l.roq_session = Some(session);
            l.hs = Some(hs.clone());
            l.flags.attached.store(true, Ordering::Relaxed);
        }
        info!(
            "[Saga Mesh] link encrypted group=[{}] link=[{link_id}] peer=[{remote}]",
            group.id
        );
        self.emit(&group.id, link_id, MESH_PEER_ENCRYPTED);

        self.apply_roster(group, &remote, &roster);
        self.broadcast_roster(group, &remote);

        let node = self.clone();
        let g = group.clone();
        let lid = link_id.to_string();
        let handle = self.rt.spawn(async move {
            node.control_loop(g, lid, remote, conn, hs, flags).await;
        });
        if let Ok(mut links) = group.links.lock() {
            if let Some(l) = links.iter_mut().find(|l| l.link_id == link_id) {
                l.task = Some(handle.abort_handle());
            }
        }
        Ok(())
    }

    /// Accept roster updates on the voice connection; its end means the peer left.
    async fn control_loop(
        self: Arc<Self>,
        group: Arc<MeshGroup>,
        link_id: String,
        peer: PublicKey,
        conn: Connection,
        hs: Arc<HandshakeOutput>,
        flags: Arc<LinkFlags>,
    ) {
        while let Ok((_send, mut recv)) = conn.accept_bi().await {
            match read_roster(&mut recv, &hs).await {
                Ok(roster) => {
                    let fp = group.links.lock().ok().and_then(|l| {
                        l.iter()
                            .find(|x| x.link_id == link_id)
                            .and_then(|x| x.fingerprint)
                    });
                    match self.validate_roster(&roster, &peer, fp) {
                        Ok(()) => self.apply_roster(&group, &peer, &roster),
                        Err(e) => warn!("[Saga Mesh] roster update rejected: {e}"),
                    }
                }
                Err(e) => warn!("[Saga Mesh] roster update read failed: {e}"),
            }
        }
        // Connection ended. A local remove/close already took the entry out.
        let removed = {
            let Ok(mut links) = group.links.lock() else {
                return;
            };
            links
                .iter()
                .position(|l| l.link_id == link_id && Arc::ptr_eq(&l.flags, &flags))
                .map(|i| links.remove(i))
        };
        if let Some(l) = removed {
            close_link_info(&l);
            if let Ok(mut r) = group.removals.lock() {
                r.push(*l.peer.as_bytes());
            }
            info!(
                "[Saga Mesh] peer left group=[{}] link=[{link_id}]",
                group.id
            );
            self.emit(&group.id, &link_id, MESH_PEER_LEFT);
        }
    }

    // ----- media ---------------------------------------------------------

    async fn attach_link(
        &self,
        group: &Arc<MeshGroup>,
        peer: &PublicKey,
        roq: &Connection,
        hs: &Arc<HandshakeOutput>,
        role: CallRole,
    ) -> Result<RoqSession, String> {
        let session = RoqSession::new(roq.clone());
        let flow = MediaPipeline::flow_id_hint();
        let tx = session
            .new_send_flow(flow)
            .await
            .map_err(|e| format!("RoQ send flow: {e}"))?;
        let rx = session
            .new_receive_flow(flow)
            .await
            .map_err(|e| format!("RoQ recv flow: {e}"))?;
        let counters = Arc::new(MediaCounters::default());
        let (sender, receiver) =
            mesh_link_srtp(hs, role, tx, rx, counters).map_err(|e| format!("mesh link: {e}"))?;
        {
            let mut mixer = group.mixer.lock().await;
            group.apply_removals(&mut mixer).await;
            mixer
                .add_link(*peer.as_bytes(), sender, receiver)
                .map_err(|e| format!("mixer add_link: {e}"))?;
        }
        self.ensure_media(group);
        Ok(session)
    }

    fn ensure_media(&self, group: &Arc<MeshGroup>) {
        let Ok(mut media) = group.media.lock() else {
            return;
        };
        if media.is_some() {
            return;
        }
        let (capture_tx, capture_rx) = mpsc::channel::<Vec<i16>>(512);
        let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>(1);
        let ring = Arc::new(StdMutex::new(VecDeque::with_capacity(RING_FRAMES)));
        self.rt.spawn(media_task(
            group.clone(),
            capture_rx,
            shutdown_rx,
            ring.clone(),
        ));
        *media = Some(GroupMedia {
            capture_tx,
            ring,
            _shutdown: shutdown_tx,
        });
    }

    /// Resolve a group id or a link id to (group, link flags if a link id).
    fn resolve(&self, id: &str) -> Option<(Arc<MeshGroup>, Option<Arc<LinkFlags>>, bool)> {
        let groups = self.groups.lock().ok()?;
        if let Some(g) = groups.get(id) {
            return Some((g.clone(), None, true));
        }
        for g in groups.values() {
            let flags = g.links.lock().ok().and_then(|links| {
                links
                    .iter()
                    .find(|l| l.link_id == id)
                    .map(|l| l.flags.clone())
            });
            if let Some(f) = flags {
                return Some((g.clone(), Some(f), false));
            }
        }
        None
    }

    /// True when `id` names a mesh group or one of its links.
    pub fn owns(&self, id: &str) -> bool {
        self.resolve(id).is_some()
    }

    /// `nativeStartMedia` for mesh ids. `None` = not a mesh id (use the 1:1 path).
    /// Media is attached as soon as a link is encrypted; this confirms it.
    pub fn start_media(&self, id: &str, mode: i32) -> Option<bool> {
        let (group, link, _) = self.resolve(id)?;
        let ok = match link {
            Some(f) => f.attached.load(Ordering::Relaxed),
            None => !group.attached_flags().is_empty(),
        };
        if ok && mode == 2 {
            info!("[Saga Mesh] startMedia [{id}] plain requested; mesh links are always SRTP");
        }
        if ok {
            saga_media::disguise_begin_call();
        }
        Some(ok)
    }

    /// Group id: stop the group media task. Link id: no-op (the link leaves with
    /// `remove_peer`).
    pub fn stop_media(&self, id: &str) -> bool {
        let Some((group, link, _)) = self.resolve(id) else {
            return false;
        };
        if link.is_none() {
            if let Ok(mut m) = group.media.lock() {
                *m = None;
            }
        }
        true
    }

    pub fn media_running(&self, id: &str) -> bool {
        self.resolve(id)
            .map(|(g, _, _)| g.media.lock().map(|m| m.is_some()).unwrap_or(false))
            .unwrap_or(false)
    }

    /// Capture PCM (little-endian i16, one 20 ms frame) for a group id or the
    /// group's leader link id. Returns true when `id` is a mesh id.
    pub fn push_capture(&self, id: &str, pcm: &[u8]) -> bool {
        let Some((group, _, may_drive)) = self.resolve(id) else {
            return false;
        };
        if pcm.len() < FRAME_BYTES {
            return true;
        }
        let drives = may_drive || group.leader_link_id().as_deref() == Some(id);
        if !drives {
            return true;
        }
        let tx = group
            .media
            .lock()
            .ok()
            .and_then(|m| m.as_ref().map(|m| m.capture_tx.clone()));
        let Some(tx) = tx else {
            return true;
        };
        let mut frame = vec![0i16; FRAME_SAMPLES];
        for (i, s) in frame.iter_mut().enumerate() {
            *s = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]);
        }
        let _ = tx.try_send(frame);
        true
    }

    #[cfg(test)]
    fn playback_len(&self, id: &str) -> usize {
        self.resolve(id)
            .and_then(|(g, _, _)| {
                g.media.lock().ok().and_then(|m| {
                    m.as_ref()
                        .map(|m| m.ring.lock().map(|r| r.len()).unwrap_or(0))
                })
            })
            .unwrap_or(0)
    }

    /// Mixed playback for a group id or the leader link id; other link ids get
    /// silence. `None` when `id` is not a mesh id.
    pub fn pull_playback(&self, id: &str, out: &mut [u8]) -> Option<usize> {
        let (group, _, may_drive) = self.resolve(id)?;
        if out.len() < FRAME_BYTES {
            return Some(0);
        }
        out[..FRAME_BYTES].fill(0);
        let drives = may_drive || group.leader_link_id().as_deref() == Some(id);
        if drives {
            let ring = group
                .media
                .lock()
                .ok()
                .and_then(|m| m.as_ref().map(|m| m.ring.clone()));
            if let Some(ring) = ring {
                if let Ok(mut r) = ring.lock() {
                    if let Some(frame) = r.pop_front() {
                        out[..FRAME_BYTES].copy_from_slice(&frame[..FRAME_BYTES]);
                    }
                }
            }
        }
        Some(FRAME_BYTES)
    }

    fn for_each_flags(&self, id: &str, f: impl Fn(&LinkFlags)) -> bool {
        let Some((group, link, _)) = self.resolve(id) else {
            return false;
        };
        match link {
            Some(flags) => f(&flags),
            None => {
                if let Ok(links) = group.links.lock() {
                    for l in links.iter() {
                        f(&l.flags);
                    }
                }
            }
        }
        true
    }

    pub fn set_muted(&self, id: &str, muted: bool) -> bool {
        self.for_each_flags(id, |f| f.muted.store(muted, Ordering::Relaxed))
    }

    pub fn set_held(&self, id: &str, held: bool) -> bool {
        self.for_each_flags(id, |f| f.held.store(held, Ordering::Relaxed))
    }

    pub fn set_muted_all(&self, muted: bool) {
        let groups: Vec<Arc<MeshGroup>> = self
            .groups
            .lock()
            .map(|g| g.values().cloned().collect())
            .unwrap_or_default();
        for g in groups {
            if let Ok(links) = g.links.lock() {
                for l in links.iter() {
                    l.flags.muted.store(muted, Ordering::Relaxed);
                }
            }
        }
    }

    /// Link id: send the answered signal on that link. Group id: every link.
    pub fn on_local_answered(&self, id: &str) -> bool {
        let Some((group, link, _)) = self.resolve(id) else {
            return false;
        };
        match link {
            Some(f) => {
                if !f.attached.load(Ordering::Relaxed) {
                    return false;
                }
                f.answer_pending.store(true, Ordering::Relaxed);
                true
            }
            None => {
                let flags = group.attached_flags();
                for (_, f) in &flags {
                    f.answer_pending.store(true, Ordering::Relaxed);
                }
                !flags.is_empty()
            }
        }
    }

    pub fn is_send_armed(&self, id: &str) -> bool {
        let Some((group, link, _)) = self.resolve(id) else {
            return false;
        };
        match link {
            Some(f) => f.attached.load(Ordering::Relaxed),
            None => !group.attached_flags().is_empty(),
        }
    }
}

fn group_hs(l: &LinkInfo) -> Option<Arc<HandshakeOutput>> {
    l.hs.clone()
}

/// One task per group: encode-once/encrypt-per-link send, drain+mix receive.
async fn media_task(
    group: Arc<MeshGroup>,
    mut capture_rx: mpsc::Receiver<Vec<i16>>,
    mut shutdown_rx: mpsc::Receiver<()>,
    ring: Arc<StdMutex<VecDeque<Vec<u8>>>>,
) {
    let frame_dur =
        Duration::from_millis(1000 * FRAME_SAMPLES as u64 / saga_media::SAMPLE_RATE as u64);
    let mut tick = tokio::time::interval(frame_dur);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    info!("[Saga Mesh] media task running group=[{}]", group.id);
    loop {
        tokio::select! {
            _ = shutdown_rx.recv() => break,
            _ = tick.tick() => {
                let flags = group.attached_flags();
                let mut m = group.mixer.lock().await;
                group.apply_removals(&mut m).await;
                for (peer, f) in &flags {
                    m.set_link_muted(peer, f.muted.load(Ordering::Relaxed));
                    m.set_link_held(peer, f.held.load(Ordering::Relaxed));
                    if f.answer_pending.swap(false, Ordering::Relaxed) {
                        let _ = m.send_answered_signal(peer);
                        m.mark_answered(peer);
                    }
                }
                if m.link_count() == 0 {
                    while capture_rx.try_recv().is_ok() {}
                    continue;
                }
                m.recv_drain_all(5).await;
                match capture_rx.try_recv() {
                    Ok(mut pcm) => {
                        if let Err(e) = m.send_pcm_frame(&mut pcm) {
                            warn!("[Saga Mesh] send: {e}");
                        }
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                }
                let mut out = [0i16; FRAME_SAMPLES];
                if m.playout_mixed(&mut out).is_ok() {
                    let mut bytes = vec![0u8; FRAME_BYTES];
                    for (i, s) in out.iter().enumerate() {
                        bytes[i * 2..i * 2 + 2].copy_from_slice(&s.to_le_bytes());
                    }
                    if let Ok(mut r) = ring.lock() {
                        if r.len() >= RING_FRAMES {
                            r.pop_front();
                        }
                        r.push_back(bytes);
                    }
                }
            }
        }
    }
    info!("[Saga Mesh] media task stopped group=[{}]", group.id);
}

async fn write_hello(send: &mut iroh::endpoint::SendStream, group_id: &str) -> Result<(), String> {
    let bytes = group_id.as_bytes();
    if bytes.is_empty() || bytes.len() > MAX_GROUP_ID_LEN {
        return Err("invalid group id".into());
    }
    let mut msg = Vec::with_capacity(1 + bytes.len());
    msg.push(bytes.len() as u8);
    msg.extend_from_slice(bytes);
    AsyncWriteExt::write_all(send, &msg)
        .await
        .map_err(|e| format!("hello write: {e}"))
}

async fn read_hello(recv: &mut iroh::endpoint::RecvStream) -> Result<String, String> {
    let mut len = [0u8; 1];
    AsyncReadExt::read_exact(recv, &mut len)
        .await
        .map_err(|e| format!("hello len: {e}"))?;
    let n = len[0] as usize;
    if n == 0 || n > MAX_GROUP_ID_LEN {
        return Err("invalid hello length".into());
    }
    let mut buf = vec![0u8; n];
    AsyncReadExt::read_exact(recv, &mut buf)
        .await
        .map_err(|e| format!("hello body: {e}"))?;
    String::from_utf8(buf).map_err(|_| "hello not utf8".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::endpoint::presets;
    use iroh::RelayMode;
    use std::sync::Mutex;

    type Events = Arc<Mutex<Vec<(String, String, i32)>>>;

    struct TestNode {
        node: Arc<MeshNode>,
        events: Events,
        ep: Endpoint,
        _accept: tokio::task::JoinHandle<()>,
    }

    async fn test_node(label: &str) -> TestNode {
        let ep = Endpoint::builder(presets::Minimal)
            .alpns(vec![SAGA_MESH_JOIN_ALPN.to_vec(), ROQ_ALPN.to_vec()])
            .secret_key(dev_secret_key(label))
            .relay_mode(RelayMode::Disabled)
            .bind()
            .await
            .expect("bind");
        let events: Events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        let node = MeshNode::new(
            ep.clone(),
            label.to_string(),
            Handle::current(),
            false,
            Arc::new(move |g, l, s| {
                ev.lock().unwrap().push((g.to_string(), l.to_string(), s));
            }),
        );
        let accept_ep = ep.clone();
        let accept_node = node.clone();
        let accept = tokio::spawn(async move {
            while let Some(incoming) = accept_ep.accept().await {
                let n = accept_node.clone();
                tokio::spawn(async move {
                    let Ok(conn) = incoming.await else {
                        return;
                    };
                    if conn.alpn() == ROQ_ALPN {
                        let _ = n.offer_roq(conn);
                    } else if conn.alpn() == SAGA_MESH_JOIN_ALPN {
                        n.handle_inbound(conn).await;
                    }
                });
            }
        });
        TestNode {
            node,
            events,
            ep,
            _accept: accept,
        }
    }

    fn wire(nodes: &[&TestNode]) {
        for a in nodes {
            for b in nodes {
                if a.node.local_id() != b.node.local_id() {
                    a.node.set_peer_addr(b.node.local_id(), b.ep.addr());
                }
            }
        }
    }

    async fn wait_for(what: &str, mut cond: impl FnMut() -> bool) {
        for _ in 0..200 {
            if cond() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("timed out waiting for {what}");
    }

    fn link_id(group: &str, label: &str) -> String {
        format!("{group}/{label}")
    }

    fn tone_frame(freq: f64, frame: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(FRAME_BYTES);
        for n in 0..FRAME_SAMPLES {
            let t = (frame * FRAME_SAMPLES + n) as f64 / saga_media::SAMPLE_RATE as f64;
            let s = ((2.0 * std::f64::consts::PI * freq * t).sin() * 0.2 * 32767.0) as i16;
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    fn tone_power(pcm: &[i16], freq_hz: f64) -> f64 {
        let w = 2.0 * std::f64::consts::PI * freq_hz / saga_media::SAMPLE_RATE as f64;
        let coeff = 2.0 * w.cos();
        let (mut s1, mut s2) = (0.0f64, 0.0f64);
        for &x in pcm {
            let s0 = x as f64 + coeff * s1 - s2;
            s2 = s1;
            s1 = s0;
        }
        (s1 * s1 + s2 * s2 - coeff * s1 * s2) / (pcm.len() as f64).powi(2)
    }

    const A: &str = "15550100011";
    const B: &str = "15550100010";
    const C: &str = "15550100030";
    const D: &str = "15550100020";

    /// Drive every node's tone for `frames` frames; return each node's mixed playout.
    async fn run_audio(nodes: &[(&TestNode, &str, f64)], frames: usize) -> Vec<Vec<i16>> {
        let mut outs: Vec<Vec<i16>> = vec![Vec::new(); nodes.len()];
        for f in 0..frames {
            for (n, id, hz) in nodes.iter() {
                n.node.push_capture(id, &tone_frame(*hz, f));
            }
            // Lockstep with the 20 ms media tick: exactly one mixed frame per pushed frame.
            for (i, (n, id, _)) in nodes.iter().enumerate() {
                for _ in 0..100 {
                    if n.node.playback_len(id) > 0 {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
                let mut buf = vec![0u8; FRAME_BYTES];
                n.node.pull_playback(id, &mut buf);
                outs[i].extend(
                    buf.chunks_exact(2)
                        .map(|c| i16::from_le_bytes([c[0], c[1]])),
                );
            }
        }
        outs
    }

    fn rel(pcm: &[i16], hz: f64) -> f64 {
        let a = 0.2 * 32767.0;
        tone_power(pcm, hz) / (a * a / 4.0)
    }

    #[test]
    fn smaller_endpoint_id_dials() {
        let a = dev_secret_key(A).public();
        let b = dev_secret_key(B).public();
        assert_ne!(local_dials(&a, &b), local_dials(&b, &a));
    }

    #[test]
    fn mesh_join_alpn_is_distinct() {
        assert_ne!(SAGA_MESH_JOIN_ALPN, crate::iroh_transport::SAGA_VOICE_ALPN);
        assert_ne!(SAGA_MESH_JOIN_ALPN, ROQ_ALPN);
    }

    #[test]
    fn fingerprint_is_64_lowercase_hex() {
        let hex = fingerprint_hex(&identity_fingerprint(A));
        assert_eq!(hex.len(), 64);
        assert!(hex
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn add_third_peer_builds_full_mesh_media_and_leave() {
        let a = test_node(A).await;
        let b = test_node(B).await;
        let c = test_node(C).await;
        wire(&[&a, &b, &c]);
        let g = "grp-test";

        assert!(a.node.open(g));
        let a_b = link_id(g, B);
        let a_c = link_id(g, C);
        assert!(a.node.add_peer(g, &a_b, B));
        wait_for("A-B encrypted", || a.node.encrypted_peers(g).len() == 1).await;
        wait_for("B inbound", || b.node.encrypted_peers(g).len() == 1).await;

        // A adds C while talking to B: B and C must then link directly.
        assert!(a.node.add_peer(g, &a_c, C));
        wait_for("full mesh", || {
            a.node.encrypted_peers(g).len() == 2
                && b.node.encrypted_peers(g).len() == 2
                && c.node.encrypted_peers(g).len() == 2
        })
        .await;

        // Fingerprints come from the handshake (64 hex lowercase).
        assert_eq!(
            a.node.peer_fingerprint(g, &a_c).unwrap(),
            fingerprint_hex(&identity_fingerprint(C))
        );
        let b_c = link_id(g, C);
        assert_eq!(
            b.node.peer_fingerprint(g, &b_c).unwrap(),
            fingerprint_hex(&identity_fingerprint(C))
        );
        assert!(a.node.peer_fingerprint(g, "missing").is_none());

        // Media: every peer hears the other two, never itself.
        let mixed = run_audio(&[(&a, g, 440.0), (&b, g, 660.0), (&c, g, 880.0)], 120).await;
        let tail = |v: &Vec<i16>| v[50 * FRAME_SAMPLES..].to_vec();
        let (ma, mb, mc) = (tail(&mixed[0]), tail(&mixed[1]), tail(&mixed[2]));
        assert!(rel(&ma, 660.0) > 0.2 && rel(&ma, 880.0) > 0.2, "A mix");
        assert!(rel(&ma, 440.0) < 0.02, "A must not hear itself");
        assert!(rel(&mb, 440.0) > 0.2 && rel(&mb, 880.0) > 0.2, "B mix");
        assert!(rel(&mb, 660.0) < 0.02);
        assert!(rel(&mc, 440.0) > 0.2 && rel(&mc, 660.0) > 0.2, "C mix");
        assert!(rel(&mc, 880.0) < 0.02);

        // Starter leaves: B and C keep their mutual link and keep talking.
        a.node.close(g);
        wait_for("B and C see A leave", || {
            b.node.encrypted_peers(g).len() == 1 && c.node.encrypted_peers(g).len() == 1
        })
        .await;
        assert!(b
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|(_, l, s)| *l == link_id(g, A) && *s == MESH_PEER_LEFT));
        let mixed = run_audio(&[(&b, g, 660.0), (&c, g, 880.0)], 70).await;
        let tail = |v: &Vec<i16>| v[30 * FRAME_SAMPLES..].to_vec();
        assert!(rel(&tail(&mixed[0]), 880.0) > 0.2, "B hears C after A left");
        assert!(rel(&tail(&mixed[1]), 660.0) > 0.2, "C hears B after A left");
        assert!(
            rel(&tail(&mixed[0]), 440.0) < 0.02,
            "A is gone from B's mix"
        );

        // No Telecom ring is possible here: mesh-join never reaches
        // handle_inbound_connection / notifyIncomingCall (separate ALPN).
        let ev = c.events.lock().unwrap();
        assert!(ev.iter().any(|(_, _, s)| *s == MESH_PEER_ENCRYPTED));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn mesh_is_limited_to_four_participants() {
        let a = test_node(A).await;
        let b = test_node(B).await;
        let c = test_node(C).await;
        let d = test_node(D).await;
        wire(&[&a, &b, &c, &d]);
        let g = "grp-four";
        assert!(a.node.open(g));
        assert!(a.node.add_peer(g, &link_id(g, B), B));
        wait_for("A-B", || a.node.encrypted_peers(g).len() == 1).await;
        assert!(a.node.add_peer(g, &link_id(g, C), C));
        wait_for("3 mesh", || {
            a.node.encrypted_peers(g).len() == 2 && b.node.encrypted_peers(g).len() == 2
        })
        .await;
        assert!(a.node.add_peer(g, &link_id(g, D), D));
        wait_for("4 mesh (6 links)", || {
            [&a, &b, &c, &d]
                .iter()
                .all(|n| n.node.encrypted_peers(g).len() == 3)
        })
        .await;
        // A fifth participant is refused before any dial.
        assert!(!a
            .node
            .add_peer(g, &link_id(g, "15550100099"), "15550100099"));
        assert_eq!(a.node.link_ids(g).len(), 3);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn remove_peer_closes_one_link_and_mute_hold_are_per_link() {
        let a = test_node(A).await;
        let b = test_node(B).await;
        wire(&[&a, &b]);
        let g = "grp-rm";
        assert!(a.node.open(g));
        let l = link_id(g, B);
        assert!(a.node.add_peer(g, &l, B));
        wait_for("link", || a.node.encrypted_peers(g).len() == 1).await;
        assert!(a.node.is_send_armed(&l));
        assert_eq!(a.node.start_media(&l, 1), Some(true));
        assert!(a.node.set_muted(&l, true));
        assert!(a.node.set_held(&l, true));
        assert!(a.node.on_local_answered(&l));
        a.node.remove_peer(g, &l);
        wait_for("B sees leave", || b.node.encrypted_peers(g).is_empty()).await;
        assert!(a.node.link_ids(g).is_empty());
        assert!(!a.node.is_send_armed(&l));
    }
}
