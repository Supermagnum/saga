//! Live PC/SC Nitrokey test — ignored unless SAGA_NITROKEY_PIN is set.

use std::sync::Arc;
use std::time::Duration;

use saga_session::{SessionToken, HANDSHAKE_HARD_TIMEOUT};
use saga_token_usb::pcsc::PcscCardFactory;
use saga_token_usb::{NitrokeySessionToken, StaticPinProvider};

#[tokio::test]
#[ignore = "requires Nitrokey on PC/SC and SAGA_NITROKEY_PIN"]
async fn live_nitrokey_reads_aut_and_offers() {
    let pin = std::env::var("SAGA_NITROKEY_PIN").expect("SAGA_NITROKEY_PIN");
    let factory = Arc::new(PcscCardFactory::new());
    let pin_provider = StaticPinProvider::from_pin_str(&pin).shared();
    let token = NitrokeySessionToken::try_open(factory, pin_provider).expect("open Nitrokey");
    eprintln!(
        "Nitrokey AUT curve={:?} fp={}",
        token.session_curve(),
        String::from_utf8_lossy(&token.fingerprint_hex())
    );
    let (init, _pending) = token
        .initiator_offer(token.session_curve(), HANDSHAKE_HARD_TIMEOUT)
        .await
        .expect("offer signed by card");
    assert_eq!(init.curve, token.session_curve());
    assert!(!init.signature.is_empty());
    // Keep the pending ephemeral for a short moment then drop (zeroise on ECDH path).
    let _ = Duration::from_millis(1);
}
