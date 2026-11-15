//! USB / OpenPGP card errors mapped into [`saga_session::TokenError`].

use saga_session::TokenError;

/// Errors from the USB CCID / OpenPGP layer before SessionToken mapping.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UsbTokenError {
    #[error("token unavailable")]
    Unavailable,
    #[error("PIN required")]
    PinRequired,
    #[error("card locked")]
    CardLocked,
    #[error("card not supported: {0}")]
    CardNotSupported(String),
    #[error("token timeout")]
    Timeout,
    #[error("USB/OpenPGP protocol: {0}")]
    Protocol(String),
}

impl From<UsbTokenError> for TokenError {
    fn from(e: UsbTokenError) -> Self {
        match e {
            UsbTokenError::Unavailable => TokenError::Unavailable,
            UsbTokenError::PinRequired => TokenError::PinRequired,
            UsbTokenError::CardLocked => TokenError::CardLocked,
            UsbTokenError::CardNotSupported(_) => TokenError::CardNotSupported,
            UsbTokenError::Timeout => TokenError::Timeout,
            UsbTokenError::Protocol(s) => TokenError::Protocol(s),
        }
    }
}

impl UsbTokenError {
    pub fn from_status(sw1: u8, sw2: u8) -> Self {
        match (sw1, sw2) {
            (0x69, 0x82) => UsbTokenError::PinRequired,
            (0x69, 0x83) => UsbTokenError::CardLocked,
            (0x63, c) if (c & 0xF0) == 0xC0 => {
                let left = c & 0x0F;
                if left == 0 {
                    UsbTokenError::CardLocked
                } else {
                    UsbTokenError::PinRequired
                }
            }
            _ => UsbTokenError::Protocol(format!("card status {sw1:02X}{sw2:02X}")),
        }
    }
}
