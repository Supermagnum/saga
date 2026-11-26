//! Map Galdra sync-export public key material to Saga ephemeral-session fingerprints.
//!
//! Saga fingerprint = SHA-256(uncompressed SEC1 verifying key) as 64 lowercase hex
//! ([`saga_session::fingerprint_hex_from_sec1`]).

use saga_session::fingerprint_hex_from_sec1;

/// Why a Galdra contact key cannot become a Saga fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportReject {
    Empty,
    WrongCurveOrFormat,
}

/// Brainpool P-256 / P-384 uncompressed SEC1 lengths (0x04 || X || Y).
const SEC1_P256: usize = 1 + 32 + 32;
const SEC1_P384: usize = 1 + 48 + 48;

/// OpenPGP OID encodings (without DER tag/length) for brainpool curves Saga accepts.
const OID_BP256: &[u8] = &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x07];
const OID_BP384: &[u8] = &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x0b];

/// Convert raw public-key bytes from a Galdra `identities.pgp_pubkey` blob into
/// Saga's 64-hex fingerprint, or explain why not.
pub fn saga_fingerprint_hex_from_pubkey(blob: &[u8]) -> Result<String, ImportReject> {
    if blob.is_empty() {
        return Err(ImportReject::Empty);
    }
    if let Some(sec1) = as_sec1(blob) {
        return Ok(hex_string(sec1));
    }
    if let Some(sec1) = extract_sec1_after_brainpool_oid(blob) {
        return Ok(hex_string(sec1));
    }
    Err(ImportReject::WrongCurveOrFormat)
}

fn hex_string(sec1: &[u8]) -> String {
    let bytes = fingerprint_hex_from_sec1(sec1);
    String::from_utf8(bytes.to_vec()).expect("hex ascii")
}

fn as_sec1(blob: &[u8]) -> Option<&[u8]> {
    if (blob.len() == SEC1_P256 || blob.len() == SEC1_P384) && blob[0] == 0x04 {
        Some(blob)
    } else {
        None
    }
}

fn extract_sec1_after_brainpool_oid(blob: &[u8]) -> Option<&[u8]> {
    for (oid, sec1_len) in [(OID_BP256, SEC1_P256), (OID_BP384, SEC1_P384)] {
        let mut start = 0usize;
        while let Some(rel) = find_subslice(&blob[start..], oid) {
            let oid_end = start + rel + oid.len();
            if let Some(sec1) = find_sec1_at(blob, oid_end, sec1_len) {
                return Some(sec1);
            }
            start = oid_end;
        }
    }
    None
}

fn find_sec1_at(blob: &[u8], from: usize, sec1_len: usize) -> Option<&[u8]> {
    let end = (from + 8).min(blob.len());
    for i in from..end {
        if i + sec1_len <= blob.len() && blob[i] == 0x04 {
            return Some(&blob[i..i + sec1_len]);
        }
        if i + 2 + sec1_len <= blob.len() && blob[i + 2] == 0x04 {
            return Some(&blob[i + 2..i + 2 + sec1_len]);
        }
    }
    None
}

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sec1_p256_round_trip_stable() {
        let mut sec1 = vec![0x04];
        sec1.extend(std::iter::repeat_n(0x11u8, 64));
        let a = saga_fingerprint_hex_from_pubkey(&sec1).unwrap();
        let b = saga_fingerprint_hex_from_pubkey(&sec1).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 64);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn empty_rejected() {
        assert_eq!(
            saga_fingerprint_hex_from_pubkey(&[]),
            Err(ImportReject::Empty)
        );
    }

    #[test]
    fn garbage_rejected() {
        assert_eq!(
            saga_fingerprint_hex_from_pubkey(&[1, 2, 3, 4]),
            Err(ImportReject::WrongCurveOrFormat)
        );
    }

    #[test]
    fn oid_prefixed_sec1_accepted() {
        let mut sec1 = vec![0x04];
        sec1.extend(std::iter::repeat_n(0x22u8, 64));
        let mut blob = OID_BP256.to_vec();
        blob.extend_from_slice(&sec1);
        let fp = saga_fingerprint_hex_from_pubkey(&blob).unwrap();
        assert_eq!(fp, saga_fingerprint_hex_from_pubkey(&sec1).unwrap());
    }
}
