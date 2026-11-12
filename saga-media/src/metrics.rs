use std::sync::atomic::{AtomicU64, Ordering};

/// Counters exposed to UI / tests. No key material.
#[derive(Debug, Default)]
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
}

impl MediaCounters {
    pub fn snapshot(&self) -> MediaCountersSnapshot {
        MediaCountersSnapshot {
            datagram_blocked_drops: self.datagram_blocked_drops.load(Ordering::Relaxed),
            tamper_or_auth_fail: self.tamper_or_auth_fail.load(Ordering::Relaxed),
            replay: self.replay.load(Ordering::Relaxed),
            played: self.played.load(Ordering::Relaxed),
            sent: self.sent.load(Ordering::Relaxed),
            plc_frames: self.plc_frames.load(Ordering::Relaxed),
        }
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
}
