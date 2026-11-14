//! Real Iroh transport (feature `iroh-transport`).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, LazyLock, Mutex, OnceLock};
use std::time::Duration;

use iroh::endpoint::Connection;
#[cfg(feature = "mock-token")]
use iroh::tls::CaTlsConfig;
#[cfg(feature = "mock-token")]
use iroh::SecretKey;
use iroh::{
    endpoint::presets, Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, Watcher,
};
use iroh_roq::ALPN as ROQ_ALPN;
use log::{error, info, warn};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

#[cfg(feature = "mock-token")]
use mock_token::{
    run_initiator_with_options, run_responder_with_options, HandshakeOutcome, MockIdentity,
};
use saga_media::CallRole;
use saga_session::HandshakeOutput;

pub const SAGA_VOICE_ALPN: &[u8] = b"saga/voice/1";

/// Per-attempt connect timeout (QUIC dial).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
/// Wall-clock budget for the whole connect phase (excluding post-connect handshake).
/// Host public-relay soak: home relay ~3s, voice connect ~70ms once discovered.
/// 18s leaves room for 2–3 slow attempts + addressing misses without a 40s hang.
const CONNECT_BUDGET: Duration = Duration::from_secs(18);
const RELAY_ONLINE_WAIT: Duration = Duration::from_secs(12);
const CONNECT_RETRY_SLEEP: Duration = Duration::from_secs(1);
const MAX_CONNECT_ATTEMPTS: u32 = 3;
/// Addressing/discovery misses fail faster — no point burning the full budget.
const ADDRESSING_MAX_ATTEMPTS: u32 = 2;

fn is_addressing_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("no addressing information")
        || lower.contains("addressing information available")
        || lower.contains("failed to resolve")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakePoll {
    Pending = 0,
    /// Constructed when a SessionToken handshake succeeds (mock-token / future USB).
    #[allow(dead_code)] // release builds have no token backend yet
    Encrypted = 1,
    Failed = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelayPoll {
    Pending = 0,
    Ready = 1,
    Failed = 2,
}

struct Session {
    connection: Connection,
    handshake: HandshakePoll,
    /// Full handshake output for SRTP key derivation (saga-media).
    handshake_output: Option<Arc<HandshakeOutput>>,
    role: CallRole,
    /// RoQ connection established after voice handshake (media path).
    roq: Option<Connection>,
}

struct EndpointHolder {
    endpoint: Endpoint,
    #[allow(dead_code)]
    accept_shutdown: oneshot::Sender<()>,
}

static RUNTIME: OnceLock<Runtime> = OnceLock::new();
static SHARED_ENDPOINT: OnceLock<EndpointHolder> = OnceLock::new();
static SESSIONS: LazyLock<Mutex<HashMap<String, Session>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// Responder waits here for the initiator's RoQ connect after voice handshake.
static PENDING_ROQ: LazyLock<Mutex<HashMap<PublicKey, oneshot::Sender<Connection>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

static DEV_IDENTITY_LABEL: Mutex<Option<String>> = Mutex::new(None);
static RELAY_URL_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
static FORCE_HANDSHAKE_FAIL: AtomicBool = AtomicBool::new(false);
static FORCE_BINDING_PERTURB: AtomicBool = AtomicBool::new(false);
static RELAY_POLL: AtomicU8 = AtomicU8::new(RelayPoll::Pending as u8);

pub(crate) fn runtime() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("saga-iroh")
            .build()
            .expect("tokio runtime for saga-iroh-core")
    })
}

pub fn set_dev_identity(label: &str) {
    if let Ok(mut guard) = DEV_IDENTITY_LABEL.lock() {
        *guard = Some(label.to_string());
        info!("dev identity label set to [{label}]");
    }
}

pub fn set_relay_url(url: &str) {
    if SHARED_ENDPOINT.get().is_some() {
        warn!("set_relay_url ignored — endpoint already bound");
        return;
    }
    let trimmed = url.trim();
    if let Ok(mut guard) = RELAY_URL_OVERRIDE.lock() {
        if trimmed.is_empty() {
            *guard = None;
            info!("relay url override cleared — using default N0 relays");
        } else {
            *guard = Some(trimmed.to_string());
            info!("relay url override set to [{trimmed}]");
        }
    }
}

/// True when at least one home relay is currently connected (live sample, not latched).
fn sample_relay_connected(endpoint: &Endpoint) -> bool {
    let mut watcher = endpoint.home_relay_status();
    watcher.get().into_iter().any(|s| s.is_connected())
}

/// Exposes live home-relay connectivity to JNI / the E2E harness.
///
/// Important: do **not** latch Ready forever after the first `online()`. Emulator and
/// local-relay churn can drop the relay after bind; a latched Ready caused the harness
/// to dial while the callee could no longer accept inbound (RING flake, first missing
/// checkpoint = `inbound_accepted`).
pub fn poll_relay_ready() -> RelayPoll {
    let Some(holder) = SHARED_ENDPOINT.get() else {
        return RelayPoll::Pending;
    };
    if RELAY_POLL.load(Ordering::SeqCst) == RelayPoll::Failed as u8 {
        return RelayPoll::Failed;
    }
    if sample_relay_connected(&holder.endpoint) {
        RELAY_POLL.store(RelayPoll::Ready as u8, Ordering::SeqCst);
        RelayPoll::Ready
    } else {
        let prev = RELAY_POLL.swap(RelayPoll::Pending as u8, Ordering::SeqCst);
        if prev == RelayPoll::Ready as u8 {
            warn!(
                "[Saga Iroh Listen] relay no longer connected — poll returns pending local id=[{}]",
                holder.endpoint.id()
            );
        }
        RelayPoll::Pending
    }
}

fn relay_url_override() -> Option<String> {
    RELAY_URL_OVERRIDE.lock().ok().and_then(|g| g.clone())
}

fn apply_relay_builder(
    builder: iroh::endpoint::Builder,
) -> Result<iroh::endpoint::Builder, String> {
    let Some(url) = relay_url_override() else {
        return Ok(builder);
    };
    let relay_url: RelayUrl = url
        .parse()
        .map_err(|e| format!("invalid relay url [{url}]: {e}"))?;
    let relay_map = RelayMap::from_iter([relay_url]);
    let builder = builder.relay_mode(RelayMode::Custom(relay_map));
    // Insecure local-relay TLS is debug/E2E only (mock-token / test-utils).
    #[cfg(feature = "mock-token")]
    {
        Ok(builder.ca_tls_config(CaTlsConfig::insecure_skip_verify()))
    }
    #[cfg(not(feature = "mock-token"))]
    {
        Ok(builder)
    }
}

/// Bind the shared endpoint and start accepting inbound Iroh connections.
/// Must run on the callee before any peer dials in (connect() alone only binds on the initiator).
pub fn ensure_listening() -> Result<(), String> {
    let endpoint = bind_shared_endpoint()?;
    info!(
        "[Saga Iroh Listen] endpoint bound and accepting inbound, local id=[{}]",
        endpoint.id()
    );
    Ok(())
}

pub fn set_force_handshake_fail(force: bool) {
    FORCE_HANDSHAKE_FAIL.store(force, Ordering::SeqCst);
}

pub fn set_force_binding_perturb(force: bool) {
    FORCE_BINDING_PERTURB.store(force, Ordering::SeqCst);
}

#[cfg(feature = "mock-token")]
fn force_handshake_fail() -> bool {
    FORCE_HANDSHAKE_FAIL.load(Ordering::SeqCst)
}

#[cfg(feature = "mock-token")]
fn force_binding_perturb() -> bool {
    FORCE_BINDING_PERTURB.load(Ordering::SeqCst)
}

#[cfg(feature = "mock-token")]
fn local_identity_label() -> String {
    DEV_IDENTITY_LABEL
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "15550100011".to_string())
}

#[cfg(feature = "mock-token")]
fn dev_secret_key(peer_label: &str) -> SecretKey {
    // Literal kept only in mock-token builds (CI release .so greps for this prefix).
    let material = format!("saga-dev-v1:{peer_label}");
    let hash = blake3::hash(material.as_bytes());
    let bytes: [u8; 32] = *hash.as_bytes();
    SecretKey::from_bytes(&bytes)
}

pub fn parse_peer_endpoint_id(peer_id: &str) -> Result<PublicKey, String> {
    if let Ok(pk) = PublicKey::from_str(peer_id) {
        return Ok(pk);
    }
    #[cfg(feature = "mock-token")]
    {
        let is_dev_label = peer_id.len() >= 8
            && peer_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if is_dev_label {
            let pk = dev_secret_key(peer_id).public();
            info!("dev peer label [{peer_id}] -> endpoint id [{pk}]");
            return Ok(pk);
        }
        Err(format!(
            "invalid iroh peer id [{peer_id}]: expected hex/base32 EndpointId or dev label (8+ alnum)"
        ))
    }
    #[cfg(not(feature = "mock-token"))]
    {
        Err(format!(
            "invalid iroh peer id [{peer_id}]: expected hex/base32 EndpointId"
        ))
    }
}

fn bind_shared_endpoint() -> Result<&'static Endpoint, String> {
    if let Some(holder) = SHARED_ENDPOINT.get() {
        return Ok(&holder.endpoint);
    }
    let holder = runtime()
        .block_on(async { bind_endpoint_async().await })
        .map_err(|e| format!("iroh endpoint bind failed: {e}"))?;
    SHARED_ENDPOINT
        .set(holder)
        .map_err(|_| "shared endpoint already set".to_string())?;
    Ok(&SHARED_ENDPOINT.get().expect("just set").endpoint)
}

#[cfg(feature = "mock-token")]
fn dev_label_for_endpoint(remote: &PublicKey) -> Option<String> {
    // Only encrypted Iroh peers. Never include cellular-only labels (thor
    // +15550100012) — those must not resolve to an Iroh identity.
    for label in ["15550100010", "15550100011", "15550100020"] {
        if dev_secret_key(label).public() == *remote {
            return Some(label.to_string());
        }
    }
    None
}

#[cfg(not(feature = "mock-token"))]
fn dev_label_for_endpoint(_remote: &PublicKey) -> Option<String> {
    None
}

fn inbound_session_id(lookup_key: &str) -> String {
    let safe: String = lookup_key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!(
        "inbound-{safe}-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    )
}

async fn handle_inbound_connection(conn: Connection) {
    let remote_pk = conn.remote_id();
    let remote_id_str = remote_pk.to_string();
    let lookup_key = dev_label_for_endpoint(&remote_pk).unwrap_or_else(|| remote_id_str.clone());
    let session_id = inbound_session_id(&lookup_key);

    info!(
        "[Saga Iroh Listen] inbound accepted remote=[{remote_id_str}] lookup=[{lookup_key}] session=[{session_id}]"
    );

    // Insert before JNI notify so Kotlin can disconnect() if PhoneAccount is disabled.
    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.insert(
            session_id.clone(),
            Session {
                connection: conn,
                handshake: HandshakePoll::Pending,
                handshake_output: None,
                role: CallRole::Callee,
                roq: None,
            },
        );
    } else {
        warn!("session map poisoned — dropping inbound [{session_id}]");
        return;
    }

    #[cfg(feature = "iroh-transport")]
    crate::bridge_notify_incoming_call(&session_id, &lookup_key, &remote_id_str);

    // Window for main-thread bridge to reject (disabled account / SecurityException).
    // Emulator main-thread lag after Settings UI / force-stop made 250ms flaky for
    // onboarding Case C (INBOUND_REJECT missing reject log).
    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

    let conn = {
        let Ok(mut sessions) = SESSIONS.lock() else {
            return;
        };
        match sessions.remove(&session_id) {
            Some(session) => session.connection,
            None => {
                info!(
                    "[Saga Iroh Listen] inbound rejected by bridge session=[{session_id}] — not reachable"
                );
                return;
            }
        }
    };

    #[cfg(feature = "mock-token")]
    let (handshake, handshake_output, roq) = match conn.accept_bi().await {
        Ok((mut send, mut recv)) => {
            let identity = MockIdentity::from_label(&local_identity_label());
            let Ok(ep) = bind_shared_endpoint() else {
                warn!("inbound mock-token: local endpoint unavailable");
                conn.close(0u32.into(), b"saga no local eid");
                return;
            };
            // Register before handshake so initiator RoQ connect is not dropped.
            let remote = conn.remote_id();
            let (tx, rx) = oneshot::channel();
            if let Ok(mut pending) = PENDING_ROQ.lock() {
                pending.insert(remote, tx);
            }
            let local_eid = *ep.id().as_bytes();
            let remote_eid = *conn.remote_id().as_bytes();
            let outcome = run_responder_with_options(
                &identity,
                &local_eid,
                &remote_eid,
                force_handshake_fail(),
                force_binding_perturb(),
                &mut recv,
                &mut send,
            )
            .await;
            match outcome {
                HandshakeOutcome::Encrypted { output } => {
                    #[cfg(debug_assertions)]
                    {
                        let (a, b) = output.session_key_debug_prefix();
                        info!(
                            "[Saga Session Key] responder session_key_hash_prefix={a:02x}{b:02x}"
                        );
                    }
                    let roq = match tokio::time::timeout(Duration::from_secs(15), rx).await {
                        Ok(Ok(c)) => {
                            info!("[Saga Media] responder RoQ accepted from {remote}");
                            Some(c)
                        }
                        _ => {
                            warn!("[Saga Media] responder RoQ accept timed out");
                            if let Ok(mut pending) = PENDING_ROQ.lock() {
                                pending.remove(&remote);
                            }
                            None
                        }
                    };
                    (HandshakePoll::Encrypted, Some(Arc::new(output)), roq)
                }
                HandshakeOutcome::Failed { reason } => {
                    warn!("inbound mock-token handshake failed: {reason}");
                    if let Ok(mut pending) = PENDING_ROQ.lock() {
                        pending.remove(&remote);
                    }
                    (HandshakePoll::Failed, None, None)
                }
            }
        }
        Err(e) => {
            warn!("inbound accept_bi failed (caller sees not reachable): {e}");
            conn.close(0u32.into(), b"saga inbound reject");
            return;
        }
    };

    #[cfg(not(feature = "mock-token"))]
    let (handshake, handshake_output, roq) = (HandshakePoll::Failed, None, None);

    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.insert(
            session_id,
            Session {
                connection: conn,
                handshake,
                handshake_output,
                role: CallRole::Callee,
                roq,
            },
        );
        info!("inbound session registered lookup=[{lookup_key}] handshake={handshake:?}");
    }
}

async fn bind_endpoint_async() -> Result<EndpointHolder, String> {
    RELAY_POLL.store(RelayPoll::Pending as u8, Ordering::SeqCst);
    // Default DnsResolver reads system nameservers via ndk_context (see
    // IrohNativeBridge.installAndroidContext). Do not pin third-party resolvers:
    // that would leak callee discovery lookups and override Private DNS / VPN / Orbot.
    let mut builder =
        Endpoint::builder(presets::N0).alpns(vec![SAGA_VOICE_ALPN.to_vec(), ROQ_ALPN.to_vec()]);
    builder = apply_relay_builder(builder)?;
    #[cfg(feature = "mock-token")]
    {
        let dev_label = DEV_IDENTITY_LABEL.lock().ok().and_then(|g| g.clone());
        if let Some(label) = dev_label {
            let sk = dev_secret_key(&label);
            info!(
                "binding iroh endpoint with dev identity label [{label}] id=[{}]",
                sk.public()
            );
            builder = builder.secret_key(sk);
        }
    }
    let endpoint = builder.bind().await.map_err(|e| format!("{e}"))?;
    info!("iroh endpoint bound, local id=[{}]", endpoint.id());
    let ep_for_online = endpoint.clone();
    tokio::spawn(async move {
        ep_for_online.online().await;
        RELAY_POLL.store(RelayPoll::Ready as u8, Ordering::SeqCst);
        info!(
            "[Saga Iroh Listen] relay online, local id=[{}]",
            ep_for_online.id()
        );
        // Keep RELAY_POLL aligned with live home-relay status (reconnect / drop).
        let mut watcher = ep_for_online.home_relay_status();
        while let Ok(statuses) = watcher.updated().await {
            let connected = statuses.iter().any(|s| s.is_connected());
            if connected {
                let prev = RELAY_POLL.swap(RelayPoll::Ready as u8, Ordering::SeqCst);
                if prev != RelayPoll::Ready as u8 {
                    info!(
                        "[Saga Iroh Listen] relay reconnected, local id=[{}]",
                        ep_for_online.id()
                    );
                }
            } else {
                let prev = RELAY_POLL.swap(RelayPoll::Pending as u8, Ordering::SeqCst);
                if prev == RelayPoll::Ready as u8 {
                    warn!(
                        "[Saga Iroh Listen] relay disconnected — awaiting reconnect local id=[{}]",
                        ep_for_online.id()
                    );
                }
            }
        }
    });

    let accept_ep = endpoint.clone();
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => break,
                incoming = accept_ep.accept() => {
                    match incoming {
                        Some(connecting) => {
                            tokio::spawn(async move {
                                info!("[Saga Iroh Listen] inbound connecting (QUIC handshake)");
                                match connecting.await {
                                    Ok(conn) => {
                                        if conn.alpn() == ROQ_ALPN {
                                            let remote = conn.remote_id();
                                            if let Ok(mut pending) = PENDING_ROQ.lock() {
                                                if let Some(tx) = pending.remove(&remote) {
                                                    let _ = tx.send(conn);
                                                    return;
                                                }
                                            }
                                            warn!("unsolicited RoQ from {remote}");
                                        } else {
                                            handle_inbound_connection(conn).await;
                                        }
                                    }
                                    Err(e) => warn!("iroh inbound handshake failed: {e}"),
                                }
                            });
                        }
                        None => break,
                    }
                }
            }
        }
    });

    Ok(EndpointHolder {
        endpoint,
        accept_shutdown: shutdown_tx,
    })
}

/// Build a dial address that does not depend solely on pkarr/DNS discovery.
///
/// Emulators often fail pkarr (`dns.iroh.link` timeouts). With a custom/local relay
/// both peers already share, hinting that relay URL on `EndpointAddr` avoids
/// `No addressing information available` connect flakes (RING first_missing=inbound_accepted).
fn endpoint_addr_for_peer(remote: PublicKey, endpoint: &Endpoint) -> EndpointAddr {
    let addr = EndpointAddr::new(remote);
    if let Some(url) = relay_url_override() {
        match url.parse::<RelayUrl>() {
            Ok(relay_url) => {
                info!("connect EndpointAddr hints custom relay [{url}]");
                return addr.with_relay_url(relay_url);
            }
            Err(e) => warn!("custom relay url parse failed [{url}]: {e}"),
        }
    }
    let mut watcher = endpoint.home_relay_status();
    for status in watcher.get() {
        if status.is_connected() {
            info!("connect EndpointAddr hints home relay [{}]", status.url());
            return addr.with_relay_url(status.url().clone());
        }
    }
    warn!("connect EndpointAddr has no relay hint — discovery-only");
    addr
}

async fn connect_async(peer_id: &str, session_id: &str) -> Result<(), String> {
    let remote = parse_peer_endpoint_id(peer_id)?;
    let endpoint = bind_shared_endpoint()?;
    if tokio::time::timeout(RELAY_ONLINE_WAIT, endpoint.online())
        .await
        .is_err()
    {
        warn!(
            "initiator relay online wait timed out before connect to [{peer_id}] — trying anyway"
        );
    } else {
        info!("initiator relay online before connect to [{peer_id}]");
    }

    let mut connection = None;
    let mut last_err = String::new();
    let deadline = tokio::time::Instant::now() + CONNECT_BUDGET;
    for attempt in 1..=MAX_CONNECT_ATTEMPTS {
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "NOT_REACHABLE: connect budget {}s exhausted after {attempt} attempt(s): {last_err}",
                CONNECT_BUDGET.as_secs()
            ));
        }
        // Refresh address each attempt so late pkarr/DNS publish can land.
        let attempt_addr = endpoint_addr_for_peer(remote, endpoint);
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            endpoint.connect(attempt_addr, SAGA_VOICE_ALPN),
        )
        .await
        {
            Ok(Ok(conn)) => {
                connection = Some(conn);
                break;
            }
            Ok(Err(e)) => {
                last_err = format!("{e}");
                let addressing = is_addressing_error(&last_err);
                let max_attempts = if addressing {
                    ADDRESSING_MAX_ATTEMPTS.min(MAX_CONNECT_ATTEMPTS)
                } else {
                    MAX_CONNECT_ATTEMPTS
                };
                if attempt < max_attempts
                    && tokio::time::Instant::now() + CONNECT_RETRY_SLEEP < deadline
                {
                    warn!(
                        "iroh connect attempt {attempt}/{max_attempts} to [{peer_id}]: {last_err} — retrying"
                    );
                    tokio::time::sleep(CONNECT_RETRY_SLEEP).await;
                    continue;
                }
                return Err(format!("NOT_REACHABLE: iroh connect error: {e}"));
            }
            Err(_) => {
                last_err = format!("timed out after {}s", CONNECT_TIMEOUT.as_secs());
                if attempt < MAX_CONNECT_ATTEMPTS
                    && tokio::time::Instant::now() + CONNECT_RETRY_SLEEP < deadline
                {
                    warn!(
                        "iroh connect attempt {attempt}/{MAX_CONNECT_ATTEMPTS} to [{peer_id}] timed out — retrying"
                    );
                    tokio::time::sleep(CONNECT_RETRY_SLEEP).await;
                    continue;
                }
                return Err(format!(
                    "NOT_REACHABLE: iroh connect timed out after {attempt} attempt(s) ({}s each)",
                    CONNECT_TIMEOUT.as_secs()
                ));
            }
        }
    }
    let connection = connection
        .ok_or_else(|| format!("NOT_REACHABLE: iroh connect failed after retries: {last_err}"))?;

    #[cfg(feature = "mock-token")]
    let (handshake, handshake_output, roq) = {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("iroh open_bi failed: {e}"))?;
        let identity = MockIdentity::from_label(&local_identity_label());
        let peer_label = dev_label_for_endpoint(&remote).unwrap_or_else(|| peer_id.to_string());
        let local_eid = *endpoint.id().as_bytes();
        let remote_eid = *connection.remote_id().as_bytes();
        let outcome = run_initiator_with_options(
            &identity,
            &peer_label,
            &local_eid,
            &remote_eid,
            force_handshake_fail(),
            force_binding_perturb(),
            &mut recv,
            &mut send,
        )
        .await;
        match outcome {
            HandshakeOutcome::Encrypted { output } => {
                #[cfg(debug_assertions)]
                {
                    let (a, b) = output.session_key_debug_prefix();
                    info!("[Saga Session Key] initiator session_key_hash_prefix={a:02x}{b:02x}");
                }
                let _ = send.finish();
                let remote_pk = connection.remote_id();
                let mut roq = None;
                let roq_addr = endpoint_addr_for_peer(remote_pk, endpoint);
                for attempt in 1..=8 {
                    match endpoint.connect(roq_addr.clone(), ROQ_ALPN).await {
                        Ok(c) => {
                            info!(
                                "[Saga Media] initiator RoQ connected to {remote_pk} attempt={attempt}"
                            );
                            roq = Some(c);
                            break;
                        }
                        Err(e) => {
                            warn!("[Saga Media] initiator RoQ connect attempt {attempt}/8: {e}");
                            tokio::time::sleep(Duration::from_millis(200)).await;
                        }
                    }
                }
                (HandshakePoll::Encrypted, Some(Arc::new(output)), roq)
            }
            HandshakeOutcome::Failed { reason } => {
                warn!("mock-token handshake failed: {reason}");
                let _ = send.finish();
                (HandshakePoll::Failed, None, None)
            }
        }
    };

    #[cfg(not(feature = "mock-token"))]
    let (handshake, handshake_output, roq) = {
        let (mut send, _recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("iroh open_bi failed: {e}"))?;
        // iroh SendStream provides write_all without AsyncWriteExt in scope.
        send.write_all(b"saga-connect-probe")
            .await
            .map_err(|e| format!("iroh probe write: {e}"))?;
        let _ = send.finish();
        (HandshakePoll::Failed, None, None)
    };

    info!(
        "iroh connected to [{}] handshake={handshake:?}",
        connection.remote_id()
    );

    SESSIONS
        .lock()
        .map_err(|_| "session map poisoned".to_string())?
        .insert(
            session_id.to_string(),
            Session {
                connection,
                handshake,
                handshake_output,
                role: CallRole::Caller,
                roq,
            },
        );

    Ok(())
}

pub fn connect(peer_id: &str, session_id: &str) -> Result<(), String> {
    if peer_id.to_ascii_lowercase().contains("fail") {
        return Err("native connect failure (peer id contains 'fail')".into());
    }
    runtime()
        .block_on(connect_async(peer_id, session_id))
        .map_err(|e| {
            error!("connect_async failed: {e}");
            e
        })
}

pub fn disconnect(session_id: &str) {
    crate::media_bridge::stop_media(session_id);
    if let Some(session) = SESSIONS.lock().ok().and_then(|mut m| m.remove(session_id)) {
        if let Some(roq) = session.roq {
            roq.close(0u32.into(), b"saga disconnect");
        }
        session.connection.close(0u32.into(), b"saga disconnect");
    }
}

pub fn poll_handshake(session_id: &str) -> HandshakePoll {
    SESSIONS
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).map(|s| s.handshake))
        .unwrap_or(HandshakePoll::Pending)
}

/// Hex session key for debug. Not yet exposed over JNI.
#[allow(dead_code)]
pub fn session_key_hex(session_id: &str) -> Option<String> {
    SESSIONS.lock().ok().and_then(|m| {
        m.get(session_id).and_then(|s| {
            s.handshake_output.as_ref().map(|o| {
                o.saga_session_key
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            })
        })
    })
}

/// True when RoQ is ready or media is running (replaces synthetic media probe).
pub fn media_round_trip_ok(session_id: &str) -> bool {
    if crate::media_bridge::media_running(session_id) {
        return true;
    }
    SESSIONS
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).map(|s| s.roq.is_some()))
        .unwrap_or(false)
}

/// Take RoQ connection for media start (once). Handshake output stays on the session.
pub fn take_media_setup(
    session_id: &str,
) -> Option<(CallRole, Option<Arc<HandshakeOutput>>, Connection)> {
    let Ok(mut map) = SESSIONS.lock() else {
        return None;
    };
    let s = map.get_mut(session_id)?;
    let roq = s.roq.take()?;
    let hs = s.handshake_output.clone();
    Some((s.role, hs, roq))
}

pub fn handshake_output(session_id: &str) -> Option<Arc<HandshakeOutput>> {
    SESSIONS
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).and_then(|s| s.handshake_output.clone()))
}

pub fn local_endpoint_id_hex() -> Result<String, String> {
    Ok(bind_shared_endpoint()?.id().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_label_is_stable() {
        let a = parse_peer_endpoint_id("bobpeer12").unwrap();
        let b = parse_peer_endpoint_id("bobpeer12").unwrap();
        assert_eq!(a, b);
    }
}
