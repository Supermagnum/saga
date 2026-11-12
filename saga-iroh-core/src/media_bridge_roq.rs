//! Android PCM ↔ shared `saga-media` over iroh-roq.
//!
//! Capture/playback JNI threads stay allocation-free: they only push/pull PCM.
//! A tokio task owns [`MediaPipeline`] (Opus + SRTP/plain + RoQ datagrams).

use std::collections::{HashMap, VecDeque};
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

    let (capture_tx, mut capture_rx) = mpsc::channel::<Vec<i16>>(8);
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    let playback_ring = Arc::new(StdMutex::new(VecDeque::with_capacity(RING_FRAMES)));
    let pipeline = Arc::new(TokioMutex::new(Some(pipeline)));
    let pipeline_task = pipeline.clone();
    let playback_task = playback_ring.clone();
    let sid = session_id.to_string();

    crate::iroh_transport::runtime().spawn(async move {
        let _roq_keep = roq_session;
        let frame_dur =
            std::time::Duration::from_millis(1000 * FRAME_SAMPLES as u64 / SAMPLE_RATE as u64);
        let mut tick = tokio::time::interval(frame_dur);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        info!("[Saga Media] RoQ task running session=[{sid}]");
        let mut ticks: u64 = 0;
        loop {
            tokio::select! {
                _ = shutdown_rx.recv() => break,
                // Capture must NOT share this select as a peer branch: when the
                // audio thread feeds every 20 ms, capture_rx.recv() stays ready
                // and starves the tick (recv+playout), leaving the dump as
                // near-silence with a short speech burst (corr~0.5, STOI~0).
                _ = tick.tick() => {
                    let mut guard = pipeline_task.lock().await;
                    if let Some(p) = guard.as_mut() {
                        // Recv first so playout sees fresh packets this tick.
                        while tokio::time::timeout(
                            std::time::Duration::from_millis(0),
                            p.recv_one_datagram(),
                        )
                        .await
                        .is_ok()
                        {}
                        // Exactly one send per 20 ms tick. Leave backlog queued
                        // (do not drain/drop — that skips inject WAV samples).
                        match capture_rx.try_recv() {
                            Ok(mut pcm) => {
                                if let Err(e) = p.send_pcm_frame(&mut pcm) {
                                    warn!("[Saga Media] send: {e}");
                                }
                            }
                            Err(mpsc::error::TryRecvError::Empty) => {}
                            Err(mpsc::error::TryRecvError::Disconnected) => break,
                        }
                        let mut out = vec![0i16; FRAME_SAMPLES];
                        if p.playout_frame(&mut out).is_ok() {
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
                        // ~2 s @ 20 ms: surface SRTP/PLC health for RoQ bisect.
                        if ticks % 100 == 0 {
                            let s = p.counters.snapshot();
                            info!(
                                "[Saga Media] counters session=[{sid}] sent={} played={} plc={} \
                                 tamper={} wrong_mode={} blocked={}",
                                s.sent,
                                s.played,
                                s.plc_frames,
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
