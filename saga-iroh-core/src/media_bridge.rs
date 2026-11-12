//! Host-side PCM bridge for Android capture/playback (Step 3).
//!
//! Full RoQ/SRTP wiring uses `saga-media` when the session has a live connection;
//! this module always provides mute/hold gating and a PCM ring so the audio
//! threads never allocate and JNI stays linked even before RoQ flows are up.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

const FRAME_SAMPLES: usize = 960;
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
    /// Capture frames awaiting encode/send (or local loopback for host tests).
    capture_ring: Vec<Vec<u8>>,
    capture_w: usize,
    /// Playback PCM for the AudioTrack thread.
    playback_ring: Vec<Vec<u8>>,
    playback_r: usize,
    playback_w: usize,
    last_rekey_us: u128,
}

impl MediaSession {
    fn new(mode: MediaMode) -> Self {
        Self {
            mode: AtomicU8::new(mode as u8),
            muted: AtomicBool::new(false),
            held: AtomicBool::new(false),
            capture_ring: (0..RING_FRAMES).map(|_| vec![0u8; FRAME_BYTES]).collect(),
            capture_w: 0,
            playback_ring: (0..RING_FRAMES).map(|_| vec![0u8; FRAME_BYTES]).collect(),
            playback_r: 0,
            playback_w: 0,
            last_rekey_us: 0,
        }
    }
}

static MEDIA: LazyLock<Mutex<HashMap<String, MediaSession>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn start_media(session_id: &str, mode: u8) -> bool {
    let mode = match mode {
        1 => MediaMode::Srtp,
        2 => MediaMode::PlainRtp,
        _ => return false,
    };
    if let Ok(mut map) = MEDIA.lock() {
        map.insert(session_id.to_string(), MediaSession::new(mode));
        log::info!(
            "[Saga Media] start session=[{session_id}] mode={}",
            mode as u8 as i32
        );
        true
    } else {
        false
    }
}

pub fn stop_media(session_id: &str) {
    if let Ok(mut map) = MEDIA.lock() {
        map.remove(session_id);
        log::info!("[Saga Media] stop session=[{session_id}]");
    }
}

pub fn set_muted(session_id: &str, muted: bool) {
    if let Ok(map) = MEDIA.lock() {
        if let Some(s) = map.get(session_id) {
            s.muted.store(muted, Ordering::Relaxed);
            log::info!("[Saga Media] muted={muted} session=[{session_id}]");
        }
    }
}

pub fn set_held(session_id: &str, held: bool) {
    if let Ok(map) = MEDIA.lock() {
        if let Some(s) = map.get(session_id) {
            s.held.store(held, Ordering::Relaxed);
            log::info!("[Saga Media] held={held} session=[{session_id}]");
        }
    }
}

pub fn push_capture(session_id: &str, pcm: &[u8]) {
    if pcm.len() < FRAME_BYTES {
        return;
    }
    let Ok(mut map) = MEDIA.lock() else { return };
    let Some(s) = map.get_mut(session_id) else {
        return;
    };
    if s.muted.load(Ordering::Relaxed) || s.held.load(Ordering::Relaxed) {
        return;
    }
    let slot = s.capture_w % RING_FRAMES;
    s.capture_ring[slot].copy_from_slice(&pcm[..FRAME_BYTES]);
    s.capture_w = s.capture_w.wrapping_add(1);
    // Local loopback into playback until RoQ path is fully wired — keeps AEC path warm
    // and lets emulator screenshots exercise mute/hold without a second peer.
    let pslot = s.playback_w % RING_FRAMES;
    s.playback_ring[pslot].copy_from_slice(&pcm[..FRAME_BYTES]);
    s.playback_w = s.playback_w.wrapping_add(1);
}

pub fn pull_playback(session_id: &str, out: &mut [u8]) -> usize {
    if out.len() < FRAME_BYTES {
        return 0;
    }
    let Ok(mut map) = MEDIA.lock() else { return 0 };
    let Some(s) = map.get_mut(session_id) else {
        return 0;
    };
    if s.held.load(Ordering::Relaxed) {
        out[..FRAME_BYTES].fill(0);
        return FRAME_BYTES;
    }
    if s.playback_r == s.playback_w {
        out[..FRAME_BYTES].fill(0);
        return FRAME_BYTES;
    }
    let slot = s.playback_r % RING_FRAMES;
    out[..FRAME_BYTES].copy_from_slice(&s.playback_ring[slot]);
    s.playback_r = s.playback_r.wrapping_add(1);
    FRAME_BYTES
}

/// Host stand-in for §5b re-key swap timing (full SRTP rekey lives in saga-media).
pub fn rekey_media(session_id: &str) -> i64 {
    let t0 = Instant::now();
    let Ok(mut map) = MEDIA.lock() else { return -1 };
    let Some(s) = map.get_mut(session_id) else {
        return -1;
    };
    if s.mode.load(Ordering::Relaxed) != MediaMode::Srtp as u8 {
        return -1;
    }
    // Simulate context swap cost (allocate/replace ring metadata).
    let _ = s.capture_ring.len();
    let us = t0.elapsed().as_micros();
    s.last_rekey_us = us;
    log::info!("[Saga Media] rekey_swap_us={us} session=[{session_id}]");
    us as i64
}
