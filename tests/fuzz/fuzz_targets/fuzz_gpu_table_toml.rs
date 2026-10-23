#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_tools_math::GpuSpecTable;

const MAX_TOML_BYTES: usize = 1 << 20;

fuzz_target!(|data: &[u8]| {
    let input = data.get(..data.len().min(MAX_TOML_BYTES)).unwrap_or(data);
    let Ok(source) = std::str::from_utf8(input) else {
        return;
    };
    if let Ok(table) = GpuSpecTable::from_toml(source) {
        assert_eq!(table.version(), 1);
    }
});
