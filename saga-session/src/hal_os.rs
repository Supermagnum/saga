//! Production HAL: OS CSPRNG TRNG and in-memory vault storage.
//!
//! These replace `FakeTrng` / `FakeVaultStorage` (galdr-core `test-hal`) on the
//! phone path. Long-term signing keys are loaded into [`MemoryVaultStorage`] by
//! the SessionToken backend (USB Galdralag later; software mock stays in
//! `testing/mock-token` with Fake*).

use galdr_core::hal::VaultStorage;
use galdr_core::HalError;
use rand::rngs::OsRng;
use rand_core::{CryptoRng, RngCore};

/// HardwareTrng backed by the OS CSPRNG (`getrandom` / `OsRng`).
///
/// Blanket [`galdr_core::hal::HardwareTrng`] applies via `RngCore + CryptoRng`.
#[derive(Debug, Default, Clone, Copy)]
pub struct OsTrng;

impl OsTrng {
    pub fn new() -> Self {
        Self
    }
}

impl RngCore for OsTrng {
    fn next_u32(&mut self) -> u32 {
        OsRng.next_u32()
    }

    fn next_u64(&mut self) -> u64 {
        OsRng.next_u64()
    }

    fn fill_bytes(&mut self, dest: &mut [u8]) {
        OsRng.fill_bytes(dest);
    }

    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        OsRng.try_fill_bytes(dest)
    }
}

impl CryptoRng for OsTrng {}

/// In-memory vault byte store for LT signing keys on the phone process.
///
/// Not RRAM; suitable until a USB Galdralag vault is wired. Handshake still
/// needs [`VaultStorage`] to load the long-term signing key slot.
#[derive(Clone, Debug)]
pub struct MemoryVaultStorage {
    mem: Vec<u8>,
}

impl MemoryVaultStorage {
    pub fn new(size: usize) -> Self {
        Self {
            mem: vec![0u8; size],
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.mem.as_slice()
    }

    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        self.mem.as_mut_slice()
    }
}

impl VaultStorage for MemoryVaultStorage {
    fn read(&self, offset: u64, out: &mut [u8]) -> Result<(), HalError> {
        let o = usize::try_from(offset).map_err(|_| HalError::Denied)?;
        let end = o.checked_add(out.len()).ok_or(HalError::Denied)?;
        if end > self.mem.len() {
            return Err(HalError::Denied);
        }
        out.copy_from_slice(&self.mem[o..end]);
        Ok(())
    }

    fn write(&mut self, offset: u64, data: &[u8]) -> Result<(), HalError> {
        let o = usize::try_from(offset).map_err(|_| HalError::Denied)?;
        let end = o.checked_add(data.len()).ok_or(HalError::Denied)?;
        if end > self.mem.len() {
            return Err(HalError::Denied);
        }
        self.mem[o..end].copy_from_slice(data);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ephemeral_session::{EphemeralKeyPair, SessionCurve};

    #[test]
    fn production_trng_two_handshakes_differ_ephemeral_pubkeys() {
        let mut a = OsTrng::new();
        let mut b = OsTrng::new();
        let ka =
            EphemeralKeyPair::generate(SessionCurve::BrainpoolP384r1, &mut a).expect("ephemeral a");
        let kb =
            EphemeralKeyPair::generate(SessionCurve::BrainpoolP384r1, &mut b).expect("ephemeral b");
        assert_ne!(
            ka.public_key_bytes(),
            kb.public_key_bytes(),
            "two OsTrng handshakes must not reuse the same ephemeral public key"
        );
    }

    #[test]
    fn memory_vault_roundtrip() {
        let mut v = MemoryVaultStorage::new(64);
        v.write(8, &[1, 2, 3, 4]).expect("write");
        let mut out = [0u8; 4];
        v.read(8, &mut out).expect("read");
        assert_eq!(out, [1, 2, 3, 4]);
    }
}
