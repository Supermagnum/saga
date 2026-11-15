//! Phone-side ephemeral ECDH helpers for Nitrokey-style SessionToken backends.
//!
//! The Galdralag `ephemeral-session` wire format does not encode where ECDH ran.
//! Nitrokey cannot host the ephemeral private key, so the phone generates it
//! with [`crate::OsTrng`], signs the handshake preimage via the card AUT key,
//! then performs ECDH in process. Weaker than on-token Galdralag TRNG (sk in
//! phone RAM); forward secrecy per call still holds after zeroise.
//!
//! Session key derivation mirrors ephemeral-session's HKDF-Extract
//! (`profile_prk`) using the same salt rule (ordered ephemeral pubkeys) and
//! crates (`hkdf` / `hmac` / `sha2`). `EphemeralSharedSecret::derive_session_keys`
//! is crate-private in the pinned firmware rev, so the phone path repeats only
//! the Extract step Saga needs for [`HandshakeOutput`].

use ephemeral_session::{
    InitMessage, LongTermCert, ResponseMessage, SessionCurve, INIT_PROTOCOL_VERSION,
    RESP_PROTOCOL_VERSION,
};
use galdr_vault::brainpool::BrainpoolPublicKey;
use galdr_vault::brainpool::BrainpoolScalar;
use galdr_vault::brainpool384::{BrainpoolP384PublicKey, BrainpoolP384Scalar};
use galdr_vault::brainpool384::{BrainpoolP384Signature, BrainpoolP384VerifyingKey};
use galdr_vault::ecdsa_brainpool::{BrainpoolSignature, BrainpoolVerifyingKey};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::channel_bind::{bind_iroh_endpoints, transcript_hash};
use crate::confirm::derive_confirmation_key;
use crate::hal_os::OsTrng;
use crate::token::{HandshakeOutput, TokenError};

/// Phone-held ephemeral keypair (zeroised on drop / after ECDH).
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct PhoneEphemeral {
    curve_wire: u8,
    public_key: Vec<u8>,
    private_key: Zeroizing<Vec<u8>>,
}

impl PhoneEphemeral {
    pub fn generate(curve: SessionCurve, trng: &mut OsTrng) -> Result<Self, TokenError> {
        match curve {
            SessionCurve::BrainpoolP256r1 => {
                let sk = BrainpoolScalar::generate(trng)
                    .map_err(|_| TokenError::Protocol("ephemeral generate".into()))?;
                let pk = sk
                    .public_key()
                    .map_err(|_| TokenError::Protocol("ephemeral pubkey".into()))?;
                let public_key = pk.to_sec1_uncompressed().to_vec();
                let private_key = Zeroizing::new(sk.to_secret_bytes_for_test().to_vec());
                Ok(Self {
                    curve_wire: curve.wire_id(),
                    public_key,
                    private_key,
                })
            }
            SessionCurve::BrainpoolP384r1 => {
                let sk = BrainpoolP384Scalar::generate(trng)
                    .map_err(|_| TokenError::Protocol("ephemeral generate".into()))?;
                let pk = sk
                    .public_key()
                    .map_err(|_| TokenError::Protocol("ephemeral pubkey".into()))?;
                let public_key = pk.to_sec1_uncompressed().to_vec();
                let private_key = Zeroizing::new(sk.to_secret_bytes_for_test().to_vec());
                Ok(Self {
                    curve_wire: curve.wire_id(),
                    public_key,
                    private_key,
                })
            }
        }
    }

    pub fn curve(&self) -> Result<SessionCurve, TokenError> {
        SessionCurve::try_from_wire(self.curve_wire).map_err(|_| TokenError::CurveMismatch)
    }

    pub fn public_key_bytes(&self) -> &[u8] {
        &self.public_key
    }

    /// ECDH then HKDF-Extract → (`shared_secret` IKM, `profile_prk`).
    #[allow(clippy::type_complexity)]
    pub fn ecdh_profile_prk(
        self,
        peer_public_key: &[u8],
        epk_initiator: &[u8],
        epk_responder: &[u8],
    ) -> Result<(Zeroizing<Vec<u8>>, Zeroizing<[u8; 32]>), TokenError> {
        let curve = self.curve()?;
        let ikm = match curve {
            SessionCurve::BrainpoolP256r1 => {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&self.private_key[..32]);
                let s = BrainpoolScalar::from_secret_key_bytes_for_test(&arr)
                    .map_err(|_| TokenError::Protocol("ecdh sk".into()))?;
                let peer = BrainpoolPublicKey::from_sec1(peer_public_key)
                    .map_err(|_| TokenError::Protocol("ecdh peer".into()))?;
                let x = s
                    .diffie_hellman(&peer)
                    .map_err(|_| TokenError::Protocol("ecdh".into()))?;
                Zeroizing::new(x.as_bytes().to_vec())
            }
            SessionCurve::BrainpoolP384r1 => {
                let mut arr = [0u8; 48];
                arr.copy_from_slice(&self.private_key[..48]);
                let s = BrainpoolP384Scalar::from_secret_key_bytes_for_test(&arr)
                    .map_err(|_| TokenError::Protocol("ecdh sk".into()))?;
                let peer = BrainpoolP384PublicKey::from_sec1(peer_public_key)
                    .map_err(|_| TokenError::Protocol("ecdh peer".into()))?;
                let x = s
                    .diffie_hellman(&peer)
                    .map_err(|_| TokenError::Protocol("ecdh".into()))?;
                Zeroizing::new(x.as_bytes().to_vec())
            }
        };
        let salt = ordered_epk_salt(epk_initiator, epk_responder);
        let profile_prk = Zeroizing::new(hkdf_extract_sha256(&salt, ikm.as_slice()));
        Ok((ikm, profile_prk))
    }
}

fn ordered_epk_salt(epk_initiator: &[u8], epk_responder: &[u8]) -> Vec<u8> {
    let mut salt = Vec::with_capacity(epk_initiator.len() + epk_responder.len());
    if epk_initiator <= epk_responder {
        salt.extend_from_slice(epk_initiator);
        salt.extend_from_slice(epk_responder);
    } else {
        salt.extend_from_slice(epk_responder);
        salt.extend_from_slice(epk_initiator);
    }
    salt
}

fn hkdf_extract_sha256(salt: &[u8], ikm: &[u8]) -> [u8; 32] {
    type HmacSha256 = Hmac<Sha256>;
    if salt.is_empty() {
        let key = [0u8; 32];
        let mut mac = HmacSha256::new_from_slice(&key).expect("hmac");
        mac.update(ikm);
        return mac.finalize().into_bytes().into();
    }
    let mut mac = HmacSha256::new_from_slice(salt).expect("hmac");
    mac.update(ikm);
    mac.finalize().into_bytes().into()
}

/// Same bytes Galdralag `ephemeral-session` signs for [`InitMessage`].
pub fn init_sign_preimage(version: u8, curve: SessionCurve, epk: &[u8]) -> Vec<u8> {
    let mut p = Vec::with_capacity(2 + epk.len());
    p.push(version);
    p.push(curve.wire_id());
    p.extend_from_slice(epk);
    p
}

/// Same bytes Galdralag `ephemeral-session` signs for [`ResponseMessage`].
pub fn response_sign_preimage(
    version: u8,
    curve: SessionCurve,
    responder_epk: &[u8],
    initiator_epk: &[u8],
) -> Vec<u8> {
    let mut p = Vec::with_capacity(2 + responder_epk.len() + initiator_epk.len());
    p.push(version);
    p.push(curve.wire_id());
    p.extend_from_slice(responder_epk);
    p.extend_from_slice(initiator_epk);
    p
}

/// SHA-256 of the sign preimage — digest the OpenPGP card ECDSA-signs for INTERNAL AUTH.
pub fn handshake_digest(preimage: &[u8]) -> [u8; 32] {
    let d = Sha256::digest(preimage);
    let mut out = [0u8; 32];
    out.copy_from_slice(&d);
    out
}

/// Verify a DER ECDSA handshake signature under [`LongTermCert`] (SHA-256 prehash).
pub fn verify_handshake_signature(
    cert: &LongTermCert,
    preimage: &[u8],
    der_sig: &[u8],
) -> Result<(), TokenError> {
    match cert.curve {
        SessionCurve::BrainpoolP256r1 => {
            let vk = BrainpoolVerifyingKey::from_sec1(cert.verifying_key.as_slice())
                .map_err(|_| TokenError::BadSignature)?;
            let sig = BrainpoolSignature::from_der_bytes(der_sig)
                .map_err(|_| TokenError::BadSignature)?;
            vk.verify_handshake_sha256_prehash(preimage, &sig)
                .map_err(|_| TokenError::BadSignature)
        }
        SessionCurve::BrainpoolP384r1 => {
            let vk = BrainpoolP384VerifyingKey::from_sec1(cert.verifying_key.as_slice())
                .map_err(|_| TokenError::BadSignature)?;
            let sig = BrainpoolP384Signature::from_der_bytes(der_sig)
                .map_err(|_| TokenError::BadSignature)?;
            vk.verify_handshake_sha256_prehash(preimage, &sig)
                .map_err(|_| TokenError::BadSignature)
        }
    }
}

/// Build [`LongTermCert`] from uncompressed SEC1 + curve (fingerprint = SHA-256(SEC1)).
pub fn long_term_cert_from_sec1(
    curve: SessionCurve,
    sec1: &[u8],
) -> Result<LongTermCert, TokenError> {
    let fingerprint = LongTermCert::fingerprint_of(sec1);
    let mut verifying_key = heapless::Vec::new();
    verifying_key
        .extend_from_slice(sec1)
        .map_err(|_| TokenError::Protocol("SEC1 too long".into()))?;
    Ok(LongTermCert {
        fingerprint,
        curve,
        verifying_key,
    })
}

/// Wire fingerprint: 64 lowercase hex ASCII.
pub fn fingerprint_hex_from_sec1(sec1: &[u8]) -> [u8; 64] {
    let raw = LongTermCert::fingerprint_of(sec1);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(raw.as_slice());
    let hex = InitMessage::encode_fingerprint_hex(&arr);
    let mut out = [0u8; 64];
    out.copy_from_slice(hex.as_slice());
    out
}

fn pack_fp_hex(sec1: &[u8]) -> Result<heapless::Vec<u8, 64>, TokenError> {
    let raw = LongTermCert::fingerprint_of(sec1);
    let mut arr = [0u8; 32];
    arr.copy_from_slice(raw.as_slice());
    let hex = InitMessage::encode_fingerprint_hex(&arr);
    let mut out = heapless::Vec::new();
    out.extend_from_slice(hex.as_slice())
        .map_err(|_| TokenError::Protocol("fp hex".into()))?;
    Ok(out)
}

fn pack_sig(der: &[u8]) -> Result<heapless::Vec<u8, 200>, TokenError> {
    let mut signature = heapless::Vec::new();
    signature
        .extend_from_slice(der)
        .map_err(|_| TokenError::Protocol("signature too long".into()))?;
    Ok(signature)
}

fn pack_epk(epk: &[u8]) -> Result<heapless::Vec<u8, 129>, TokenError> {
    let mut ephemeral_public_key = heapless::Vec::new();
    ephemeral_public_key
        .extend_from_slice(epk)
        .map_err(|_| TokenError::Protocol("ephemeral pubkey too long".into()))?;
    Ok(ephemeral_public_key)
}

/// Assemble a signed [`InitMessage`] after the card returned a DER signature.
pub fn build_init_message(
    curve: SessionCurve,
    ephemeral_public_key: &[u8],
    local_sec1: &[u8],
    der_signature: &[u8],
) -> Result<InitMessage, TokenError> {
    Ok(InitMessage {
        version: INIT_PROTOCOL_VERSION,
        curve,
        ephemeral_public_key: pack_epk(ephemeral_public_key)?,
        long_term_fingerprint: pack_fp_hex(local_sec1)?,
        signature: pack_sig(der_signature)?,
    })
}

/// Assemble a signed [`ResponseMessage`].
pub fn build_response_message(
    curve: SessionCurve,
    responder_epk: &[u8],
    initiator_epk: &[u8],
    local_sec1: &[u8],
    der_signature: &[u8],
) -> Result<ResponseMessage, TokenError> {
    Ok(ResponseMessage {
        version: RESP_PROTOCOL_VERSION,
        curve,
        ephemeral_public_key: pack_epk(responder_epk)?,
        long_term_fingerprint: pack_fp_hex(local_sec1)?,
        initiator_ephemeral_public_key: pack_epk(initiator_epk)?,
        signature: pack_sig(der_signature)?,
    })
}

fn output_from_prk(
    shared_secret: Zeroizing<Vec<u8>>,
    profile_prk: Zeroizing<[u8; 32]>,
    init_epk: &[u8],
    resp_epk: &[u8],
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
) -> HandshakeOutput {
    let transcript_hash = transcript_hash(init_epk, resp_epk);
    let saga_session_key = bind_iroh_endpoints(&profile_prk, local_endpoint_id, remote_endpoint_id);
    let confirmation_key = Zeroizing::new(
        *derive_confirmation_key(&profile_prk, local_endpoint_id, remote_endpoint_id).as_bytes(),
    );
    HandshakeOutput {
        shared_secret,
        transcript_hash,
        profile_prk,
        saga_session_key,
        confirmation_key,
    }
}

/// Verify peer [`ResponseMessage`] fields that bind the session, then ECDH.
pub fn initiator_finish_phone(
    ephemeral: PhoneEphemeral,
    init_message: &InitMessage,
    response: &ResponseMessage,
    peer_cert: &LongTermCert,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
) -> Result<HandshakeOutput, TokenError> {
    if response.version != RESP_PROTOCOL_VERSION {
        return Err(TokenError::TruncatedMessage);
    }
    if response.curve != init_message.curve {
        return Err(TokenError::CurveMismatch);
    }
    let my_epk = init_message.ephemeral_public_key.as_slice();
    if my_epk.len() != response.initiator_ephemeral_public_key.len()
        || bool::from(!my_epk.ct_eq(response.initiator_ephemeral_public_key.as_slice()))
    {
        return Err(TokenError::BadSignature);
    }
    let wire_fp = InitMessage::decode_fingerprint_hex(response.long_term_fingerprint.as_slice())
        .map_err(|_| TokenError::TruncatedMessage)?;
    if bool::from(!peer_cert.fingerprint.as_slice().ct_eq(&wire_fp)) {
        return Err(TokenError::UnknownSigner);
    }
    let resp_pre = response_sign_preimage(
        RESP_PROTOCOL_VERSION,
        response.curve,
        response.ephemeral_public_key.as_slice(),
        response.initiator_ephemeral_public_key.as_slice(),
    );
    verify_handshake_signature(peer_cert, &resp_pre, response.signature.as_slice())?;
    let (ikm, profile_prk) = ephemeral.ecdh_profile_prk(
        response.ephemeral_public_key.as_slice(),
        init_message.ephemeral_public_key.as_slice(),
        response.ephemeral_public_key.as_slice(),
    )?;
    Ok(output_from_prk(
        ikm,
        profile_prk,
        init_message.ephemeral_public_key.as_slice(),
        response.ephemeral_public_key.as_slice(),
        local_endpoint_id,
        remote_endpoint_id,
    ))
}

/// Verify peer [`InitMessage`].
pub fn verify_init_message(init: &InitMessage, peer_cert: &LongTermCert) -> Result<(), TokenError> {
    if init.version != INIT_PROTOCOL_VERSION {
        return Err(TokenError::TruncatedMessage);
    }
    if init.curve != peer_cert.curve {
        return Err(TokenError::CurveMismatch);
    }
    let wire_fp = InitMessage::decode_fingerprint_hex(init.long_term_fingerprint.as_slice())
        .map_err(|_| TokenError::TruncatedMessage)?;
    if bool::from(!peer_cert.fingerprint.as_slice().ct_eq(&wire_fp)) {
        return Err(TokenError::UnknownSigner);
    }
    let init_pre = init_sign_preimage(
        init.version,
        init.curve,
        init.ephemeral_public_key.as_slice(),
    );
    verify_handshake_signature(peer_cert, &init_pre, init.signature.as_slice())
}

/// Complete responder ECDH after the response signature is produced.
pub fn responder_ecdh_output(
    ephemeral: PhoneEphemeral,
    init: &InitMessage,
    response: &ResponseMessage,
    local_endpoint_id: &[u8],
    remote_endpoint_id: &[u8],
) -> Result<HandshakeOutput, TokenError> {
    let (ikm, profile_prk) = ephemeral.ecdh_profile_prk(
        init.ephemeral_public_key.as_slice(),
        init.ephemeral_public_key.as_slice(),
        response.ephemeral_public_key.as_slice(),
    )?;
    Ok(output_from_prk(
        ikm,
        profile_prk,
        init.ephemeral_public_key.as_slice(),
        response.ephemeral_public_key.as_slice(),
        local_endpoint_id,
        remote_endpoint_id,
    ))
}
