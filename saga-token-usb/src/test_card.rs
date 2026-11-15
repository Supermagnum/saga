//! Software AUT signer for host/CI (no USB).
//!
//! Signs SHA-256 digests with Brainpool P-256 ECDSA (DER), matching Nitrokey
//! `authenticate_for_hash(ECC, digest)` behaviour — one ECDSA over the digest,
//! no extra hash. Uses the same `ecdsa` / `bp256` stack as galdr_vault.

use std::sync::Arc;

use bp256::BrainpoolP256r1;
use ecdsa::signature::hazmat::PrehashSigner;
use ecdsa::{Signature, SigningKey};
use galdr_core::fake_hal::FakeTrng;
use galdr_vault::ecdsa_brainpool::BrainpoolSigningKey;
use saga_session::{SessionCurve, TokenError};

use crate::nitrokey::{AutSigner, NitrokeySessionToken};

/// Software AUT key (Brainpool P-256) backed by galdr_vault keygen.
pub struct SoftwareAutSigner {
    scalar: [u8; 32],
    sec1: Vec<u8>,
}

impl SoftwareAutSigner {
    pub fn from_seed(seed: u64) -> Self {
        let mut trng = FakeTrng::from_seed(seed);
        let sk = BrainpoolSigningKey::generate(&mut trng).expect("p256");
        let sec1 = sk.verifying_key().to_sec1_uncompressed().to_vec();
        let scalar = sk.to_scalar_bytes_for_test();
        Self { scalar, sec1 }
    }

    pub fn sec1(&self) -> &[u8] {
        &self.sec1
    }

    pub fn shared(self) -> Arc<dyn AutSigner> {
        Arc::new(self)
    }
}

impl AutSigner for SoftwareAutSigner {
    fn curve(&self) -> SessionCurve {
        SessionCurve::BrainpoolP256r1
    }

    fn sec1(&self) -> &[u8] {
        &self.sec1
    }

    fn sign_digest(&self, digest: &[u8]) -> Result<Vec<u8>, TokenError> {
        let sk = SigningKey::<BrainpoolP256r1>::from_slice(self.scalar.as_slice())
            .map_err(|e| TokenError::Protocol(format!("bp256 sk: {e}")))?;
        let sig: Signature<BrainpoolP256r1> = sk
            .sign_prehash(digest)
            .map_err(|e| TokenError::Protocol(format!("sign_prehash: {e}")))?;
        Ok(sig.to_der().as_bytes().to_vec())
    }
}

/// Build a [`NitrokeySessionToken`] over a software AUT key (host/CI interop).
pub fn software_nitrokey_token_from_seed(seed: u64) -> Result<NitrokeySessionToken, TokenError> {
    NitrokeySessionToken::from_signer(SoftwareAutSigner::from_seed(seed).shared())
}

/// Software Nitrokey token with a fixed seed (deterministic tests).
pub fn software_nitrokey_token() -> Result<NitrokeySessionToken, TokenError> {
    software_nitrokey_token_from_seed(0x5A6A_C0DE_u64)
}
