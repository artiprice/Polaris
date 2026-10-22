//! Exercise arbitrary-input parsing and hash -> short marker -> parse round trips.
#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_sdk::ids::ShortRef;
use polaris_storage_cas::{FOLD_REF_LEN, content_id, fold_ref};
fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(short) = text.parse::<ShortRef>() {
            assert_eq!(short.to_marker().parse::<ShortRef>().ok(), Some(short));
        }
    }
    let cid = content_id(data);
    let short = ShortRef::of(&cid);
    let mut marker = [0; FOLD_REF_LEN];
    assert_eq!(fold_ref(&cid, &mut marker), FOLD_REF_LEN);
    let parsed = std::str::from_utf8(&marker)
        .ok()
        .and_then(|value| value.parse::<ShortRef>().ok());
    assert_eq!(parsed, Some(short));
});
