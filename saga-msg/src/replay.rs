//! Sliding replay / duplicate rejection window (sequence + message id).

use std::collections::HashSet;

use crate::seal::MSG_ID_LEN;

/// How many recent sequence numbers to remember per direction.
pub const REPLAY_WINDOW_SIZE: u64 = 256;

/// Tracks seen `(seq, msg_id)` pairs and rejects duplicates / old seqs.
#[derive(Debug, Default)]
pub struct ReplayWindow {
    highest: Option<u64>,
    seen_ids: HashSet<[u8; MSG_ID_LEN]>,
    /// Sequence numbers currently held in the window (for pruning).
    seen_seqs: HashSet<u64>,
}

impl ReplayWindow {
    pub fn new() -> Self {
        Self::default()
    }

    /// Accept a new message. Returns `true` if this is the first time we see it
    /// and `seq` is within the window of `highest`.
    pub fn accept(&mut self, seq: u64, msg_id: &[u8; MSG_ID_LEN]) -> bool {
        if self.seen_ids.contains(msg_id) {
            return false;
        }
        if let Some(hi) = self.highest {
            if seq + REPLAY_WINDOW_SIZE < hi {
                // Too old relative to the highest accepted seq.
                return false;
            }
            if self.seen_seqs.contains(&seq) {
                return false;
            }
        }
        self.seen_ids.insert(*msg_id);
        self.seen_seqs.insert(seq);
        let hi = self.highest.map(|h| h.max(seq)).unwrap_or(seq);
        self.highest = Some(hi);
        self.prune(hi);
        true
    }

    fn prune(&mut self, highest: u64) {
        if highest < REPLAY_WINDOW_SIZE {
            return;
        }
        let floor = highest - REPLAY_WINDOW_SIZE;
        self.seen_seqs.retain(|&s| s > floor);
        // Message ids for pruned seqs are left; bound set size by clearing when large.
        if self.seen_ids.len() > (REPLAY_WINDOW_SIZE as usize) * 2 {
            self.seen_ids.clear();
            // Re-seed is conservative: after clear, only seq floor protects.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u8) -> [u8; MSG_ID_LEN] {
        let mut id = [0u8; MSG_ID_LEN];
        id[0] = n;
        id
    }

    #[test]
    fn rejects_duplicate_id() {
        let mut w = ReplayWindow::new();
        assert!(w.accept(1, &id(1)));
        assert!(!w.accept(2, &id(1)));
    }

    #[test]
    fn rejects_duplicate_seq() {
        let mut w = ReplayWindow::new();
        assert!(w.accept(1, &id(1)));
        assert!(!w.accept(1, &id(2)));
    }

    #[test]
    fn rejects_too_old() {
        let mut w = ReplayWindow::new();
        assert!(w.accept(300, &id(1)));
        assert!(!w.accept(1, &id(2)));
    }
}
