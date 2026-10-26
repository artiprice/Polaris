/// Hard limits for the built-in deterministic input corpus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundaryLimits {
    /// Maximum number of inputs returned.
    pub max_cases: usize,
    /// Maximum bytes per input.
    pub max_case_bytes: usize,
}

/// Generates deterministic boundary byte strings without a random or model dependency.
#[derive(Debug, Clone, Copy, Default)]
pub struct BoundaryGen;

impl BoundaryGen {
    /// Return a fixed corpus, rotated by `seed`, clipped to caller limits.
    #[must_use]
    pub fn generate(seed: u64, limits: BoundaryLimits) -> Vec<Vec<u8>> {
        let candidates: [&[u8]; 22] = [
            b"0",
            b"-1",
            b"1",
            b"18446744073709551615",
            b"-9223372036854775808",
            b"9223372036854775807",
            b"0.0",
            b"-0.0",
            b"NaN",
            b"inf",
            b"-inf",
            b"",
            b"a",
            b"../",
            b"..\\",
            b"%2e%2e%2f",
            b"\0",
            b"\xff",
            b"\xe2\x80\x8b",
            b"[]",
            b"{}",
            b"null",
        ];
        if limits.max_cases == 0 || limits.max_case_bytes == 0 {
            return Vec::new();
        }
        let len = limits.max_cases.min(candidates.len().saturating_add(2));
        let len_u64 = match u64::try_from(len) {
            Ok(value) if value > 0 => value,
            Ok(_) | Err(_) => return Vec::new(),
        };
        let seed_remainder = match seed.checked_rem(len_u64) {
            Some(value) => value,
            None => return Vec::new(),
        };
        let rotation = match usize::try_from(seed_remainder) {
            Ok(value) => value,
            Err(_) => return Vec::new(),
        };
        let mut generated_inputs = Vec::with_capacity(len);
        for index in 0..len {
            let rotated = match index.checked_add(rotation) {
                Some(value) => value,
                None => return Vec::new(),
            };
            let position = match rotated.checked_rem(len) {
                Some(value) => value,
                None => return Vec::new(),
            };
            let generated = match position {
                value if value == candidates.len() => {
                    vec![b'a'; limits.max_case_bytes.min(MAX_GENERATED_STRING_BYTES)]
                }
                value if value == candidates.len().saturating_add(1) => {
                    nested_json(limits.max_case_bytes.min(MAX_GENERATED_STRING_BYTES))
                }
                value => match candidates.get(value) {
                    Some(candidate) => candidate.to_vec(),
                    None => return Vec::new(),
                },
            };
            if generated.len() <= limits.max_case_bytes {
                generated_inputs.push(generated);
            }
        }
        generated_inputs
    }
}

const MAX_GENERATED_STRING_BYTES: usize = 4096;

fn nested_json(byte_limit: usize) -> Vec<u8> {
    let depth = byte_limit.saturating_div(2).min(128);
    let mut value = Vec::with_capacity(depth.saturating_mul(2));
    value.extend(std::iter::repeat_n(b'[', depth));
    value.extend(std::iter::repeat_n(b']', depth));
    value
}
