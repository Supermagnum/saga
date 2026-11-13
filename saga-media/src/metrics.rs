use std::sync::atomic::{AtomicU64, Ordering};

use crate::codec::FRAME_MS;

/// Histogram bins for JB depth in packets (index = depth). Cap is 300 ms → 15.
const JB_DEPTH_HIST_BINS: usize = 64;

/// Counters exposed to UI / tests. No key material.
#[derive(Debug)]
pub struct MediaCounters {
    /// Packets dropped because the QUIC datagram send buffer was full.
    pub datagram_blocked_drops: AtomicU64,
    /// SRTP auth / decrypt failures (tamper or wrong key).
    pub tamper_or_auth_fail: AtomicU64,
    /// SRTP replay detector drops.
    pub replay: AtomicU64,
    /// Packets successfully played out after decode.
    pub played: AtomicU64,
    /// Packets sent (queued on the wire).
    pub sent: AtomicU64,
    /// PLC concealment frames synthesized.
    pub plc_frames: AtomicU64,
    /// Packets dropped because Saga media tag did not match this pipeline mode.
    pub wrong_media_mode: AtomicU64,
    /// RTP arrived after playout had already advanced past that seq (late).
    pub late_arrivals: AtomicU64,
    /// Gaps declared lost by the jitter buffer (after reorder wait).
    pub lost_packets: AtomicU64,
    /// Max observed JB depth in packets.
    pub jb_depth_max: AtomicU64,
    /// Sum of depth samples (packets) for mean.
    pub jb_depth_sum: AtomicU64,
    /// Number of depth samples.
    pub jb_depth_n: AtomicU64,
    /// Depth histogram (packets → count); used for median.
    pub jb_depth_hist: [AtomicU64; JB_DEPTH_HIST_BINS],
    /// Latest adaptive JB target (ms).
    pub jb_target_ms: AtomicU64,
}

impl Default for MediaCounters {
    fn default() -> Self {
        Self {
            datagram_blocked_drops: AtomicU64::new(0),
            tamper_or_auth_fail: AtomicU64::new(0),
            replay: AtomicU64::new(0),
            played: AtomicU64::new(0),
            sent: AtomicU64::new(0),
            plc_frames: AtomicU64::new(0),
            wrong_media_mode: AtomicU64::new(0),
            late_arrivals: AtomicU64::new(0),
            lost_packets: AtomicU64::new(0),
            jb_depth_max: AtomicU64::new(0),
            jb_depth_sum: AtomicU64::new(0),
            jb_depth_n: AtomicU64::new(0),
            jb_depth_hist: [const { AtomicU64::new(0) }; JB_DEPTH_HIST_BINS],
            jb_target_ms: AtomicU64::new(0),
        }
    }
}

impl MediaCounters {
    pub fn record_jb_depth(&self, depth_packets: usize) {
        let d = depth_packets as u64;
        self.jb_depth_max.fetch_max(d, Ordering::Relaxed);
        self.jb_depth_sum.fetch_add(d, Ordering::Relaxed);
        self.jb_depth_n.fetch_add(1, Ordering::Relaxed);
        let idx = depth_packets.min(JB_DEPTH_HIST_BINS - 1);
        self.jb_depth_hist[idx].fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_jb_target_ms(&self, target_ms: u32) {
        self.jb_target_ms
            .store(u64::from(target_ms), Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> MediaCountersSnapshot {
        let n = self.jb_depth_n.load(Ordering::Relaxed);
        let sum = self.jb_depth_sum.load(Ordering::Relaxed);
        let max_pkts = self.jb_depth_max.load(Ordering::Relaxed);
        let median_pkts = self.median_depth_packets();
        MediaCountersSnapshot {
            datagram_blocked_drops: self.datagram_blocked_drops.load(Ordering::Relaxed),
            tamper_or_auth_fail: self.tamper_or_auth_fail.load(Ordering::Relaxed),
            replay: self.replay.load(Ordering::Relaxed),
            played: self.played.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            plc_frames: self.plc_frames.load(Ordering::Relaxed),
            wrong_media_mode: self.wrong_media_mode.load(Ordering::Relaxed),
            late_arrivals: self.late_arrivals.load(Ordering::Relaxed),
            lost_packets: self.lost_packets.load(Ordering::Relaxed),
            jb_depth_max_ms: max_pkts * u64::from(FRAME_MS),
            jb_depth_median_ms: median_pkts * u64::from(FRAME_MS),
            jb_depth_mean_ms: if n == 0 {
                0
            } else {
                (sum * u64::from(FRAME_MS)) / n
            },
            jb_depth_samples: n,
            jb_target_ms: self.jb_target_ms.load(Ordering::Relaxed),
        }
    }

    fn median_depth_packets(&self) -> u64 {
        let n = self.jb_depth_n.load(Ordering::Relaxed);
        if n == 0 {
            return 0;
        }
        let target = n.div_ceil(2);
        let mut seen = 0u64;
        for (depth, bin) in self.jb_depth_hist.iter().enumerate() {
            seen += bin.load(Ordering::Relaxed);
            if seen >= target {
                return depth as u64;
            }
        }
        (JB_DEPTH_HIST_BINS - 1) as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MediaCountersSnapshot {
    pub datagram_blocked_drops: u64,
    pub tamper_or_auth_fail: u64,
    pub replay: u64,
    pub played: u64,
    pub sent: u64,
    pub plc_frames: u64,
    pub wrong_media_mode: u64,
    pub late_arrivals: u64,
    pub lost_packets: u64,
    pub jb_depth_max_ms: u64,
    pub jb_depth_median_ms: u64,
    pub jb_depth_mean_ms: u64,
    pub jb_depth_samples: u64,
    pub jb_target_ms: u64,
}
