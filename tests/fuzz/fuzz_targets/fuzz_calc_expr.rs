#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_tools_math::{CalcLimits, calc, calc_f64};

const MAX_INPUT_BYTES: usize = 4 * 1024;

fuzz_target!(|data: &[u8]| {
    let input = data.get(..data.len().min(MAX_INPUT_BYTES)).unwrap_or(data);
    let Ok(expression) = std::str::from_utf8(input) else {
        return;
    };
    let limits = CalcLimits {
        max_len: MAX_INPUT_BYTES,
        ..CalcLimits::default()
    };
    let _ = calc(expression, &limits);
    if let Ok(value) = calc_f64(expression, &limits) {
        assert!(value.is_finite());
    }
});
