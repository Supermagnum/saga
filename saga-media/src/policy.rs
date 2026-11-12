//! When Iroh media may start, and which wire mode is allowed.
//!
//! Encrypted → SRTP. Not-encrypted → plain RTP inside RoQ (QUIC still
//! encrypts the transport; no token/SRTP). Never silent-switch after a
//! call has been encrypted (§5b).

use crate::error::{MediaError, Result};

/// Wire mode for media datagrams after the 1-byte Saga media tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaWireMode {
    /// SRTP AEAD_AES_256_GCM (encrypted call).
    Srtp,
    /// Plain RTP (unsecured Iroh; QUIC/iroh endpoint keys only).
    PlainRtp,
}

/// Security state once the call has resolved (§5 step 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityResolution {
    Encrypted,
    /// Handshake failed / never completed; UI remains "Not encrypted".
    NotEncrypted {
        /// Contact has prior successful encrypted Iroh history.
        prior_encrypted_history: bool,
        /// User tapped continue on the §5a modal (ignored if no prior history).
        user_continued_past_5a: bool,
    },
}

/// Decide whether media may start and which wire mode to use.
///
/// - Encrypted → SRTP.
/// - Never-encrypted contact, not-encrypted outcome → plain RTP.
/// - Previously-encrypted contact → plain RTP only if user continued past §5a;
///   otherwise no media (user ended / dismissed).
pub fn media_permit(resolution: SecurityResolution) -> Result<MediaWireMode> {
    match resolution {
        SecurityResolution::Encrypted => Ok(MediaWireMode::Srtp),
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: false,
            ..
        } => Ok(MediaWireMode::PlainRtp),
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: true,
            user_continued_past_5a: true,
        } => Ok(MediaWireMode::PlainRtp),
        SecurityResolution::NotEncrypted {
            prior_encrypted_history: true,
            user_continued_past_5a: false,
        } => Err(MediaError::MediaDeclined),
    }
}

/// §5b: once a call ran encrypted, a later failure must not become plain RTP.
pub fn refuse_silent_downgrade(
    was_encrypted: bool,
    requested: MediaWireMode,
) -> Result<MediaWireMode> {
    if was_encrypted && requested == MediaWireMode::PlainRtp {
        return Err(MediaError::NoSilentDowngrade);
    }
    Ok(requested)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encrypted_permits_srtp() {
        assert_eq!(
            media_permit(SecurityResolution::Encrypted).unwrap(),
            MediaWireMode::Srtp
        );
    }

    #[test]
    fn never_encrypted_permits_plain() {
        assert_eq!(
            media_permit(SecurityResolution::NotEncrypted {
                prior_encrypted_history: false,
                user_continued_past_5a: false,
            })
            .unwrap(),
            MediaWireMode::PlainRtp
        );
    }

    #[test]
    fn prior_encrypted_without_continue_declines_media() {
        assert!(matches!(
            media_permit(SecurityResolution::NotEncrypted {
                prior_encrypted_history: true,
                user_continued_past_5a: false,
            }),
            Err(MediaError::MediaDeclined)
        ));
    }

    #[test]
    fn prior_encrypted_with_continue_permits_plain() {
        assert_eq!(
            media_permit(SecurityResolution::NotEncrypted {
                prior_encrypted_history: true,
                user_continued_past_5a: true,
            })
            .unwrap(),
            MediaWireMode::PlainRtp
        );
    }

    #[test]
    fn no_silent_downgrade_after_encrypted() {
        assert!(matches!(
            refuse_silent_downgrade(true, MediaWireMode::PlainRtp),
            Err(MediaError::NoSilentDowngrade)
        ));
        assert_eq!(
            refuse_silent_downgrade(true, MediaWireMode::Srtp).unwrap(),
            MediaWireMode::Srtp
        );
    }
}
