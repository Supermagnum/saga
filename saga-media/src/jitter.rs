//! Adaptive jitter buffer: target ~40–60 ms, cap ~200 ms.

use std::collections::BTreeMap;

use crate::codec::FRAME_MS;

const TARGET_MS_LOW: u32 = 40;
const TARGET_MS_HIGH: u32 = 60;
const CAP_MS: u32 = 200;

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
    /// Wait (silence): next missing but not yet declared lost.
    Wait,
    /// Gap declared lost; caller should conceal. Sequence already advanced.
    Lost,
}

#[derive(Debug)]
pub struct JitterBuffer {
    packets: BTreeMap<u16, Vec<u8>>,
    /// Next sequence expected for playout.
    next_seq: Option<u16>,
    /// True after the first successful pop / PLC advance.
    playout_started: bool,
    /// Current delay target in frames (adaptive).
    target_frames: u32,
    max_packets: usize,
    /// Consecutive ticks with later packets buffered but `next` missing.
    wait_ticks: u32,
}

impl Default for JitterBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl JitterBuffer {
    pub fn new() -> Self {
        let target_frames = ((TARGET_MS_LOW + TARGET_MS_HIGH) / 2) / FRAME_MS;
        let max_packets = (CAP_MS / FRAME_MS) as usize;
        Self {
            packets: BTreeMap::new(),
            next_seq: None,
            playout_started: false,
            target_frames: target_frames.max(1),
            max_packets,
            wait_ticks: 0,
        }
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

    /// True once a real packet has been popped (playout clock engaged).
    pub fn playout_started(&self) -> bool {
        self.playout_started
    }

    /// True when no RTP has been inserted yet (still waiting for first packet).
    pub fn awaiting_first_packet(&self) -> bool {
        self.next_seq.is_none()
    }

    /// Insert a packet. Drops if buffer would exceed cap (keeps nearer-to-playout).
    pub fn push(&mut self, seq: u16, payload: Vec<u8>) {
        match self.next_seq {
            None => self.next_seq = Some(seq),
            Some(next) => {
                let delta = next.wrapping_sub(seq);
                if !self.playout_started && delta > 0 && delta < 0x100 {
                    // Still priming: accept an earlier seq as the new start.
                    self.next_seq = Some(seq);
                } else if self.already_played(seq, next) {
                    return;
                }
            }
        }
        self.packets.insert(seq, payload);
        while self.packets.len() > self.max_packets {
            // Drop farthest-from-playout (highest seq ahead of next).
            if let Some((&k, _)) = self.packets.iter().next_back() {
                self.packets.remove(&k);
            } else {
                break;
            }
        }
    }

    fn already_played(&self, seq: u16, next: u16) -> bool {
        let delta = next.wrapping_sub(seq);
        delta > 0 && delta < 0x8000
    }

    /// Pop next packet, or indicate wait / loss.
    pub fn pop_ready(&mut self) -> PopOutcome {
        let Some(next) = self.next_seq else {
            return PopOutcome::Wait;
        };
        if let Some(payload) = self.packets.remove(&next) {
            self.playout_started = true;
            self.next_seq = Some(next.wrapping_add(1));
            self.wait_ticks = 0;
            self.adapt_after_hit();
            return PopOutcome::Packet(BufferedPacket { seq: next, payload });
        }
        if self.packets.is_empty() {
            self.wait_ticks = 0;
            return PopOutcome::Wait;
        }
        // Later packets exist but `next` is missing: allow a short reorder
        // window, then declare loss so we do not freeze on a permanent hole
        // (B→A emulator: played stuck at 10 while plc climbed).
        self.wait_ticks = self.wait_ticks.saturating_add(1);
        let ahead = self
            .packets
            .keys()
            .next()
            .map(|e| e.wrapping_sub(next))
            .unwrap_or(0);
        // Allow ~200 ms reorder/slow-recv before declaring loss. Too-short
        // windows skipped still-arriving seqs on loaded emulators.
        let lost = self.wait_ticks >= 10
            || (ahead >= 8 && ahead < 0x8000 && self.packets.len() as u32 >= self.target_frames);
        if lost {
            self.next_seq = Some(next.wrapping_add(1));
            self.wait_ticks = 0;
            self.adapt_after_miss();
            return PopOutcome::Lost;
        }
        PopOutcome::Wait
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
            self.adapt_after_miss();
        }
    }

    fn adapt_after_hit(&mut self) {
        let high = TARGET_MS_HIGH / FRAME_MS;
        if self.target_frames > high {
            self.target_frames -= 1;
        }
    }

    fn adapt_after_miss(&mut self) {
        let cap = CAP_MS / FRAME_MS;
        if self.target_frames < cap {
            self.target_frames += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_playout() {
        let mut jb = JitterBuffer::new();
        jb.push(1, vec![1]);
        jb.push(0, vec![0]);
        // Need depth: push fillers
        for i in 2..10 {
            jb.push(i, vec![i as u8]);
        }
        let PopOutcome::Packet(a) = jb.pop_ready() else {
            panic!("expected packet");
        };
        assert_eq!(a.seq, 0);
        let PopOutcome::Packet(b) = jb.pop_ready() else {
            panic!("expected packet");
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
}
