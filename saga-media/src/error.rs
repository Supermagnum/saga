use thiserror::Error;

/// Typed media-path failures. Never panic on the live path.
#[derive(Debug, Error)]
pub enum MediaError {
    #[error("QUIC datagrams unsupported on this connection")]
    DatagramsUnsupported,
    #[error("SRTP packet exceeds max datagram size ({need} > {limit})")]
    DatagramTooLarge { need: usize, limit: usize },
    #[error("media failed: {0}")]
    Failed(String),
    #[error(transparent)]
    Srtp(#[from] webrtc_srtp::Error),
    #[error(transparent)]
    Opus(#[from] opus::Error),
    #[error("rtp/util: {0}")]
    RtpUtil(String),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<webrtc_util::Error> for MediaError {
    fn from(e: webrtc_util::Error) -> Self {
        MediaError::RtpUtil(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, MediaError>;
