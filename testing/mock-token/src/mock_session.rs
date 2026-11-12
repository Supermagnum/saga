//! [`SessionToken`] implementation backed by in-process ephemeral-session + FakeTrng.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use ephemeral_session::{
    InitMessage, InitiatorSession, LongTermCert, ResponderSession, ResponseMessage, SessionCurve,
    SessionKeys,
};
use galdr_core::fake_hal::FakeTrng;
use zeroize::Zeroizing;

use crate::channel_bind::{bind_iroh_endpoints, transcript_hash};
use crate::confirm::derive_confirmation_key;
use crate::identity::{MockIdentity, LOCAL_SLOT};
use crate::token::{HandshakeOutput, InitiatorPending, SessionToken, TokenError};

#[cfg(debug_assertions)]
use crate::token::MOCK_TOKEN_MARKER;
#[cfg(debug_assertions)]
use log::info;

/// Software mock token. Ephemeral ECDH runs in-process via FakeTrng, mirroring
/// on-device steps without USB.
pub struct MockSessionToken {
    identity: MockIdentity,
    /// Deterministic ephemeral TRNG seed (tests / vectors). `None` = fresh from label hash.
    ephemeral_seed: Option<u64>,
    /// Serialised InitMessages already answered (replay detection for negative tests).
    seen_inits: Mutex<HashSet<Vec<u8>>>,
}

impl MockSessionToken {
    pub fn new(identity: MockIdentity) -> Self {
        Self {
            identity,
            ephemeral_seed: None,
            seen_inits: Mutex::new(HashSet::new()),
        }
    }

    /// Fixed FakeTrng seed for ephemeral key generation (test vectors).
    pub fn with_ephemeral_seed(identity: MockIdentity, seed: u64) -> Self {
        Self {
            identity,
            ephemeral_seed: Some(seed),
            seen_inits: Mutex::new(HashSet::new()),
        }
    }

    pub fn identity(&self) -> &MockIdentity {
        &self.identity
    }

    fn output_from_keys(
        keys: &SessionKeys,
        init_epk: &[u8],
        resp_epk: &[u8],
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
    ) -> HandshakeOutput {
        let shared_secret = Zeroizing::new(keys.cess_inner_cascade_ikm().to_vec());
        let transcript_hash = transcript_hash(init_epk, resp_epk);
        let profile_prk = Zeroizing::new(*keys.profile_prk());
        let saga_session_key =
            bind_iroh_endpoints(&profile_prk, local_endpoint_id, remote_endpoint_id);
        let confirmation_key = Zeroizing::new(
            *derive_confirmation_key(&profile_prk, local_endpoint_id, remote_endpoint_id)
                .as_bytes(),
        );
        HandshakeOutput {
            shared_secret,
            transcript_hash,
            profile_prk,
            saga_session_key,
            confirmation_key,
        }
    }
}

async fn with_timeout<T, F>(timeout: Duration, work: F) -> Result<T, TokenError>
where
    F: FnOnce() -> Result<T, TokenError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(work)).await {
        Ok(Ok(inner)) => inner,
        Ok(Err(join)) => Err(TokenError::Protocol(format!("token task join: {join}"))),
        Err(_) => Err(TokenError::Timeout),
    }
}

#[async_trait]
impl SessionToken for MockSessionToken {
    fn long_term_cert(&self) -> LongTermCert {
        self.identity.long_term_cert()
    }

    fn fingerprint_hex(&self) -> [u8; 64] {
        self.identity.fingerprint_hex()
    }

    async fn initiator_offer(
        &self,
        curve: SessionCurve,
        timeout: Duration,
    ) -> Result<(InitMessage, InitiatorPending), TokenError> {
        let identity = self.identity.clone();
        let seed_base = self.ephemeral_seed;
        with_timeout(timeout, move || {
            if curve != identity.curve {
                return Err(TokenError::CurveMismatch);
            }
            let mut storage = identity.vault_with_self();
            let mut trng = if let Some(s) = seed_base {
                FakeTrng::from_seed(s)
            } else {
                let material = format!("saga-mock-eph-init:{}", identity.label);
                let hash = blake3::hash(material.as_bytes());
                FakeTrng::from_seed(u64::from_le_bytes(
                    hash.as_bytes()[0..8].try_into().expect("8"),
                ))
            };
            let mut session = InitiatorSession::new();
            let init = session
                .init(curve, &LOCAL_SLOT, &mut trng, &mut storage)
                .map_err(TokenError::from)?;
            let init_ephemeral_public_key = init.ephemeral_public_key.as_slice().to_vec();
            Ok((
                init,
                InitiatorPending {
                    inner: session,
                    curve,
                    init_ephemeral_public_key,
                },
            ))
        })
        .await
    }

    async fn initiator_finish(
        &self,
        mut pending: InitiatorPending,
        response: ResponseMessage,
        peer_cert: LongTermCert,
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
        timeout: Duration,
    ) -> Result<HandshakeOutput, TokenError> {
        let local = local_endpoint_id.to_vec();
        let remote = remote_endpoint_id.to_vec();
        with_timeout(timeout, move || {
            let init_epk = pending.init_ephemeral_public_key.clone();
            let keys = pending
                .inner
                .complete(&response, &peer_cert)
                .map_err(TokenError::from)?;
            let resp_epk = response.ephemeral_public_key.as_slice();
            let out =
                MockSessionToken::output_from_keys(&keys, &init_epk, resp_epk, &local, &remote);
            #[cfg(debug_assertions)]
            {
                let (a, b) = out.session_key_debug_prefix();
                info!(
                    "[{MOCK_TOKEN_MARKER}] initiator finish session_key_hash_prefix={a:02x}{b:02x}"
                );
            }
            Ok(out)
        })
        .await
    }

    async fn responder_reply(
        &self,
        init: InitMessage,
        peer_cert: LongTermCert,
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
        timeout: Duration,
    ) -> Result<(ResponseMessage, HandshakeOutput), TokenError> {
        let serial = init
            .serialise()
            .map_err(TokenError::from)?
            .as_slice()
            .to_vec();
        {
            let mut seen = self
                .seen_inits
                .lock()
                .map_err(|_| TokenError::Protocol("replay lock poisoned".into()))?;
            if !seen.insert(serial) {
                return Err(TokenError::Replay);
            }
        }

        let identity = self.identity.clone();
        let seed_base = self.ephemeral_seed;
        let local = local_endpoint_id.to_vec();
        let remote = remote_endpoint_id.to_vec();
        with_timeout(timeout, move || {
            if init.curve != identity.curve {
                return Err(TokenError::CurveMismatch);
            }
            let mut storage = identity.vault_with_self();
            let mut trng = if let Some(s) = seed_base {
                FakeTrng::from_seed(s.wrapping_add(1))
            } else {
                let material = format!("saga-mock-eph-resp:{}", identity.label);
                let hash = blake3::hash(material.as_bytes());
                FakeTrng::from_seed(u64::from_le_bytes(
                    hash.as_bytes()[0..8].try_into().expect("8"),
                ))
            };
            let init_epk = init.ephemeral_public_key.as_slice().to_vec();
            let (resp, keys) =
                ResponderSession::respond(&init, &LOCAL_SLOT, &peer_cert, &mut trng, &mut storage)
                    .map_err(TokenError::from)?;
            let resp_epk = resp.ephemeral_public_key.as_slice();
            let out =
                MockSessionToken::output_from_keys(&keys, &init_epk, resp_epk, &local, &remote);
            #[cfg(debug_assertions)]
            {
                let (a, b) = out.session_key_debug_prefix();
                info!(
                    "[{MOCK_TOKEN_MARKER}] responder [{}] session_key_hash_prefix={a:02x}{b:02x}",
                    identity.label
                );
            }
            Ok((resp, out))
        })
        .await
    }
}
