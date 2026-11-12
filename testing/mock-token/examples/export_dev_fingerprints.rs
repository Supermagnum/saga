fn main() {
    for label in ["15550100010", "15550100011", "15550100012"] {
        let id = mock_token::MockIdentity::from_label(label);
        let sec1 = id.verifying_sec1();
        println!(
            "{{\"label\":\"{label}\",\"curve\":\"BrainpoolP384r1\",\"fingerprint_hex\":\"{}\",\"sec1_hex\":\"{}\"}}",
            id.fingerprint_hex_string(),
            hex::encode(&sec1)
        );
    }
}
