#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_security_capability::{Clock, Facts, PublicKeyBytes, ResourcePath, Right, Token, MAX_TOKEN_BLOCKS, authorize};
struct FixtureClock;
impl Clock for FixtureClock {
    fn now(&self) -> jiff::Timestamp { jiff::Timestamp::UNIX_EPOCH }
}
// Synthetic corpus format: public verification key followed by bearer bytes. The arbitrary key
// exercises key parsing; this is not a claim that attacker-supplied issuer keys are trusted.
fuzz_target!(|data: &[u8]| {
    let Some((key, encoded)) = data.split_at_checked(32) else { return; };
    let Ok(key) = <[u8;32]>::try_from(key) else { return; };
    let Ok(key) = PublicKeyBytes::from_bytes(key) else { return; };
    let Ok(token) = Token::from_bytes(encoded, &key) else { return; };
    assert!((1..=MAX_TOKEN_BLOCKS).contains(&token.block_count()));
    let Ok(resource) = ResourcePath::new("src/a.rs") else { return; };
    let input = Facts { resource: &resource, symbol: None, churn_lines: 0, touches_unsafe: false,
        diff_blake3: None, right: Right::Read };
    let _decision = authorize(&token, &input, &key, &FixtureClock);
});
