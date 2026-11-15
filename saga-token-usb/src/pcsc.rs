//! Host PC/SC [`CardFactory`] (feature `pcsc`).

use card_backend::CardBackend;
use card_backend_pcsc::PcscBackend;

use crate::transport::CardFactory;
use crate::UsbTokenError;

/// Open the first PC/SC OpenPGP reader (Nitrokey when plugged in).
pub struct PcscCardFactory {
    /// Optional reader name substring filter (e.g. `"Nitrokey"`).
    reader_filter: Option<String>,
}

impl PcscCardFactory {
    pub fn new() -> Self {
        Self {
            reader_filter: None,
        }
    }

    pub fn with_reader_filter(filter: impl Into<String>) -> Self {
        Self {
            reader_filter: Some(filter.into()),
        }
    }
}

impl Default for PcscCardFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl CardFactory for PcscCardFactory {
    fn open(&self) -> Result<Box<dyn CardBackend + Send + Sync>, UsbTokenError> {
        let cards = PcscBackend::cards(None)
            .map_err(|e| UsbTokenError::Protocol(format!("pcsc list readers failed: {e}")))?;
        // Take the first successful backend. Reader-name filter is reserved for
        // a future card-backend-pcsc API that exposes the reader string after open.
        let _ = &self.reader_filter;
        let Some(first) = cards.into_iter().next() else {
            return Err(UsbTokenError::Unavailable);
        };
        match first {
            Ok(backend) => Ok(Box::new(backend)),
            Err(e) => Err(UsbTokenError::Protocol(format!("pcsc connect: {e}"))),
        }
    }
}
