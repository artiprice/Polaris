//! Fuzz target `directives_grammar` (`polaris-kernel-directives`): `parse` must never panic, spans
//! must stay in bounds on char boundaries, and every parsed directive must survive
//! `render` → `parse` unchanged (the D11 persistence contract).
#![no_main]

use libfuzzer_sys::fuzz_target;
use polaris_kernel_directives::render::render;
use polaris_kernel_directives::{ParseLimits, parse};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let limits = ParseLimits::default();
    let Ok(parsed) = parse(&text, &limits) else { return };
    for span in parsed.directives.iter().map(|d| &d.span).chain(parsed.mentions.iter().map(|m| &m.span)) {
        assert!(span.start <= span.end && span.end <= text.len());
        assert!(text.is_char_boundary(span.start) && text.is_char_boundary(span.end));
    }
    for directive in &parsed.directives {
        let rendered = render(&directive.value);
        let again = parse(&rendered, &limits);
        let roundtrip = again.as_ref().ok().and_then(|p| p.directives.first()).map(|d| &d.value);
        assert_eq!(roundtrip, Some(&directive.value), "rendered {rendered:?} from {text:?}");
    }
});
