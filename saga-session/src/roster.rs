//! Mesh call roster message: who is in the call, authenticated per link.
//!
//! The roster travels over a second bi-stream on the already-established
//! voice `Connection` (QUIC/TLS protects confidentiality of the hop) with
//! u16-length framing, the same framing as the handshake. Integrity and
//! binding to this pairwise session come from HMAC-SHA256 under a key
//! expanded with HKDF from `saga_session_key` using the label
//! `saga-mesh-roster-v1` (plus the transcript hash, as the SRTP labels do).
//! Only `hkdf`, `hmac`, `sha2` and `subtle` are used: no new primitive.
//!
//! Wire body (before the 32-byte tag):
//!
//! ```text
//! version u8 | flags u8 (bit0 = introducer present) | count u8
//! count x ( endpoint_id [32] | fingerprint [32] )
//! introducer endpoint_id [32]   (only when flags bit0 is set)
//! ```
//!
//! A framed message is `u16 big-endian length | body | tag[32]` where the
//! length covers body plus tag.

use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

use crate::token::HandshakeOutput;

/// Roster wire version.
pub const ROSTER_VERSION: u8 = 1;
/// Maximum participants in a mesh call.
pub const ROSTER_MAX_PARTICIPANTS: usize = 4;
/// HMAC-SHA256 tag length.
pub const ROSTER_TAG_LEN: usize = 32;

const ROSTER_LABEL: &[u8] = b"saga-mesh-roster-v1";
const FLAG_INTRODUCER: u8 = 0b0000_0001;
const ENTRY_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RosterError {
    TooManyParticipants,
    Truncated,
    BadVersion(u8),
    BadFlags(u8),
    BadTag,
    TooLarge,
    Io(String),
}

impl std::fmt::Display for RosterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RosterError::TooManyParticipants => {
                write!(f, "roster exceeds {ROSTER_MAX_PARTICIPANTS} participants")
            }
            RosterError::Truncated => write!(f, "roster message truncated"),
            RosterError::BadVersion(v) => write!(f, "unsupported roster version {v}"),
            RosterError::BadFlags(v) => write!(f, "unknown roster flags {v:#04x}"),
            RosterError::BadTag => write!(f, "roster authentication tag mismatch"),
            RosterError::TooLarge => write!(f, "roster message too large"),
            RosterError::Io(e) => write!(f, "roster io: {e}"),
        }
    }
}

impl std::error::Error for RosterError {}

/// One roster participant: Iroh EndpointId plus raw identity fingerprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub endpoint_id: [u8; 32],
    pub fingerprint: [u8; 32],
}

/// Call roster as announced by one peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Roster {
    pub version: u8,
    pub participants: Vec<RosterEntry>,
    /// EndpointId of the participant who added the sender (if any).
    pub introducer: Option<[u8; 32]>,
}

impl Roster {
    pub fn new(
        participants: Vec<RosterEntry>,
        introducer: Option<[u8; 32]>,
    ) -> Result<Self, RosterError> {
        if participants.len() > ROSTER_MAX_PARTICIPANTS {
            return Err(RosterError::TooManyParticipants);
        }
        Ok(Self {
            version: ROSTER_VERSION,
            participants,
            introducer,
        })
    }

    fn encode_body(&self) -> Vec<u8> {
        let mut b = Vec::with_capacity(3 + self.participants.len() * ENTRY_LEN + 32);
        b.push(self.version);
        b.push(if self.introducer.is_some() {
            FLAG_INTRODUCER
        } else {
            0
        });
        b.push(self.participants.len() as u8);
        for p in &self.participants {
            b.extend_from_slice(&p.endpoint_id);
            b.extend_from_slice(&p.fingerprint);
        }
        if let Some(i) = &self.introducer {
            b.extend_from_slice(i);
        }
        b
    }

    fn decode_body(body: &[u8]) -> Result<Self, RosterError> {
        if body.len() < 3 {
            return Err(RosterError::Truncated);
        }
        let version = body[0];
        if version != ROSTER_VERSION {
            return Err(RosterError::BadVersion(version));
        }
        let flags = body[1];
        if flags & !FLAG_INTRODUCER != 0 {
            return Err(RosterError::BadFlags(flags));
        }
        let count = body[2] as usize;
        if count > ROSTER_MAX_PARTICIPANTS {
            return Err(RosterError::TooManyParticipants);
        }
        let has_intro = flags & FLAG_INTRODUCER != 0;
        let expect = 3 + count * ENTRY_LEN + if has_intro { 32 } else { 0 };
        if body.len() != expect {
            return Err(RosterError::Truncated);
        }
        let mut participants = Vec::with_capacity(count);
        let mut off = 3;
        for _ in 0..count {
            let mut endpoint_id = [0u8; 32];
            let mut fingerprint = [0u8; 32];
            endpoint_id.copy_from_slice(&body[off..off + 32]);
            fingerprint.copy_from_slice(&body[off + 32..off + 64]);
            participants.push(RosterEntry {
                endpoint_id,
                fingerprint,
            });
            off += ENTRY_LEN;
        }
        let introducer = if has_intro {
            let mut i = [0u8; 32];
            i.copy_from_slice(&body[off..off + 32]);
            Some(i)
        } else {
            None
        };
        Ok(Self {
            version,
            participants,
            introducer,
        })
    }

    /// Serialize and authenticate for one link: `body | tag`.
    pub fn seal(&self, link: &HandshakeOutput) -> Vec<u8> {
        let mut msg = self.encode_body();
        let tag = tag_for(&roster_key(link), &msg);
        msg.extend_from_slice(&tag);
        msg
    }

    /// Verify the tag (constant time) and parse. `msg` is `body | tag`.
    pub fn open(link: &HandshakeOutput, msg: &[u8]) -> Result<Self, RosterError> {
        if msg.len() < ROSTER_TAG_LEN + 3 {
            return Err(RosterError::Truncated);
        }
        let (body, tag) = msg.split_at(msg.len() - ROSTER_TAG_LEN);
        let mut mac = new_mac(&roster_key(link));
        mac.update(body);
        mac.verify_slice(tag).map_err(|_| RosterError::BadTag)?;
        Self::decode_body(body)
    }
}

/// HKDF-Expand(saga_session_key, "saga-mesh-roster-v1" || transcript_hash).
fn roster_key(link: &HandshakeOutput) -> Zeroizing<[u8; 32]> {
    let hk = Hkdf::<Sha256>::new(None, link.saga_session_key.as_slice());
    let mut info = Vec::with_capacity(ROSTER_LABEL.len() + link.transcript_hash.len());
    info.extend_from_slice(ROSTER_LABEL);
    info.extend_from_slice(&link.transcript_hash);
    let mut out = Zeroizing::new([0u8; 32]);
    hk.expand(&info, out.as_mut())
        .expect("HKDF-Expand 32 bytes fits SHA-256");
    out
}

fn new_mac(key: &[u8; 32]) -> Hmac<Sha256> {
    Hmac::<Sha256>::new_from_slice(key).expect("hmac accepts any key length")
}

fn tag_for(key: &[u8; 32], body: &[u8]) -> [u8; ROSTER_TAG_LEN] {
    let mut mac = new_mac(key);
    mac.update(body);
    let mut out = [0u8; ROSTER_TAG_LEN];
    out.copy_from_slice(&mac.finalize().into_bytes());
    out
}

/// Write one sealed roster with a u16 big-endian length prefix.
pub async fn write_roster<W>(
    writer: &mut W,
    link: &HandshakeOutput,
    roster: &Roster,
) -> Result<(), RosterError>
where
    W: AsyncWrite + Unpin,
{
    let msg = roster.seal(link);
    let len = u16::try_from(msg.len()).map_err(|_| RosterError::TooLarge)?;
    writer
        .write_all(&len.to_be_bytes())
        .await
        .map_err(|e| RosterError::Io(e.to_string()))?;
    writer
        .write_all(&msg)
        .await
        .map_err(|e| RosterError::Io(e.to_string()))?;
    writer
        .flush()
        .await
        .map_err(|e| RosterError::Io(e.to_string()))
}

/// Read and verify one length-prefixed roster.
pub async fn read_roster<R>(reader: &mut R, link: &HandshakeOutput) -> Result<Roster, RosterError>
where
    R: AsyncRead + Unpin,
{
    let mut lenb = [0u8; 2];
    reader
        .read_exact(&mut lenb)
        .await
        .map_err(|e| RosterError::Io(e.to_string()))?;
    let len = u16::from_be_bytes(lenb) as usize;
    let max = 3 + ROSTER_MAX_PARTICIPANTS * ENTRY_LEN + 32 + ROSTER_TAG_LEN;
    if len > max {
        return Err(RosterError::TooLarge);
    }
    let mut msg = vec![0u8; len];
    reader
        .read_exact(&mut msg)
        .await
        .map_err(|e| RosterError::Io(e.to_string()))?;
    Roster::open(link, &msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(seed: u8) -> HandshakeOutput {
        HandshakeOutput {
            shared_secret: Zeroizing::new(vec![seed; 48]),
            transcript_hash: [seed.wrapping_add(1); 32],
            profile_prk: Zeroizing::new([seed.wrapping_add(2); 32]),
            saga_session_key: Zeroizing::new([seed.wrapping_add(3); 32]),
            confirmation_key: Zeroizing::new([seed.wrapping_add(4); 32]),
        }
    }

    fn sample(n: usize, intro: bool) -> Roster {
        let participants = (0..n)
            .map(|i| RosterEntry {
                endpoint_id: [i as u8 + 1; 32],
                fingerprint: [i as u8 + 0x80; 32],
            })
            .collect();
        Roster::new(participants, intro.then_some([0xEE; 32])).unwrap()
    }

    #[test]
    fn round_trip_with_and_without_introducer() {
        for (n, intro) in [(0, false), (2, false), (4, true), (3, true)] {
            let r = sample(n, intro);
            let sealed = r.seal(&link(1));
            assert_eq!(Roster::open(&link(1), &sealed).unwrap(), r);
        }
    }

    #[test]
    fn wrong_link_key_rejected() {
        let sealed = sample(3, true).seal(&link(1));
        assert_eq!(
            Roster::open(&link(2), &sealed).unwrap_err(),
            RosterError::BadTag
        );
    }

    #[test]
    fn tamper_rejected() {
        let mut sealed = sample(3, false).seal(&link(1));
        sealed[5] ^= 0x01;
        assert_eq!(
            Roster::open(&link(1), &sealed).unwrap_err(),
            RosterError::BadTag
        );
    }

    #[test]
    fn fifth_participant_refused() {
        let participants = (0..5)
            .map(|i| RosterEntry {
                endpoint_id: [i as u8; 32],
                fingerprint: [0; 32],
            })
            .collect();
        assert_eq!(
            Roster::new(participants, None).unwrap_err(),
            RosterError::TooManyParticipants
        );
    }

    #[test]
    fn key_label_differs_from_confirmation_and_session_key() {
        let l = link(1);
        assert_ne!(roster_key(&l).as_slice(), l.saga_session_key.as_slice());
        assert_ne!(roster_key(&l).as_slice(), l.confirmation_key.as_slice());
    }

    #[tokio::test]
    async fn framed_stream_round_trip() {
        let r = sample(4, true);
        let l = link(9);
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_roster(&mut a, &l, &r).await.unwrap();
        assert_eq!(read_roster(&mut b, &l).await.unwrap(), r);
    }
}
