//! USB device IDs and APDU transport abstractions shared by Nitrokey and Galdralag.

use std::sync::{Arc, Mutex};

use card_backend::{CardBackend, CardCaps, CardTransaction, PinType, SmartcardError};
use openpgp_card::ocard::crypto::{PublicKeyMaterial, SigningAlgo};
use openpgp_card::ocard::KeyType;
use openpgp_card::{Card, Error as CardError};
use secrecy::SecretString;
use zeroize::Zeroizing;

use crate::algo::curve_from_algorithm_attributes;
use crate::UsbTokenError;

/// Nitrokey 3 (USB CCID). Firmware with SE supports Brainpool AUT.
pub const NITROKEY_3: UsbDeviceIds = UsbDeviceIds {
    vendor_id: 0x20a0,
    product_id: 0x42b2,
    name: "Nitrokey 3",
};

/// Nitrokey Pro 2 (OpenPGP Card 3.x over USB).
pub const NITROKEY_PRO2: UsbDeviceIds = UsbDeviceIds {
    vendor_id: 0x20a0,
    product_id: 0x4108,
    name: "Nitrokey Pro 2",
};

/// Galdralag Dabao (future CCID peer; same Android USB filter path).
pub const GALDRALAG_DABAO: UsbDeviceIds = UsbDeviceIds {
    vendor_id: 0x1d50,
    product_id: 0x6197,
    name: "Galdralag Dabao",
};

/// USB Host filter entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbDeviceIds {
    pub vendor_id: u16,
    pub product_id: u16,
    pub name: &'static str,
}

/// Devices Saga's Android USB filter should claim.
pub fn supported_usb_devices() -> &'static [UsbDeviceIds] {
    &[NITROKEY_3, NITROKEY_PRO2, GALDRALAG_DABAO]
}

/// Low-level APDU exchange (Android USB CCID or test double).
pub trait ApduTransport: Send + Sync {
    fn transmit(&self, capdu: &[u8]) -> Result<Vec<u8>, UsbTokenError>;
}

type ApduCallback = Box<dyn FnMut(&[u8]) -> Result<Vec<u8>, UsbTokenError> + Send>;

/// Closure-backed transport for JNI (`transmitApdu` from Kotlin).
pub struct CallbackTransport {
    inner: Mutex<ApduCallback>,
}

impl CallbackTransport {
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(&[u8]) -> Result<Vec<u8>, UsbTokenError> + Send + 'static,
    {
        Self {
            inner: Mutex::new(Box::new(f)),
        }
    }
}

impl ApduTransport for CallbackTransport {
    fn transmit(&self, capdu: &[u8]) -> Result<Vec<u8>, UsbTokenError> {
        let mut g = self
            .inner
            .lock()
            .map_err(|_| UsbTokenError::Protocol("APDU lock poisoned".into()))?;
        g(capdu)
    }
}

/// Factory that opens a fresh [`CardBackend`] for each handshake step.
pub trait CardFactory: Send + Sync {
    fn open(&self) -> Result<Box<dyn CardBackend + Send + Sync>, UsbTokenError>;
}

/// Shared factory handle.
pub type SharedCardFactory = Arc<dyn CardFactory>;

/// OpenPGP AUT operations needed by [`crate::NitrokeySessionToken`].
pub struct OpenPgpAutCard;

impl OpenPgpAutCard {
    /// Read AUT curve + SEC1 public key; reject unsupported cards.
    pub fn read_aut_identity(
        backend: Box<dyn CardBackend + Send + Sync>,
    ) -> Result<(saga_session::SessionCurve, Vec<u8>), UsbTokenError> {
        let mut card = Card::new(backend).map_err(map_card_err)?;
        let mut tx = card.transaction().map_err(map_card_err)?;
        let attrs = tx
            .algorithm_attributes(KeyType::Authentication)
            .map_err(map_card_err)?;
        let curve = curve_from_algorithm_attributes(&attrs)?.session;
        let pk = tx
            .public_key_material(KeyType::Authentication)
            .map_err(map_card_err)?;
        let sec1 = match pk {
            PublicKeyMaterial::E(ecc) => ecc.data().to_vec(),
            other => {
                return Err(UsbTokenError::CardNotSupported(format!(
                    "AUT pubkey {other:?}"
                )))
            }
        };
        if sec1.first() != Some(&0x04) {
            return Err(UsbTokenError::CardNotSupported(
                "AUT pubkey is not uncompressed SEC1".into(),
            ));
        }
        let expect = match curve {
            saga_session::SessionCurve::BrainpoolP256r1 => 65,
            saga_session::SessionCurve::BrainpoolP384r1 => 97,
        };
        if sec1.len() != expect {
            return Err(UsbTokenError::CardNotSupported(format!(
                "AUT SEC1 len {} (want {expect})",
                sec1.len()
            )));
        }
        Ok((curve, sec1))
    }

    /// Verify User PIN (PW1 / P2=82) and INTERNAL AUTHENTICATE over `digest` (ECC: raw hash).
    pub fn internal_authenticate(
        backend: Box<dyn CardBackend + Send + Sync>,
        pin: &Zeroizing<Vec<u8>>,
        digest: &[u8],
    ) -> Result<Vec<u8>, UsbTokenError> {
        let pin_str = String::from_utf8(pin.to_vec())
            .map_err(|_| UsbTokenError::Protocol("PIN must be UTF-8 for OpenPGP VERIFY".into()))?;
        let secret: SecretString = pin_str.into();
        let mut card = Card::new(backend).map_err(map_card_err)?;
        let mut tx = card.transaction().map_err(map_card_err)?;
        // Ensure AUT key is selected for INTERNAL AUTHENTICATE.
        let _ = tx.manage_security_environment(KeyType::Authentication, KeyType::Authentication);
        tx.verify_user_pin(secret).map_err(map_pin_err)?;
        let sig = tx
            .card()
            .authenticate_for_hash(SigningAlgo::ECC, digest)
            .map_err(map_card_err)?;
        Ok(sig)
    }
}

fn map_card_err(e: CardError) -> UsbTokenError {
    let s = e.to_string();
    let lower = s.to_lowercase();
    if lower.contains("6982")
        || lower.contains("69 82")
        || lower.contains("security status")
        || lower.contains("pw1")
        || lower.contains("pin")
    {
        return UsbTokenError::PinRequired;
    }
    if lower.contains("6983") || lower.contains("69 83") || lower.contains("blocked") {
        return UsbTokenError::CardLocked;
    }
    if lower.contains("63c0") || lower.contains("63 c0") {
        return UsbTokenError::CardLocked;
    }
    if lower.contains("63c") || lower.contains("63 c") {
        return UsbTokenError::PinRequired;
    }
    UsbTokenError::Protocol(s)
}

fn map_pin_err(e: CardError) -> UsbTokenError {
    let mapped = map_card_err(e);
    match mapped {
        UsbTokenError::Protocol(_) => UsbTokenError::PinRequired,
        other => other,
    }
}

/// [`CardBackend`] that forwards APDUs to an [`ApduTransport`] (Android USB CCID).
pub struct ApduCardBackend {
    transport: Arc<dyn ApduTransport>,
}

impl ApduCardBackend {
    pub fn new(transport: Arc<dyn ApduTransport>) -> Self {
        Self { transport }
    }
}

impl CardBackend for ApduCardBackend {
    fn limit_card_caps(&self, card_caps: CardCaps) -> CardCaps {
        card_caps
    }

    fn transaction(
        &mut self,
        _reselect_application: Option<&[u8]>,
    ) -> Result<Box<dyn CardTransaction + Send + Sync + '_>, SmartcardError> {
        Ok(Box::new(ApduCardTransaction {
            transport: Arc::clone(&self.transport),
        }))
    }
}

struct ApduCardTransaction {
    transport: Arc<dyn ApduTransport>,
}

impl CardTransaction for ApduCardTransaction {
    fn transmit(&mut self, cmd: &[u8], _buf_size: usize) -> Result<Vec<u8>, SmartcardError> {
        self.transport
            .transmit(cmd)
            .map_err(|e| SmartcardError::Error(e.to_string()))
    }

    fn feature_pinpad_verify(&self) -> bool {
        false
    }

    fn feature_pinpad_modify(&self) -> bool {
        false
    }

    fn pinpad_verify(
        &mut self,
        _pin: PinType,
        _card_caps: &Option<CardCaps>,
    ) -> Result<Vec<u8>, SmartcardError> {
        Err(SmartcardError::Error("pinpad not supported".into()))
    }

    fn pinpad_modify(
        &mut self,
        _pin: PinType,
        _card_caps: &Option<CardCaps>,
    ) -> Result<Vec<u8>, SmartcardError> {
        Err(SmartcardError::Error("pinpad not supported".into()))
    }

    fn was_reset(&self) -> bool {
        false
    }
}

/// Factory wrapping a shared [`ApduTransport`].
pub struct ApduCardFactory {
    transport: Arc<dyn ApduTransport>,
}

impl ApduCardFactory {
    pub fn new(transport: Arc<dyn ApduTransport>) -> Self {
        Self { transport }
    }
}

impl CardFactory for ApduCardFactory {
    fn open(&self) -> Result<Box<dyn CardBackend + Send + Sync>, UsbTokenError> {
        Ok(Box::new(ApduCardBackend::new(Arc::clone(&self.transport))))
    }
}
