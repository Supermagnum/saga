//! Real Iroh transport (feature `iroh-transport`).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex, OnceLock};
use std::time::Duration;

use blake3;
use iroh::endpoint::Connection;
use iroh::tls::CaTlsConfig;
use iroh::{Endpoint, EndpointAddr, PublicKey, RelayMap, RelayMode, RelayUrl, SecretKey, endpoint::presets};
use log::{error, info, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::runtime::Runtime;
use tokio::sync::oneshot;

#[cfg(feature = "mock-token")]
use mock_token::{
    HandshakeOutcome, MockIdentity, SYNTHETIC_PLAINTEXT, decrypt_frame, encrypt_frame,
    run_initiator, run_responder,
};

pub const SAGA_VOICE_ALPN: &[u8] = b"saga/voice/1";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(45);
const RELAY_ONLINE_WAIT: Duration = Duration::from_secs(60);
const CONNECT_RETRY_SLEEP: Duration = Duration::from_secs(5);
const MAX_CONNECT_ATTEMPTS: u32 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakePoll {
    Pending = 0,
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
    session_key: Option<[u8; 32]>,
    media_round_trip_ok: bool,
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
static DEV_IDENTITY_LABEL: Mutex<Option<String>> = Mutex::new(None);
static RELAY_URL_OVERRIDE: Mutex<Option<String>> = Mutex::new(None);
static FORCE_HANDSHAKE_FAIL: AtomicBool = AtomicBool::new(false);
static RELAY_POLL: AtomicU8 = AtomicU8::new(RelayPoll::Pending as u8);

fn runtime() -> &'static Runtime {
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

pub fn poll_relay_ready() -> RelayPoll {
    if SHARED_ENDPOINT.get().is_none() {
        return RelayPoll::Pending;
    }
    match RELAY_POLL.load(Ordering::SeqCst) {
        1 => RelayPoll::Ready,
        2 => RelayPoll::Failed,
        _ => RelayPoll::Pending,
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
    Ok(builder
        .relay_mode(RelayMode::Custom(relay_map))
        .ca_tls_config(CaTlsConfig::insecure_skip_verify()))
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

fn force_handshake_fail() -> bool {
    FORCE_HANDSHAKE_FAIL.load(Ordering::SeqCst)
}

fn local_identity_label() -> String {
    DEV_IDENTITY_LABEL
        .lock()
        .ok()
        .and_then(|g| g.clone())
        .unwrap_or_else(|| "15550100011".to_string())
}

fn dev_secret_key(peer_label: &str) -> SecretKey {
    let material = format!("saga-dev-v1:{peer_label}");
    let hash = blake3::hash(material.as_bytes());
    let bytes: [u8; 32] = *hash.as_bytes();
    SecretKey::from_bytes(&bytes)
}

pub fn parse_peer_endpoint_id(peer_id: &str) -> Result<PublicKey, String> {
    if let Ok(pk) = PublicKey::from_str(peer_id) {
        return Ok(pk);
    }
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

fn dev_label_for_endpoint(remote: &PublicKey) -> Option<String> {
    for label in ["15550100010", "15550100011", "15550100012"] {
        if dev_secret_key(label).public() == *remote {
            return Some(label.to_string());
        }
    }
    None
}

fn inbound_session_id(lookup_key: &str) -> String {
    let safe: String = lookup_key
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("inbound-{safe}-{}", std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0))
}

async fn handle_inbound_connection(conn: Connection) {
    let remote_pk = conn.remote_id();
    let remote_id_str = remote_pk.to_string();
    let lookup_key = dev_label_for_endpoint(&remote_pk).unwrap_or_else(|| remote_id_str.clone());
    let session_id = inbound_session_id(&lookup_key);

    info!(
        "[Saga Iroh Listen] inbound accepted remote=[{remote_id_str}] lookup=[{lookup_key}] session=[{session_id}]"
    );

    #[cfg(feature = "iroh-transport")]
    crate::bridge_notify_incoming_call(&session_id, &lookup_key, &remote_id_str);

    let mut handshake = HandshakePoll::Pending;
    let mut session_key = None;
    let mut media_round_trip_ok = false;

    #[cfg(feature = "mock-token")]
    {
        match conn.accept_bi().await {
            Ok((mut send, mut recv)) => {
                let identity = MockIdentity::from_label(&local_identity_label());
                let outcome =
                    run_responder(&identity, force_handshake_fail(), &mut recv, &mut send).await;
                match outcome {
                    HandshakeOutcome::Encrypted { session_key: key } => {
                        handshake = HandshakePoll::Encrypted;
                        session_key = Some(key);
                        media_round_trip_ok =
                            read_media_round_trip(&mut recv, &key, "responder").await.unwrap_or(false);
                        info!(
                            "[Saga Media Round-Trip] responder decrypted_ok={media_round_trip_ok}"
                        );
                    }
                    HandshakeOutcome::Failed { reason } => {
                        handshake = HandshakePoll::Failed;
                        warn!("inbound mock-token handshake failed: {reason}");
                    }
                }
            }
            Err(e) => {
                handshake = HandshakePoll::Failed;
                warn!("inbound accept_bi failed: {e}");
            }
        }
    }

    #[cfg(not(feature = "mock-token"))]
    {
        handshake = HandshakePoll::Encrypted;
    }

    if let Ok(mut sessions) = SESSIONS.lock() {
        sessions.insert(
            session_id,
            Session {
                connection: conn,
                handshake,
                session_key,
                media_round_trip_ok,
            },
        );
        info!("inbound session registered lookup=[{lookup_key}] handshake={handshake:?}");
    }
}

#[cfg(feature = "mock-token")]
async fn read_media_round_trip<R: AsyncReadExt + Unpin>(
    reader: &mut R,
    key: &[u8; 32],
    role: &str,
) -> Result<bool, String> {
    let mut len_buf = [0u8; 4];
    reader
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("read media len: {e}"))?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut frame = vec![0u8; len];
    reader
        .read_exact(&mut frame)
        .await
        .map_err(|e| format!("read media frame: {e}"))?;
    let plain = decrypt_frame(key, &frame)?;
    let ok = plain == SYNTHETIC_PLAINTEXT;
    info!(
        "[Saga Media Round-Trip] {role} decrypted_ok={ok} plaintext_match={ok}"
    );
    Ok(ok)
}

#[cfg(feature = "mock-token")]
async fn send_media_round_trip<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    key: &[u8; 32],
) -> Result<bool, String> {
    let frame = encrypt_frame(key, SYNTHETIC_PLAINTEXT);
    let len = (frame.len() as u32).to_be_bytes();
    writer
        .write_all(&len)
        .await
        .map_err(|e| format!("write media len: {e}"))?;
    writer
        .write_all(&frame)
        .await
        .map_err(|e| format!("write media frame: {e}"))?;
    writer
        .flush()
        .await
        .map_err(|e| format!("flush media: {e}"))?;
    Ok(true)
}

async fn bind_endpoint_async() -> Result<EndpointHolder, String> {
    RELAY_POLL.store(RelayPoll::Pending as u8, Ordering::SeqCst);
    let dev_label = DEV_IDENTITY_LABEL.lock().ok().and_then(|g| g.clone());
    let mut builder = Endpoint::builder(presets::N0).alpns(vec![SAGA_VOICE_ALPN.to_vec()]);
    builder = apply_relay_builder(builder)?;
    if let Some(label) = dev_label {
        let sk = dev_secret_key(&label);
        info!(
            "binding iroh endpoint with dev identity label [{label}] id=[{}]",
            sk.public()
        );
        builder = builder.secret_key(sk);
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
                                match connecting.await {
                                    Ok(conn) => handle_inbound_connection(conn).await,
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

async fn connect_async(peer_id: &str, session_id: &str) -> Result<(), String> {
    let remote = parse_peer_endpoint_id(peer_id)?;
    let endpoint = bind_shared_endpoint()?;
    if tokio::time::timeout(RELAY_ONLINE_WAIT, endpoint.online())
        .await
        .is_err()
    {
        warn!("initiator relay online wait timed out before connect to [{peer_id}] — trying anyway");
    } else {
        info!("initiator relay online before connect to [{peer_id}]");
    }

    let addr = EndpointAddr::from(remote);
    let mut connection = None;
    let mut last_err = String::new();
    for attempt in 1..=MAX_CONNECT_ATTEMPTS {
        match tokio::time::timeout(
            CONNECT_TIMEOUT,
            endpoint.connect(addr.clone(), SAGA_VOICE_ALPN),
        )
        .await
        {
            Ok(Ok(conn)) => {
                connection = Some(conn);
                break;
            }
            Ok(Err(e)) => {
                last_err = format!("{e}");
                if attempt < MAX_CONNECT_ATTEMPTS {
                    warn!(
                        "iroh connect attempt {attempt}/{MAX_CONNECT_ATTEMPTS} to [{peer_id}]: {last_err} — retrying"
                    );
                    tokio::time::sleep(CONNECT_RETRY_SLEEP).await;
                    continue;
                }
                return Err(format!("iroh connect error: {e}"));
            }
            Err(_) => {
                last_err = format!("timed out after {}s", CONNECT_TIMEOUT.as_secs());
                if attempt < MAX_CONNECT_ATTEMPTS {
                    warn!(
                        "iroh connect attempt {attempt}/{MAX_CONNECT_ATTEMPTS} to [{peer_id}] timed out — retrying"
                    );
                    tokio::time::sleep(CONNECT_RETRY_SLEEP).await;
                    continue;
                }
                return Err(format!(
                    "iroh connect timed out after {} attempts ({}s each)",
                    MAX_CONNECT_ATTEMPTS,
                    CONNECT_TIMEOUT.as_secs()
                ));
            }
        }
    }
    let connection = connection.ok_or_else(|| {
        format!("iroh connect failed after retries: {last_err}")
    })?;

    let mut handshake = HandshakePoll::Encrypted;
    let mut session_key = None;
    let mut media_round_trip_ok = false;

    #[cfg(feature = "mock-token")]
    {
        let (mut send, mut recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("iroh open_bi failed: {e}"))?;
        let identity = MockIdentity::from_label(&local_identity_label());
        let outcome = run_initiator(
            &identity,
            peer_id,
            force_handshake_fail(),
            &mut recv,
            &mut send,
        )
        .await;
        match outcome {
            HandshakeOutcome::Encrypted { session_key: key } => {
                handshake = HandshakePoll::Encrypted;
                session_key = Some(key);
                media_round_trip_ok = send_media_round_trip(&mut send, &key).await.unwrap_or(false);
                let _ = send.finish();
            }
            HandshakeOutcome::Failed { reason } => {
                handshake = HandshakePoll::Failed;
                warn!("mock-token handshake failed: {reason}");
            }
        }
    }

    #[cfg(not(feature = "mock-token"))]
    {
        let (mut send, _recv) = connection
            .open_bi()
            .await
            .map_err(|e| format!("iroh open_bi failed: {e}"))?;
        send.write_all(b"saga-connect-probe")
            .await
            .map_err(|e| format!("iroh probe write: {e}"))?;
        let _ = send.finish();
    }

    info!("iroh connected to [{}] handshake={handshake:?}", connection.remote_id());

    SESSIONS
        .lock()
        .map_err(|_| "session map poisoned".to_string())?
        .insert(
            session_id.to_string(),
            Session {
                connection,
                handshake,
                session_key,
                media_round_trip_ok,
            },
        );

    Ok(())
}

pub fn connect(peer_id: &str, session_id: &str) -> Result<(), String> {
    if peer_id.to_ascii_lowercase().contains("fail") {
        return Err("native connect failure (peer id contains 'fail')".into());
    }
    runtime().block_on(connect_async(peer_id, session_id)).map_err(|e| {
        error!("connect_async failed: {e}");
        e
    })
}

pub fn disconnect(session_id: &str) {
    if let Some(session) = SESSIONS.lock().ok().and_then(|mut m| m.remove(session_id)) {
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

pub fn session_key_hex(session_id: &str) -> Option<String> {
    SESSIONS.lock().ok().and_then(|m| {
        m.get(session_id).and_then(|s| {
            s.session_key.map(|key| {
                key.iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            })
        })
    })
}

pub fn media_round_trip_ok(session_id: &str) -> bool {
    SESSIONS
        .lock()
        .ok()
        .and_then(|m| m.get(session_id).map(|s| s.media_round_trip_ok))
        .unwrap_or(false)
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
