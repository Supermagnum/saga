//! Fixed-size ciphertext length buckets (metadata hygiene).

use crate::seal::MsgError;

/// Padding buckets for padded plaintext (before AEAD). Issue #41: 256 B / 1 KB / 4 KB.
pub const PAD_BUCKETS: [usize; 3] = [256, 1024, 4096];

/// Largest accepted UTF-8 text before padding equals the top bucket body budget.
/// Wire pad layout: `[u16 BE plaintext_len][plaintext][zeros…]` fitting a bucket.
pub const PAD_MAX_PLAINTEXT: usize = 4094; // 2-byte length + body ≤ 4096

/// Pad plaintext into the smallest bucket that fits (2-byte length prefix + body).
pub fn pad_plaintext(plaintext: &[u8]) -> Result<Vec<u8>, MsgError> {
    if plaintext.len() > PAD_MAX_PLAINTEXT {
        return Err(MsgError::TooLarge);
    }
    let need = 2 + plaintext.len();
    let bucket = PAD_BUCKETS
        .iter()
        .copied()
        .find(|&b| b >= need)
        .ok_or(MsgError::TooLarge)?;
    let mut out = vec![0u8; bucket];
    let len = plaintext.len() as u16;
    out[0..2].copy_from_slice(&len.to_be_bytes());
    out[2..2 + plaintext.len()].copy_from_slice(plaintext);
    Ok(out)
}

/// Strip length prefix and trailing pad zeros.
pub fn unpad_plaintext(padded: &[u8]) -> Result<Vec<u8>, MsgError> {
    if padded.len() < 2 {
        return Err(MsgError::BadPadding);
    }
    if !PAD_BUCKETS.contains(&padded.len()) {
        return Err(MsgError::BadPadding);
    }
    let len = u16::from_be_bytes([padded[0], padded[1]]) as usize;
    if 2 + len > padded.len() {
        return Err(MsgError::BadPadding);
    }
    // Trailing bytes after plaintext must be zeros (constant-size pad).
    if padded[2 + len..].iter().any(|&b| b != 0) {
        return Err(MsgError::BadPadding);
    }
    Ok(padded[2..2 + len].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_round_trip() {
        for msg in [
            b"" as &[u8],
            b"hi",
            &[b'x'; 200],
            &[b'y'; 1000],
            &[b'z'; 4000],
        ] {
            let padded = pad_plaintext(msg).unwrap();
            assert!(PAD_BUCKETS.contains(&padded.len()));
            assert_eq!(unpad_plaintext(&padded).unwrap(), msg);
        }
    }

    #[test]
    fn rejects_oversized() {
        let big = vec![b'a'; PAD_MAX_PLAINTEXT + 1];
        assert!(matches!(pad_plaintext(&big), Err(MsgError::TooLarge)));
    }
}
