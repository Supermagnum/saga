//! Android PCM ↔ shared `saga-media` over iroh-roq.
//!
//! Capture/playback JNI threads stay allocation-free: they only push/pull PCM.
//! A tokio task owns [`MediaPipeline`] (Opus + SRTP/plain + RoQ datagrams).

use std::collections::{HashMap, VecDeque};
#[cfg(feature = "mock-token")]
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Instant;

use log::{info, warn};
use saga_media::{
    CallRole, MediaCounters, MediaCountersSnapshot, MediaPipeline, SecurityResolution,
    FRAME_SAMPLES, SAMPLE_RATE,
};
use saga_session::HandshakeOutput;
use tokio::sync::{mpsc, Mutex as TokioMutex};

const FRAME_BYTES: usize = FRAME_SAMPLES * 2;
const RING_FRAMES: usize = 50;

/// Debug-only: re-inject decoded PCM into the local send path (echo RTT).
#[cfg(feature = "mock-token")]
static ECHO_MODE: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "mock-token")]
static ECHO_ARMED_LOG: AtomicBool = AtomicBool::new(false);

/// Debug-only: receive-side fault injector (drop % and optional delay jitter).
#[cfg(feature = "mock-token")]
static RX_FAULT_DROP_PCT: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "mock-token")]
static RX_FAULT_JITTER_MS: AtomicU32 = AtomicU32::new(0);
#[cfg(feature = "mock-token")]
static RX_FAULT_ARMED_LOG: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "mock-token")]
static RX_FAULT_RNG: StdMutex<u64> = StdMutex::new(0xC0FFEE_u64);

/// Enable/disable debug echo mode (never linked in release).
#[cfg(feature = "mock-token")]
pub fn set_echo_mode(enabled: bool) {
    ECHO_MODE.store(enabled, Ordering::Relaxed);
    ECHO_ARMED_LOG.store(false, Ordering::Relaxed);
    if enabled {
        // Marker string must appear only in debug .so (check-release-so.sh).
        info!("saga-echo-mode enabled");
    } else {
        info!("saga-echo-mode disabled");
    }
}

#[cfg(not(feature = "mock-token"))]
pub fn set_echo_mode(_enabled: bool) {}

/// Configure debug Rx fault injector (drop_pct 0–100, jitter_ms optional delay).
#[cfg(feature = "mock-token")]
pub fn set_rx_fault(drop_pct: u32, jitter_ms: u32) {
    RX_FAULT_DROP_PCT.store(drop_pct.min(100), Ordering::Relaxed);
    RX_FAULT_JITTER_MS.store(jitter_ms, Ordering::Relaxed);
    RX_FAULT_ARMED_LOG.store(false, Ordering::Relaxed);
    if drop_pct > 0 || jitter_ms > 0 {
        info!("saga-rx-fault drop_pct={drop_pct} jitter_ms={jitter_ms}");
    } else {
        info!("saga-rx-fault disabled");
    }
}

#[cfg(not(feature = "mock-token"))]
pub fn set_rx_fault(_drop_pct: u32, _jitter_ms: u32) {}

#[cfg(feature = "mock-token")]
fn rx_fault_rand_u32() -> u32 {
    let mut g = RX_FAULT_RNG.lock().unwrap_or_else(|e| e.into_inner());
    // xorshift64*
    let mut x = *g;
    if x == 0 {
        x = Instant::now().elapsed().as_nanos() as u64 | 1;
    }
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *g = x;
    (x % 100) as u32
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MediaMode {
    Srtp = 1,
    PlainRtp = 2,
}

struct MediaSession {
    mode: AtomicU8,
    muted: AtomicBool,
    held: AtomicBool,
    capture_tx: mpsc::Sender<Vec<i16>>,
    playback_ring: Arc<StdMutex<VecDeque<Vec<u8>>>>,
    counters: Arc<MediaCounters>,
    pipeline: Arc<TokioMutex<Option<MediaPipeline>>>,
    role: CallRole,
    last_rekey_us: StdMutex<u128>,
    _shutdown: mpsc::Sender<()>,
}

static MEDIA: std::sync::LazyLock<StdMutex<HashMap<String, MediaSession>>> =
    std::sync::LazyLock::new(|| StdMutex::new(HashMap::new()));

/// Start Opus+SRTP/plain media over existing RoQ flows for `session_id`.
pub fn start_media(
    session_id: &str,
    mode: MediaMode,
    role: CallRole,
    handshake: Option<&HandshakeOutput>,
    send: iroh_roq::SendFlow,
    recv: iroh_roq::ReceiveFlow,
    roq_session: iroh_roq::Session,
) -> bool {
    let resolution = match mode {
        MediaMode::Srtp => SecurityResolution::Encrypted,
        MediaMode::PlainRtp => SecurityResolution::NotEncrypted {
            prior_encrypted_history: false,
            user_continued_past_5a: true,
        },
    };
    let counters = Arc::new(MediaCounters::default());
    let pipeline =
        match MediaPipeline::start(resolution, handshake, role, send, recv, counters.clone()) {
            Ok(p) => p,
            Err(e) => {
                warn!("[Saga Media] start failed session=[{session_id}]: {e}");
                return false;
            }
        };

    // ≥ speech-clip length at 20 ms/frame so paced inject cannot overrun when
    // the RoQ tick briefly stalls (was 8 → dump noise on reverse path).
    let (capture_tx, mut capture_rx) = mpsc::channel::<Vec<i16>>(512);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let playback_ring = Arc::new(StdMutex::new(VecDeque::with_capacity(RING_FRAMES)));
    let pipeline = Arc::new(TokioMutex::new(Some(pipeline)));
    let pipeline_task = pipeline.clone();
    let playback_task = playback_ring.clone();
    let sid = session_id.to_string();
    let _role = role;

    crate::iroh_transport::runtime().spawn(async move {
        let _roq_keep = roq_session;
        let frame_dur =
            std::time::Duration::from_millis(1000 * FRAME_SAMPLES as u64 / SAMPLE_RATE as u64);
        let mut tick = tokio::time::interval(frame_dur);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        info!("[Saga Media] RoQ task running session=[{sid}]");
        let mut ticks: u64 = 0;
        #[cfg(feature = "mock-token")]
        let mut rx_delayed: VecDeque<(Instant, bytes::Bytes)> = VecDeque::new();
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                // Capture must NOT share this select as a peer branch: when the
                // audio thread feeds every 20 ms, capture_rx.recv() stays ready
                // and starves the tick (recv+playout).
                _ = tick.tick() => {
                    let mut guard = pipeline_task.lock().await;
                    if let Some(p) = guard.as_mut() {
                        // Recv first so playout sees fresh packets this tick.
                        // Cap drain: unbounded recv of a RoQ catch-up burst used
                        // to fill the 300 ms JB and force jump-forward (audio
                        // warp). Leave ≥1 slot and stop after a few datagrams;
                        // the rest wait for the next 20 ms tick after playout.
                        #[cfg(feature = "mock-token")]
                        {
                            let now = Instant::now();
                            while let Some((ready, _)) = rx_delayed.front() {
                                if *ready > now {
                                    break;
                                }
                                if p.jitter_len() + 1 >= p.jitter_cap() {
                                    break;
                                }
                                if let Some((_, bytes)) = rx_delayed.pop_front() {
                                    p.ingest_datagram(&bytes);
                                }
                            }
                        }
                        let mut drained = 0u32;
                        while drained < 5 {
                            if p.jitter_len() + 1 >= p.jitter_cap() {
                                break;
                            }
                            #[cfg(feature = "mock-token")]
                            let fault_on = RX_FAULT_DROP_PCT.load(Ordering::Relaxed) > 0
                                || RX_FAULT_JITTER_MS.load(Ordering::Relaxed) > 0;
                            #[cfg(not(feature = "mock-token"))]
                            let fault_on = false;
                            let got = if fault_on {
                                #[cfg(feature = "mock-token")]
                                {
                                    match tokio::time::timeout(
                                        std::time::Duration::from_millis(0),
                                        p.read_datagram_bytes(),
                                    )
                                    .await
                                    {
                                        Ok(Ok(bytes)) => {
                                            let drop_pct =
                                                RX_FAULT_DROP_PCT.load(Ordering::Relaxed);
                                            let jitter_ms =
                                                RX_FAULT_JITTER_MS.load(Ordering::Relaxed);
                                            if !RX_FAULT_ARMED_LOG.swap(true, Ordering::Relaxed) {
                                                info!(
                                                    "saga-rx-fault active drop_pct={drop_pct} \
                                                     jitter_ms={jitter_ms}"
                                                );
                                            }
                                            if drop_pct > 0 && rx_fault_rand_u32() < drop_pct {
                                                // Intentional drop — do not ingest.
                                            } else if jitter_ms > 0 {
                                                let d = rx_fault_rand_u32() % (jitter_ms + 1);
                                                if d == 0 {
                                                    p.ingest_datagram(&bytes);
                                                } else {
                                                    rx_delayed.push_back((
                                                        Instant::now()
                                                            + std::time::Duration::from_millis(
                                                                d as u64,
                                                            ),
                                                        bytes,
                                                    ));
                                                }
                                            } else {
                                                p.ingest_datagram(&bytes);
                                            }
                                            true
                                        }
                                        _ => false,
                                    }
                                }
                                #[cfg(not(feature = "mock-token"))]
                                {
                                    false
                                }
                            } else if tokio::time::timeout(
                                std::time::Duration::from_millis(0),
                                p.recv_one_datagram(),
                            )
                            .await
                            .is_ok()
                            {
                                true
                            } else {
                                false
                            };
                            if !got {
                                break;
                            }
                            drained = drained.saturating_add(1);
                        }
                        #[cfg(feature = "mock-token")]
                        let echo_on = ECHO_MODE.load(Ordering::Relaxed);
                        #[cfg(not(feature = "mock-token"))]
                        let echo_on = false;
                        // Exactly one send per 20 ms tick.
                        // Echo mode replaces mic capture with decoded PCM.
                        if !echo_on {
                            match capture_rx.try_recv() {
                                Ok(mut pcm) => {
                                    if let Err(e) = p.send_pcm_frame(&mut pcm) {
                                        warn!("[Saga Media] send: {e}");
                                    }
                                }
                                Err(mpsc::error::TryRecvError::Empty) => {}
                                Err(mpsc::error::TryRecvError::Disconnected) => break,
                            }
                        } else {
                            // Drain mic so the channel does not back up under echo.
                            while capture_rx.try_recv().is_ok() {}
                        }
                        let mut out = vec![0i16; FRAME_SAMPLES];
                        if p.playout_frame(&mut out).is_ok() {
                            #[cfg(feature = "mock-token")]
                            if echo_on {
                                if !ECHO_ARMED_LOG.swap(true, Ordering::Relaxed) {
                                    info!("saga-echo-mode looping decoded PCM to send");
                                }
                                let mut echo = out.clone();
                                if let Err(e) = p.send_pcm_frame(&mut echo) {
                                    warn!("[Saga Media] echo send: {e}");
                                }
                            }
                            let mut bytes = vec![0u8; FRAME_BYTES];
                            for (i, s) in out.iter().enumerate() {
                                let b = s.to_le_bytes();
                                bytes[i * 2] = b[0];
                                bytes[i * 2 + 1] = b[1];
                            }
                            if let Ok(mut ring) = playback_task.lock() {
                                if ring.len() >= RING_FRAMES {
                                    ring.pop_front();
                                }
                                ring.push_back(bytes);
                            }
                        }
                        ticks = ticks.wrapping_add(1);
                        if ticks.is_multiple_of(100) {
                            let s = p.counters.snapshot();
                            info!(
                                "[Saga Media] counters session=[{sid}] sent={} played={} plc={} \
                                 late={} lost={} jb_med_ms={} jb_max_ms={} jb_target_ms={} \
                                 tamper={} wrong_mode={} blocked={}",
                                s.sent,
                                s.played,
                                s.plc_frames,
                                s.late_arrivals,
                                s.lost_packets,
                                s.jb_depth_median_ms,
                                s.jb_depth_max_ms,
                                s.jb_target_ms,
                                s.tamper_or_auth_fail,
                                s.wrong_media_mode,
                                s.datagram_blocked_drops
                            );
                        }
                    }
                }
            }
        }
        info!("[Saga Media] RoQ task stopped session=[{sid}]");
        drop(_roq_keep);
    });

    let session = MediaSession {
        mode: AtomicU8::new(mode as u8),
        muted: AtomicBool::new(false),
        held: AtomicBool::new(false),
        capture_tx,
        playback_ring,
        counters,
        pipeline,
        role,
        last_rekey_us: StdMutex::new(0),
        _shutdown: shutdown_tx,
    };

    if let Ok(mut map) = MEDIA.lock() {
        map.insert(session_id.to_string(), session);
        info!(
            "[Saga Media] start session=[{session_id}] mode={} (RoQ/saga-media)",
            mode as u8 as i32
        );
        true
    } else {
        false
    }
}

pub fn stop_media(session_id: &str) {
    if let Ok(mut map) = MEDIA.lock() {
        if let Some(s) = map.remove(session_id) {
            let _ = s._shutdown.try_send(());
            // Drop pipeline on the runtime so we don't block JNI.
            let pipe = s.pipeline.clone();
            crate::iroh_transport::runtime().spawn(async move {
                let mut g = pipe.lock().await;
                *g = None;
            });
            info!("[Saga Media] stop session=[{session_id}]");
        }
    }
}

pub fn set_muted(session_id: &str, muted: bool) {
    if let Ok(map) = MEDIA.lock() {
        if let Some(s) = map.get(session_id) {
            s.muted.store(muted, Ordering::Relaxed);
            info!("[Saga Media] muted={muted} session=[{session_id}]");
        }
    }
}

/// Mute/unmute every live RoQ media session (E2E / debug).
pub fn set_muted_all(muted: bool) {
    if let Ok(map) = MEDIA.lock() {
        for (sid, s) in map.iter() {
            s.muted.store(muted, Ordering::Relaxed);
            info!("[Saga Media] muted={muted} session=[{sid}]");
        }
    }
}

pub fn set_held(session_id: &str, held: bool) {
    if let Ok(map) = MEDIA.lock() {
        if let Some(s) = map.get(session_id) {
            s.held.store(held, Ordering::Relaxed);
            info!("[Saga Media] held={held} session=[{session_id}]");
        }
    }
}

pub fn push_capture(session_id: &str, pcm: &[u8]) {
    if pcm.len() < FRAME_BYTES {
        return;
    }
    let Ok(map) = MEDIA.lock() else { return };
    let Some(s) = map.get(session_id) else {
        return;
    };
    if s.muted.load(Ordering::Relaxed) || s.held.load(Ordering::Relaxed) {
        return;
    }
    let mut frame = vec![0i16; FRAME_SAMPLES];
    for i in 0..FRAME_SAMPLES {
        frame[i] = i16::from_le_bytes([pcm[i * 2], pcm[i * 2 + 1]]);
    }
    match s.capture_tx.try_send(frame) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Full(_)) => {
            s.counters
                .datagram_blocked_drops
                .fetch_add(1, Ordering::Relaxed);
        }
        Err(mpsc::error::TrySendError::Closed(_)) => {}
    }
}

pub fn pull_playback(session_id: &str, out: &mut [u8]) -> usize {
    if out.len() < FRAME_BYTES {
        return 0;
    }
    let Ok(map) = MEDIA.lock() else { return 0 };
    let Some(s) = map.get(session_id) else {
        return 0;
    };
    if s.held.load(Ordering::Relaxed) {
        out[..FRAME_BYTES].fill(0);
        return FRAME_BYTES;
    }
    let Ok(mut ring) = s.playback_ring.lock() else {
        return 0;
    };
    if let Some(frame) = ring.pop_front() {
        out[..FRAME_BYTES].copy_from_slice(&frame[..FRAME_BYTES]);
        FRAME_BYTES
    } else {
        out[..FRAME_BYTES].fill(0);
        FRAME_BYTES
    }
}

/// §5b re-key: swap SRTP contexts on the live pipeline.
pub fn rekey_media(session_id: &str, output: &HandshakeOutput) -> i64 {
    let Ok(map) = MEDIA.lock() else { return -1 };
    let Some(s) = map.get(session_id) else {
        return -1;
    };
    if s.mode.load(Ordering::Relaxed) != MediaMode::Srtp as u8 {
        return -1;
    }
    let pipeline = s.pipeline.clone();
    let role = s.role;
    // `rekey_srtp` only needs the output briefly; block_on on the Iroh runtime.
    let us = crate::iroh_transport::runtime().block_on(async {
        let mut guard = pipeline.lock().await;
        let Some(p) = guard.as_mut() else {
            return -1_i64;
        };
        match p.rekey_srtp(output, role) {
            Ok(us) => {
                info!("[Saga Media] rekey_swap_us={us}");
                us as i64
            }
            Err(e) => {
                warn!("[Saga Media] rekey failed: {e}");
                -1
            }
        }
    });
    if us >= 0 {
        if let Ok(mut last) = s.last_rekey_us.lock() {
            *last = us as u128;
        }
        info!("[Saga Media] rekey_swap_us={us} session=[{session_id}]");
    }
    let _ = Instant::now();
    us
}

/// True once a RoQ media session is running (replaces synthetic media probe).
pub fn media_running(session_id: &str) -> bool {
    MEDIA
        .lock()
        .ok()
        .map(|m| m.contains_key(session_id))
        .unwrap_or(false)
}

#[allow(dead_code)]
pub fn counters_snapshot(session_id: &str) -> Option<MediaCountersSnapshot> {
    let map = MEDIA.lock().ok()?;
    let s = map.get(session_id)?;
    Some(s.counters.snapshot())
}
