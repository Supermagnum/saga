//! Adaptive jitter buffer: playout delay separate from loss detection.
//!
//! - **Playout target** starts at ~50 ms (40–60 ms band), tracks a high
//!   percentile of inter-arrival deviation + a small margin, grows on late
//!   arrivals / underruns, shrinks slowly when stable. Hard cap [`CAP_MS`].
//! - **Loss**: a packet is lost only when its playout deadline passes without
//!   the payload present; then the caller runs PLC/FEC. There is no separate
//!   multi-hundred-ms "reorder wait" that pads call latency.
//! - Shrink catch-up drops excess buffered frames only when depth sits above
//!   the target (one frame at a time) to avoid audible jumps from time-warp.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use crate::codec::FRAME_MS;

/// Initial / steady-state target floor (ms).
pub const TARGET_MS_MIN: u32 = 40;
/// Preferred steady-state ceiling before jitter forces growth (ms).
pub const TARGET_MS_HIGH: u32 = 60;
/// Absolute playout depth cap (ms).
pub const CAP_MS: u32 = 300;

const FRAME_DUR: Duration = Duration::from_millis(FRAME_MS as u64);
/// Margin added on top of measured jitter percentile (ms).
const JITTER_MARGIN_MS: u32 = 20;
/// Ring of inter-arrival deviations used for the percentile estimate.
const JITTER_HIST: usize = 64;
/// Hits without late/loss before shrinking target by one frame.
const SHRINK_AFTER_HITS: u32 = 50; // ~1 s at 20 ms
/// Percentile of |inter-arrival − 20 ms| used as jitter estimate (×100).
const JITTER_PERCENTILE: usize = 90;

/// Packet held in the jitter buffer.
#[derive(Debug, Clone)]
pub struct BufferedPacket {
    pub seq: u16,
    pub payload: Vec<u8>,
}

/// Result of a playout tick against the jitter buffer.
#[derive(Debug)]
pub enum PopOutcome {
    /// Next packet ready to decode.
    Packet(BufferedPacket),
    /// Priming / early: silence, do not advance the playout cursor.
    Wait,
    /// Gap past its playout deadline; caller should conceal. Sequence advanced.
    Lost,
}

/// Clock abstraction so unit tests can drive virtual time.
pub trait Clock: Clone {
    fn now(&self) -> Instant;
}

/// Wall-clock Instant source.
#[derive(Debug, Clone, Default)]
pub struct WallClock;

impl Clock for WallClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Manual clock for deterministic JB tests.
#[derive(Debug, Clone)]
#[allow(dead_code)] // constructed from unit tests; kept public for harnesses
pub struct ManualClock {
    origin: Instant,
    offset: Duration,
}

impl Default for ManualClock {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            offset: Duration::ZERO,
        }
    }
}

impl ManualClock {
    #[allow(dead_code)]
    pub fn advance(&mut self, d: Duration) {
        self.offset = self.offset.saturating_add(d);
    }

    #[allow(dead_code)]
    pub fn advance_ms(&mut self, ms: u64) {
        self.advance(Duration::from_millis(ms));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.origin + self.offset
    }
}

#[derive(Debug)]
pub struct JitterBuffer<C: Clock = WallClock> {
    packets: BTreeMap<u16, Vec<u8>>,
    /// First seq observed (anchors the playout timeline).
    first_seq: Option<u16>,
    /// Next sequence expected for playout.
    next_seq: Option<u16>,
    /// True after the first successful pop / PLC advance.
    playout_started: bool,
    /// Wall time when playout of `first_seq` is due.
    playout_anchor: Option<Instant>,
    /// Current delay target in frames (adaptive).
    target_frames: u32,
    max_packets: usize,
    clock: C,
    last_arrival: Option<Instant>,
    /// Absolute inter-arrival deviation samples (ms).
    jitter_samples: Vec<u32>,
    hits_since_adapt: u32,
    late_or_loss_recent: bool,
}

impl Default for JitterBuffer<WallClock> {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterBuffer<WallClock> {
    pub fn new() -> Self {
        Self::with_clock(WallClock)
    }
}

impl<C: Clock> JitterBuffer<C> {
    pub fn with_clock(clock: C) -> Self {
        let target_frames = ((TARGET_MS_MIN + TARGET_MS_HIGH) / 2) / FRAME_MS;
        let max_packets = (CAP_MS / FRAME_MS) as usize;
        Self {
            packets: BTreeMap::new(),
            first_seq: None,
            next_seq: None,
            playout_started: false,
            playout_anchor: None,
            target_frames: target_frames.max(2), // 40–60 ms → 2–3 frames
            max_packets,
            clock,
            last_arrival: None,
            jitter_samples: Vec::with_capacity(JITTER_HIST),
            hits_since_adapt: 0,
            late_or_loss_recent: false,
        }
    }

    pub fn clock_mut(&mut self) -> &mut C {
        &mut self.clock
    }

    pub fn len(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn max_packets(&self) -> usize {
        self.max_packets
    }

    pub fn playout_started(&self) -> bool {
        self.playout_started
    }

    pub fn awaiting_first_packet(&self) -> bool {
        self.next_seq.is_none()
    }

    pub fn depth(&self) -> usize {
        self.packets.len()
    }

    /// Current adaptive playout target in milliseconds.
    pub fn target_ms(&self) -> u32 {
        self.target_frames * FRAME_MS
    }

    pub fn target_frames(&self) -> u32 {
        self.target_frames
    }

    /// Insert a packet. Returns whether it was accepted (false = late / already played).
    pub fn push(&mut self, seq: u16, payload: Vec<u8>) -> bool {
        let now = self.clock.now();
        self.note_arrival(now);

        match self.next_seq {
            None => {
                self.first_seq = Some(seq);
                self.next_seq = Some(seq);
                // Playout of first_seq is due after the current target delay.
                self.playout_anchor =
                    Some(now + Duration::from_millis(u64::from(self.target_ms())));
            }
            Some(next) => {
                let delta = next.wrapping_sub(seq);
                if !self.playout_started && delta > 0 && delta < 0x100 {
                    self.first_seq = Some(seq);
                    self.next_seq = Some(seq);
                    self.playout_anchor =
                        Some(now + Duration::from_millis(u64::from(self.target_ms())));
                } else if self.already_played(seq, next) {
                    self.late_or_loss_recent = true;
                    self.grow_from_late();
                    return false;
                }
            }
        }
        self.packets.insert(seq, payload);
        self.enforce_cap();
        self.recompute_target_from_jitter();
        true
    }

    fn note_arrival(&mut self, now: Instant) {
        if let Some(prev) = self.last_arrival {
            let ia = now.saturating_duration_since(prev);
            let expected = FRAME_DUR;
            let dev = ia.abs_diff(expected);
            let dev_ms = u32::try_from(dev.as_millis()).unwrap_or(u32::MAX);
            if self.jitter_samples.len() >= JITTER_HIST {
                self.jitter_samples.remove(0);
            }
            self.jitter_samples.push(dev_ms.min(CAP_MS));
        }
        self.last_arrival = Some(now);
    }

    fn enforce_cap(&mut self) {
        // Over cap: drop nearest-to-playout and jump the playout cursor past it.
        while self.packets.len() > self.max_packets {
            let Some((&k, _)) = self.packets.iter().next() else {
                break;
            };
            self.packets.remove(&k);
            if let Some(next) = self.next_seq {
                let ahead = k.wrapping_sub(next);
                if ahead < 0x8000 {
                    self.next_seq = Some(k.wrapping_add(1));
                    self.bump_anchor_for_jump(next, k.wrapping_add(1));
                }
            }
        }
    }

    fn bump_anchor_for_jump(&mut self, from: u16, to: u16) {
        let Some(anchor) = self.playout_anchor else {
            return;
        };
        let skipped = to.wrapping_sub(from);
        if skipped > 0 && skipped < 0x8000 {
            self.playout_anchor = Some(anchor + FRAME_DUR.saturating_mul(u32::from(skipped)));
        }
    }

    fn already_played(&self, seq: u16, next: u16) -> bool {
        let delta = next.wrapping_sub(seq);
        delta > 0 && delta < 0x8000
    }

    /// Deadline for `next_seq` based on the playout timeline.
    fn deadline_for(&self, seq: u16) -> Option<Instant> {
        let anchor = self.playout_anchor?;
        let first = self.first_seq?;
        let offset = seq.wrapping_sub(first);
        if offset >= 0x8000 {
            return Some(anchor);
        }
        Some(anchor + FRAME_DUR.saturating_mul(u32::from(offset)))
    }

    /// Pop next packet, or indicate wait / loss.
    pub fn pop_ready(&mut self) -> PopOutcome {
        let Some(next) = self.next_seq else {
            return PopOutcome::Wait;
        };
        let now = self.clock.now();

        // Priming: hold until the first-packet deadline (target delay).
        if !self.playout_started {
            if let Some(deadline) = self.playout_anchor {
                if now < deadline && !self.packets.contains_key(&next) {
                    return PopOutcome::Wait;
                }
                if now < deadline && self.depth() < self.target_frames as usize {
                    // Prefer filling toward target when packets arrive early.
                    if self.have_contiguous_from(next) < self.target_frames {
                        return PopOutcome::Wait;
                    }
                }
            }
        }

        if let Some(payload) = self.packets.remove(&next) {
            self.playout_started = true;
            self.next_seq = Some(next.wrapping_add(1));
            self.hits_since_adapt = self.hits_since_adapt.saturating_add(1);
            self.maybe_shrink();
            self.maybe_catch_up_depth();
            return PopOutcome::Packet(BufferedPacket { seq: next, payload });
        }

        // Empty buffer / network stall: silence without advancing the cursor so
        // a late-but-still-useful packet is not marked already_played. Grow the
        // target so the next burst is absorbed. This is underrun, not loss.
        if self.packets.is_empty() {
            // Do not inflate target before any packet has been accepted (caller
            // may send before callee JB exists).
            if self.playout_started && self.first_seq.is_some() {
                self.grow_from_late();
            }
            return PopOutcome::Wait;
        }

        // Hole: later packets exist but `next` is missing. Lost only once the
        // playout deadline for `next` has passed → PLC/FEC.
        let Some(deadline) = self.deadline_for(next) else {
            return PopOutcome::Wait;
        };
        if now < deadline {
            return PopOutcome::Wait;
        }

        // Deadline passed — declare loss. Jump forward if a large hole already
        // has a much later packet buffered.
        if let Some(&oldest) = self.packets.keys().next() {
            let ahead = oldest.wrapping_sub(next);
            if (2u16..0x8000).contains(&ahead) {
                // Gap with later packets already buffered: catch up to the
                // contiguous run instead of PLC+grow (startup misses were
                // inflating the JB to the 300 ms cap for the whole call).
                self.first_seq = Some(oldest);
                self.next_seq = Some(oldest);
                self.playout_anchor = Some(
                    self.clock.now() + Duration::from_millis(u64::from(self.target_ms())),
                );
                self.late_or_loss_recent = false;
                return PopOutcome::Wait;
            }
        }
        self.next_seq = Some(next.wrapping_add(1));
        self.late_or_loss_recent = true;
        self.grow_from_late();
        self.playout_started = true;
        PopOutcome::Lost
    }

    fn have_contiguous_from(&self, start: u16) -> u32 {
        let mut n = 0u32;
        let mut s = start;
        while self.packets.contains_key(&s) {
            n += 1;
            s = s.wrapping_add(1);
            if n > self.max_packets as u32 {
                break;
            }
        }
        n
    }

    /// Compatibility: packet or none (treats Wait/Lost as none).
    pub fn pop_packet(&mut self) -> Option<BufferedPacket> {
        match self.pop_ready() {
            PopOutcome::Packet(p) => Some(p),
            _ => None,
        }
    }

    /// Force PLC advance when playout clock ticks and nothing is ready.
    pub fn advance_on_underrun(&mut self) {
        if let Some(next) = self.next_seq {
            self.playout_started = true;
            self.next_seq = Some(next.wrapping_add(1));
            self.late_or_loss_recent = true;
            self.grow_from_late();
        }
    }

    fn grow_from_late(&mut self) {
        let cap = CAP_MS / FRAME_MS;
        if self.target_frames < cap {
            self.target_frames += 1;
        }
        self.hits_since_adapt = 0;
    }

    fn recompute_target_from_jitter(&mut self) {
        if self.jitter_samples.len() < 8 {
            return;
        }
        let mut sorted = self.jitter_samples.clone();
        sorted.sort_unstable();
        let idx = (sorted.len() * JITTER_PERCENTILE / 100).min(sorted.len() - 1);
        let p = sorted[idx];
        let desired_ms = (p + JITTER_MARGIN_MS).clamp(TARGET_MS_MIN, CAP_MS);
        let desired_frames = (desired_ms / FRAME_MS).max(2);
        // Grow immediately toward measurement; never exceed cap.
        if desired_frames > self.target_frames {
            self.target_frames = desired_frames.min(CAP_MS / FRAME_MS);
            self.hits_since_adapt = 0;
        }
    }

    fn maybe_shrink(&mut self) {
        if self.late_or_loss_recent {
            self.late_or_loss_recent = false;
            self.hits_since_adapt = 0;
            return;
        }
        if self.hits_since_adapt < SHRINK_AFTER_HITS {
            return;
        }
        let min_frames = TARGET_MS_MIN / FRAME_MS;
        // After a startup spike toward the cap, converge faster than 1 frame/sec
        // so steady-state returns to ~40–60 ms within a few seconds.
        if self.target_frames > min_frames.max(2) + 2 {
            let step = ((self.target_frames - min_frames.max(2)) / 2).max(1);
            self.target_frames -= step;
        } else if self.target_frames > min_frames.max(2) {
            self.target_frames -= 1;
        }
        self.hits_since_adapt = 0;
    }

    /// When depth sits well above the target, drop one excess future frame so
    /// the buffer converges without a large time jump.
    fn maybe_catch_up_depth(&mut self) {
        let target = self.target_frames as usize;
        if self.packets.len() <= target.saturating_add(1) {
            return;
        }
        // Drop the farthest-ahead packet (shed backlog, keep near-term playout).
        if let Some((&k, _)) = self.packets.iter().next_back() {
            self.packets.remove(&k);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn push_paced(jb: &mut JitterBuffer<ManualClock>, seq: u16, payload: u8) {
        jb.push(seq, vec![payload]);
        jb.clock_mut().advance_ms(FRAME_MS as u64);
    }

    #[test]
    fn ordered_playout_after_prime() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        for i in 0..10u16 {
            jb.push(i, vec![i as u8]);
        }
        let prime = jb.target_ms() as u64;
        jb.clock_mut().advance_ms(prime);
        let PopOutcome::Packet(a) = jb.pop_ready() else {
            panic!("expected first packet");
        };
        assert_eq!(a.seq, 0);
        let PopOutcome::Packet(b) = jb.pop_ready() else {
            panic!("expected second packet");
        };
        assert_eq!(b.seq, 1);
    }

    #[test]
    fn respects_cap() {
        let mut jb = JitterBuffer::new();
        for i in 0..1000u16 {
            jb.push(i, vec![0]);
        }
        assert!(jb.len() <= jb.max_packets());
    }

    #[test]
    fn over_cap_jumps_forward_not_drop_farthest() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        let cap = jb.max_packets() as u16;
        for i in 0..(cap + 5) {
            assert!(jb.push(i, vec![i as u8]));
        }
        assert!(jb.len() <= jb.max_packets());
        let prime = jb.target_ms() as u64;
        jb.clock_mut().advance_ms(prime);
        let PopOutcome::Packet(p) = jb.pop_ready() else {
            panic!("expected packet after over-cap jump");
        };
        assert!(
            p.seq >= 5,
            "expected jump-forward past shed head, got seq={}",
            p.seq
        );
    }

    #[test]
    fn late_arrival_before_deadline_is_not_loss() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        jb.push(0, vec![0]);
        let prime = jb.target_ms() as u64;
        jb.clock_mut().advance_ms(prime);
        assert!(matches!(jb.pop_ready(), PopOutcome::Packet(_)));
        // next=1 missing; deadline for seq 1 is +20 ms from anchor+0.
        jb.push(2, vec![2]);
        // Before deadline: Wait, not Lost.
        assert!(matches!(jb.pop_ready(), PopOutcome::Wait));
        jb.push(1, vec![1]);
        let PopOutcome::Packet(p) = jb.pop_ready() else {
            panic!("expected packet 1 after late-but-in-time arrival");
        };
        assert_eq!(p.seq, 1);
    }

    #[test]
    fn loss_only_after_playout_deadline() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        jb.push(0, vec![0]);
        let prime = jb.target_ms() as u64;
        jb.clock_mut().advance_ms(prime);
        assert!(matches!(jb.pop_ready(), PopOutcome::Packet(_)));
        jb.push(2, vec![2]);
        // Advance past seq 1 deadline.
        jb.clock_mut().advance_ms(FRAME_MS as u64 + 1);
        assert!(matches!(jb.pop_ready(), PopOutcome::Lost));
        let PopOutcome::Packet(p) = jb.pop_ready() else {
            panic!("expected seq 2 after loss of 1");
        };
        assert_eq!(p.seq, 2);
    }

    #[test]
    fn steady_link_target_converges_near_60ms() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        // Perfect 20 ms pacing for a long stretch.
        for i in 0..400u16 {
            push_paced(&mut jb, i, 0);
            // Drive playout once we are past priming.
            if i >= jb.target_frames() as u16 {
                let _ = jb.pop_ready();
            }
        }
        let t = jb.target_ms();
        assert!(
            t <= TARGET_MS_HIGH + FRAME_MS,
            "steady target should converge ≤ ~60 ms, got {t}"
        );
        assert!(t >= TARGET_MS_MIN, "target floor {t}");
    }

    #[test]
    fn jitter_burst_grows_then_shrinks() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        // Warm up steady.
        for i in 0..40u16 {
            push_paced(&mut jb, i, 0);
            if i >= 3 {
                let _ = jb.pop_ready();
            }
        }
        let before = jb.target_ms();
        // Burst: 80 ms inter-arrival spikes.
        for i in 40..80u16 {
            jb.push(i, vec![0]);
            jb.clock_mut().advance_ms(80);
            let _ = jb.pop_ready();
            jb.clock_mut().advance_ms(FRAME_MS as u64);
        }
        let peak = jb.target_ms();
        assert!(
            peak > before,
            "jitter should grow target ({before} → {peak})"
        );
        assert!(peak <= CAP_MS, "must not exceed cap");
        // Return to steady and allow shrink.
        for i in 80..400u16 {
            push_paced(&mut jb, i, 0);
            let _ = jb.pop_ready();
        }
        let after = jb.target_ms();
        assert!(
            after < peak,
            "target should shrink after stable period ({peak} → {after})"
        );
    }

    #[test]
    fn target_never_exceeds_cap() {
        let mut jb = JitterBuffer::with_clock(ManualClock::default());
        for i in 0..200u16 {
            jb.push(i, vec![0]);
            // Huge jitter samples.
            jb.clock_mut().advance_ms(500);
            let _ = jb.pop_ready();
        }
        assert!(jb.target_ms() <= CAP_MS);
        assert!(jb.len() <= jb.max_packets());
    }
}
