//! Hostile patch parsing must remain total and deterministic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use polaris_tools_builtin::apply_unified_patch;
fuzz_target!(|data: &[u8]| {
    if data.len() > 131_072 {
        return;
    }
    let midpoint = data.len() / 2;
    let Some(base) = data.get(..midpoint) else {
        return;
    };
    let Some(patch_bytes) = data.get(midpoint..) else {
        return;
    };
    let Ok(patch) = std::str::from_utf8(patch_bytes) else {
        return;
    };
    assert_eq!(
        apply_unified_patch(base, patch),
        apply_unified_patch(base, patch)
    );
});
