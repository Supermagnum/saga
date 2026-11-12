//! Deterministic software long-term Brainpool identities for E2E.

use ephemeral_session::{InitMessage, LongTermCert, SessionCurve};
use galdr_core::fake_hal::{FakeTrng, FakeVaultStorage};
use galdr_vault::brainpool384::BrainpoolP384SigningKey;
use galdr_vault::ecdsa_brainpool::BrainpoolSigningKey;
use galdr_vault::rsa_vault::KeySlot;
use galdr_vault::session_long_term_signing::{
    vault_store_session_long_term_signing_key, SessionLongTermSigningKey,
};
use zeroize::Zeroizing;

use saga_session::DEFAULT_SESSION_CURVE;

/// Vault backing size used by FakeVaultStorage in firmware tests.
pub(crate) const VAULT_SIZE: usize = 0x120_000;

/// Slot holding this identity's long-term signing key inside a fresh vault.
pub(crate) const LOCAL_SLOT: KeySlot = KeySlot(0);

/// Software Galdralag long-term identity derived deterministically from a label.
///
/// Iroh EndpointIds stay separate (`saga-dev-v1:{label}` in saga-iroh-core).
/// Token keys use `saga-mock-token-v2-brainpool:{label}:{curve_wire_id}`.
#[derive(Clone)]
pub struct MockIdentity {
    pub label: String,
    pub curve: SessionCurve,
    /// Scalar bytes (32 for P-256, 48 for P-384).
    scalar: Zeroizing<Vec<u8>>,
}

impl MockIdentity {
    /// Default profile: Brainpool P-384r1 (firmware / CESS preference).
    pub fn from_label(label: &str) -> Self {
        Self::from_label_curve(label, DEFAULT_SESSION_CURVE)
    }

    pub fn from_label_curve(label: &str, curve: SessionCurve) -> Self {
        let material = format!("saga-mock-token-v2-brainpool:{label}:{}", curve.wire_id());
        let hash = blake3::hash(material.as_bytes());
        let seed = u64::from_le_bytes(hash.as_bytes()[0..8].try_into().expect("8 bytes"));
        let mut trng = FakeTrng::from_seed(seed);
        let scalar = match curve {
            SessionCurve::BrainpoolP256r1 => {
                let sk = BrainpoolSigningKey::generate(&mut trng).expect("p256 lt key");
                Zeroizing::new(sk.to_scalar_bytes_for_test().to_vec())
            }
            SessionCurve::BrainpoolP384r1 => {
                let sk = BrainpoolP384SigningKey::generate(&mut trng).expect("p384 lt key");
                Zeroizing::new(sk.to_scalar_bytes_for_test().to_vec())
            }
        };
        Self {
            label: label.to_string(),
            curve,
            scalar,
        }
    }

    /// Rebuild signing key enum for vault store.
    pub(crate) fn signing_key(&self) -> SessionLongTermSigningKey {
        match self.curve {
            SessionCurve::BrainpoolP256r1 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&self.scalar[..32]);
                SessionLongTermSigningKey::P256(
                    BrainpoolSigningKey::from_scalar_bytes_for_test(&arr).expect("p256 scalar"),
                )
            }
            SessionCurve::BrainpoolP384r1 => {
                let mut arr = [0u8; 48];
                arr.copy_from_slice(&self.scalar[..48]);
                SessionLongTermSigningKey::P384(
                    BrainpoolP384SigningKey::from_scalar_bytes_for_test(&arr).expect("p384 scalar"),
                )
            }
        }
    }

    /// Uncompressed SEC1 verifying key bytes.
    pub fn verifying_sec1(&self) -> Vec<u8> {
        match self.signing_key() {
            SessionLongTermSigningKey::P256(sk) => {
                sk.verifying_key().to_sec1_uncompressed().to_vec()
            }
            SessionLongTermSigningKey::P384(sk) => {
                sk.verifying_key().to_sec1_uncompressed().to_vec()
            }
        }
    }

    /// Long-term certificate as used by ephemeral-session trust checks.
    pub fn long_term_cert(&self) -> LongTermCert {
        let sec1 = self.verifying_sec1();
        let mut verifying_key = heapless::Vec::new();
        verifying_key
            .extend_from_slice(&sec1)
            .expect("sec1 fits MAX_SEC1");
        LongTermCert {
            fingerprint: LongTermCert::fingerprint_of(&sec1),
            curve: self.curve,
            verifying_key,
        }
    }

    /// Wire fingerprint: 64 lowercase hex ASCII bytes.
    pub fn fingerprint_hex(&self) -> [u8; 64] {
        let cert = self.long_term_cert();
        let mut raw = [0u8; 32];
        raw.copy_from_slice(cert.fingerprint.as_slice());
        let hex = InitMessage::encode_fingerprint_hex(&raw);
        let mut out = [0u8; 64];
        out.copy_from_slice(hex.as_slice());
        out
    }

    /// Fingerprint as lowercase hex string (contact storage format).
    pub fn fingerprint_hex_string(&self) -> String {
        String::from_utf8(self.fingerprint_hex().to_vec()).expect("hex ascii")
    }

    /// Fresh vault containing this identity's LT key at [`LOCAL_SLOT`].
    pub(crate) fn vault_with_self(&self) -> FakeVaultStorage {
        let mut storage = FakeVaultStorage::new(VAULT_SIZE);
        vault_store_session_long_term_signing_key(
            &mut storage,
            &LOCAL_SLOT,
            &self.signing_key(),
            true,
        )
        .expect("store lt key");
        storage
    }
}
