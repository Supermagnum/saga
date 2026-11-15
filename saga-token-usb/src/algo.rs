//! OpenPGP AUT algorithm-attribute checks (Brainpool only).

use openpgp_card::ocard::algorithm::{AlgorithmAttributes, Curve};
use openpgp_card::ocard::crypto::EccType;
use saga_session::SessionCurve;

use crate::UsbTokenError;

/// Brainpool OID bytes as used by OpenPGP card algorithm attributes.
pub mod brainpool_oid {
    /// brainpoolP256r1 — 1.3.36.3.3.2.8.1.1.7
    pub const P256R1: &[u8] = &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x07];
    /// brainpoolP384r1 — 1.3.36.3.3.2.8.1.1.11
    pub const P384R1: &[u8] = &[0x2b, 0x24, 0x03, 0x03, 0x02, 0x08, 0x01, 0x01, 0x0b];
}

/// AUT key is a supported Brainpool ECDSA curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrainpoolAutCurve {
    pub session: SessionCurve,
}

/// Map OpenPGP algorithm attributes to a Saga [`SessionCurve`], or reject.
pub fn curve_from_algorithm_attributes(
    attrs: &AlgorithmAttributes,
) -> Result<BrainpoolAutCurve, UsbTokenError> {
    match attrs {
        AlgorithmAttributes::Ecc(ecc) => {
            if ecc.ecc_type() != EccType::ECDSA {
                return Err(UsbTokenError::CardNotSupported(format!(
                    "AUT ecc type {:?} (need ECDSA)",
                    ecc.ecc_type()
                )));
            }
            map_curve(ecc.curve())
        }
        other => Err(UsbTokenError::CardNotSupported(format!(
            "AUT attributes {other:?} (need ECDSA Brainpool)"
        ))),
    }
}

fn map_curve(curve: &Curve) -> Result<BrainpoolAutCurve, UsbTokenError> {
    match curve {
        Curve::BrainpoolP256r1 => Ok(BrainpoolAutCurve {
            session: SessionCurve::BrainpoolP256r1,
        }),
        Curve::BrainpoolP384r1 => Ok(BrainpoolAutCurve {
            session: SessionCurve::BrainpoolP384r1,
        }),
        other => Err(UsbTokenError::CardNotSupported(format!(
            "AUT curve {other:?} (need brainpoolP256r1 or brainpoolP384r1)"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_lengths_match_openpgp_card_docs() {
        assert_eq!(brainpool_oid::P256R1.len(), 9);
        assert_eq!(brainpool_oid::P384R1.len(), 9);
    }
}
