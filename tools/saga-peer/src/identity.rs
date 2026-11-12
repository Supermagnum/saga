//! Dev identity labels (phone-digit keys) shared with Android mock-token builds.
//!
//! Cellular-only contacts (thor) must never appear here — they must not resolve
//! to an Iroh secret or MockIdentity.

use iroh::SecretKey;

pub const LABEL_BOB: &str = "15550100010";
pub const LABEL_ALICE: &str = "15550100011";
/// Laptop Linux peer (MockIdentity fingerprint seeded as contact "laptop").
/// Distinct from thor's cellular number (+15550100012).
pub const LABEL_LAPTOP: &str = "15550100020";

/// BrainpoolP384r1 MockIdentity fingerprint for [`LABEL_LAPTOP`].
pub const FINGERPRINT_LAPTOP: &str =
    "43676d156721e149e722ad891a441287df5dcbe66f8daf155f1ba0168274d471";

pub fn known_dev_labels() -> &'static [&'static str] {
    &[LABEL_BOB, LABEL_ALICE, LABEL_LAPTOP]
}

/// Same BLAKE3 material as `saga-iroh-core` mock-token builds.
pub fn dev_secret_key(peer_label: &str) -> SecretKey {
    let material = format!("saga-dev-v1:{peer_label}");
    let hash = blake3::hash(material.as_bytes());
    SecretKey::from_bytes(hash.as_bytes())
}

/// Map an Iroh public key or phone-digit label to a MockIdentity label when known.
pub fn resolve_peer_label(peer: &str) -> String {
    if known_dev_labels().contains(&peer) {
        return peer.to_string();
    }
    if let Ok(pk) = peer.parse::<iroh::PublicKey>() {
        for label in known_dev_labels() {
            if dev_secret_key(label).public() == pk {
                return (*label).to_string();
            }
        }
    }
    peer.to_string()
}
