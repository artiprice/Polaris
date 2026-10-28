//! Deterministic compatibility guard for the pinned tonic generator's three status fallbacks.

use std::io;

const STATUS_CAST: &str = "(tonic::Code::Unimplemented as i32)";
const STATUS_CONVERSION: &str = "i32::from(tonic::Code::Unimplemented)";

/// Replace the three known generated enum casts with tonic's equivalent `From<Code>` API.
///
/// The descriptor and wire schemas stay unchanged. A different generator shape requires an
/// explicit review: neither unexpected occurrence counts nor new primitive casts are accepted.
/// # Errors
/// Invalid generated-source shape. This fails the build rather than weakening a source lint.
pub(crate) fn rewrite_generated_status_casts(source: &str) -> io::Result<String> {
    if source.matches(STATUS_CAST).count() != 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected tonic status fallback generation",
        ));
    }
    let checked = source.replace(STATUS_CAST, STATUS_CONVERSION);
    if contains_primitive_cast(&checked) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected primitive cast in generated protocol",
        ));
    }
    Ok(checked)
}

fn contains_primitive_cast(source: &str) -> bool {
    const PRIMITIVES: [&str; 15] = [
        "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
        "f32", "f64", "_",
    ];
    source
        .split_whitespace()
        .zip(source.split_whitespace().skip(1))
        .any(|(token, target)| {
            token == "as"
                && PRIMITIVES.iter().any(|primitive| {
                    target.strip_prefix(primitive).is_some_and(|rest| {
                        rest.chars().next().is_none_or(|character| {
                            !character.is_ascii_alphanumeric() && character != '_'
                        })
                    })
                })
        })
}
