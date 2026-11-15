//! PIN providers for OpenPGP PW1 (User / INTERNAL AUTHENTICATE).
//!
//! PIN bytes are never logged. Callers must zeroise after use ([`zeroize::Zeroizing`]).

use std::sync::{Arc, Mutex};

use zeroize::Zeroizing;

use crate::UsbTokenError;

/// Supply the OpenPGP User PIN (PW1) when the card needs it.
pub trait PinProvider: Send + Sync {
    /// Return the User PIN bytes, or [`UsbTokenError::PinRequired`] if cancelled.
    fn user_pin(&self) -> Result<Zeroizing<Vec<u8>>, UsbTokenError>;
}

/// Fixed PIN for host tests (from env / test harness). Do not use in release UI.
pub struct StaticPinProvider {
    pin: Mutex<Zeroizing<Vec<u8>>>,
}

impl StaticPinProvider {
    pub fn new(pin: impl Into<Vec<u8>>) -> Self {
        Self {
            pin: Mutex::new(Zeroizing::new(pin.into())),
        }
    }

    pub fn from_pin_str(pin: &str) -> Self {
        Self::new(pin.as_bytes().to_vec())
    }

    pub fn shared(self) -> Arc<dyn PinProvider> {
        Arc::new(self)
    }
}

impl PinProvider for StaticPinProvider {
    fn user_pin(&self) -> Result<Zeroizing<Vec<u8>>, UsbTokenError> {
        let g = self
            .pin
            .lock()
            .map_err(|_| UsbTokenError::Protocol("PIN lock poisoned".into()))?;
        Ok(Zeroizing::new(g.to_vec()))
    }
}

/// Channel-backed PIN provider for Android UI (JNI posts the PIN once).
pub struct ChannelPinProvider {
    rx: Mutex<Option<std::sync::mpsc::Receiver<Zeroizing<Vec<u8>>>>>,
}

impl ChannelPinProvider {
    pub fn pair() -> (Self, std::sync::mpsc::Sender<Zeroizing<Vec<u8>>>) {
        let (tx, rx) = std::sync::mpsc::channel();
        (
            Self {
                rx: Mutex::new(Some(rx)),
            },
            tx,
        )
    }
}

impl PinProvider for ChannelPinProvider {
    fn user_pin(&self) -> Result<Zeroizing<Vec<u8>>, UsbTokenError> {
        let g = self
            .rx
            .lock()
            .map_err(|_| UsbTokenError::Protocol("PIN channel lock poisoned".into()))?;
        let rx = g.as_ref().ok_or(UsbTokenError::PinRequired)?;
        rx.recv().map_err(|_| UsbTokenError::PinRequired)
    }
}
