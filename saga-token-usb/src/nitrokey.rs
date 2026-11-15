//! [`NitrokeySessionToken`] — production SessionToken with phone ECDH + card AUT sign.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use saga_session::{
    build_init_message, build_response_message, fingerprint_hex_from_sec1, handshake_digest,
    init_sign_preimage, initiator_finish_phone, long_term_cert_from_sec1, responder_ecdh_output,
    response_sign_preimage, verify_init_message, HandshakeOutput, InitMessage, InitiatorPending,
    LongTermCert, OsTrng, PhoneEphemeral, ResponseMessage, SessionCurve, SessionToken, TokenError,
    INIT_PROTOCOL_VERSION, RESP_PROTOCOL_VERSION,
};
use zeroize::Zeroizing;

use crate::pin::PinProvider;
use crate::transport::{OpenPgpAutCard, SharedCardFactory};

/// Signs SHA-256 digests with the long-term AUT key (card or software test double).
pub trait AutSigner: Send + Sync {
    fn curve(&self) -> SessionCurve;
    fn sec1(&self) -> &[u8];
    fn sign_digest(&self, digest: &[u8]) -> Result<Vec<u8>, TokenError>;
}

/// OpenPGP-card AUT signer (INTERNAL AUTHENTICATE, ECC digest as-is).
pub struct OpenPgpAutSigner {
    factory: SharedCardFactory,
    pin: Arc<dyn PinProvider>,
    curve: SessionCurve,
    sec1: Vec<u8>,
}

impl OpenPgpAutSigner {
    pub fn try_open(
        factory: SharedCardFactory,
        pin: Arc<dyn PinProvider>,
    ) -> Result<Self, TokenError> {
        let backend = factory.open().map_err(TokenError::from)?;
        let (curve, sec1) = OpenPgpAutCard::read_aut_identity(backend).map_err(TokenError::from)?;
        Ok(Self {
            factory,
            pin,
            curve,
            sec1,
        })
    }
}

impl AutSigner for OpenPgpAutSigner {
    fn curve(&self) -> SessionCurve {
        self.curve
    }

    fn sec1(&self) -> &[u8] {
        &self.sec1
    }

    fn sign_digest(&self, digest: &[u8]) -> Result<Vec<u8>, TokenError> {
        let pin = self.pin.user_pin().map_err(TokenError::from)?;
        let backend = self.factory.open().map_err(TokenError::from)?;
        OpenPgpAutCard::internal_authenticate(backend, &pin, digest).map_err(TokenError::from)
    }
}

/// Production Nitrokey / OpenPGP-card SessionToken.
///
/// Ephemeral ECDH runs on the phone ([`OsTrng`]); the card signs with AUT via
/// INTERNAL AUTHENTICATE. See crate-level docs for the security trade-off.
pub struct NitrokeySessionToken {
    signer: Arc<dyn AutSigner>,
    cert: LongTermCert,
    fp_hex: [u8; 64],
}

impl NitrokeySessionToken {
    /// Build from any [`AutSigner`] (OpenPGP card or software test double).
    pub fn from_signer(signer: Arc<dyn AutSigner>) -> Result<Self, TokenError> {
        let cert = long_term_cert_from_sec1(signer.curve(), signer.sec1())?;
        let fp_hex = fingerprint_hex_from_sec1(signer.sec1());
        Ok(Self {
            signer,
            cert,
            fp_hex,
        })
    }

    /// Open a physical/virtual OpenPGP card via [`CardFactory`].
    pub fn try_open(
        factory: SharedCardFactory,
        pin: Arc<dyn PinProvider>,
    ) -> Result<Self, TokenError> {
        let signer = OpenPgpAutSigner::try_open(factory, pin)?;
        Self::from_signer(Arc::new(signer))
    }

    pub fn session_curve(&self) -> SessionCurve {
        self.signer.curve()
    }

    pub fn sec1_public(&self) -> &[u8] {
        self.signer.sec1()
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
impl SessionToken for NitrokeySessionToken {
    fn long_term_cert(&self) -> LongTermCert {
        self.cert.clone()
    }

    fn fingerprint_hex(&self) -> [u8; 64] {
        self.fp_hex
    }

    async fn initiator_offer(
        &self,
        curve: SessionCurve,
        timeout: Duration,
    ) -> Result<(InitMessage, InitiatorPending), TokenError> {
        let signer = Arc::clone(&self.signer);
        with_timeout(timeout, move || {
            if curve != signer.curve() {
                return Err(TokenError::CurveMismatch);
            }
            let mut trng = OsTrng::new();
            let ephemeral = PhoneEphemeral::generate(curve, &mut trng)?;
            let pre =
                init_sign_preimage(INIT_PROTOCOL_VERSION, curve, ephemeral.public_key_bytes());
            let digest = handshake_digest(&pre);
            let der = signer.sign_digest(&digest)?;
            let init =
                build_init_message(curve, ephemeral.public_key_bytes(), signer.sec1(), &der)?;
            let pending = InitiatorPending::from_phone_ephemeral(ephemeral, init.clone(), curve);
            Ok((init, pending))
        })
        .await
    }

    async fn initiator_finish(
        &self,
        pending: InitiatorPending,
        response: ResponseMessage,
        peer_cert: LongTermCert,
        local_endpoint_id: &[u8],
        remote_endpoint_id: &[u8],
        timeout: Duration,
    ) -> Result<HandshakeOutput, TokenError> {
        let local = local_endpoint_id.to_vec();
        let remote = remote_endpoint_id.to_vec();
        with_timeout(timeout, move || {
            let (ephemeral, init_message) = pending.take_phone_ephemeral()?;
            initiator_finish_phone(
                ephemeral,
                &init_message,
                &response,
                &peer_cert,
                &local,
                &remote,
            )
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
        let signer = Arc::clone(&self.signer);
        let local = local_endpoint_id.to_vec();
        let remote = remote_endpoint_id.to_vec();
        with_timeout(timeout, move || {
            if init.curve != signer.curve() {
                return Err(TokenError::CurveMismatch);
            }
            verify_init_message(&init, &peer_cert)?;
            let mut trng = OsTrng::new();
            let ephemeral = PhoneEphemeral::generate(signer.curve(), &mut trng)?;
            let pre = response_sign_preimage(
                RESP_PROTOCOL_VERSION,
                signer.curve(),
                ephemeral.public_key_bytes(),
                init.ephemeral_public_key.as_slice(),
            );
            let digest = handshake_digest(&pre);
            let der = signer.sign_digest(&digest)?;
            let response = build_response_message(
                signer.curve(),
                ephemeral.public_key_bytes(),
                init.ephemeral_public_key.as_slice(),
                signer.sec1(),
                &der,
            )?;
            let out = responder_ecdh_output(ephemeral, &init, &response, &local, &remote)?;
            Ok((response, out))
        })
        .await
    }
}

/// Re-export for PIN zeroize in Android bridges.
pub type PinBytes = Zeroizing<Vec<u8>>;
