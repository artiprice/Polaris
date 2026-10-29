// Unit tests of crates/polaris-search/crates/providers/src/fetch.rs (`tests`); compiled into that module via #[path].
use super::{MAX_DOM_DEPTH, MAX_DOM_NODES, convert_html, decode_text_bounded};
use crate::SearchProviderError;
use std::error::Error;

#[test]
fn decodes_declared_legacy_charset_and_falls_back_for_unknown_label() -> Result<(), Box<dyn Error>>
{
    let legacy = [b'c', b'a', b'f', 0xe9];
    if decode_text_bounded(&legacy, Some("windows-1252"), 8)? != "café" {
        return Err("fetcher did not decode a supported declared charset".into());
    }
    if decode_text_bounded(&[0xff], Some("unknown-charset-label"), 8)?.is_empty() {
        return Err("unknown charset did not produce bounded replacement text".into());
    }
    Ok(())
}

#[test]
fn rejects_decoded_expansion_above_configured_byte_limit() -> Result<(), Box<dyn Error>> {
    if !matches!(
        decode_text_bounded(&[0x80], Some("windows-1252"), 1),
        Err(SearchProviderError::TooLarge)
    ) {
        return Err("legacy charset expansion exceeded configured decoded bound".into());
    }
    Ok(())
}

#[test]
fn rejects_markdown_output_above_configured_byte_limit() -> Result<(), Box<dyn Error>> {
    if !matches!(
        convert_html("<p>*</p>", 1),
        Err(SearchProviderError::TooLarge)
    ) {
        return Err("Markdown output exceeded configured output bound without rejection".into());
    }
    Ok(())
}

#[test]
fn rejects_dom_depth_and_node_count_before_markdown_walk() -> Result<(), Box<dyn Error>> {
    let nested = format!(
        "{}x{}",
        "<div>".repeat(MAX_DOM_DEPTH),
        "</div>".repeat(MAX_DOM_DEPTH)
    );
    if !matches!(
        convert_html(nested.as_str(), u64::MAX),
        Err(SearchProviderError::TooLarge)
    ) {
        return Err("deep DOM passed the recursive Markdown conversion gate".into());
    }
    let many_nodes = "<span>x</span>".repeat(MAX_DOM_NODES);
    if !matches!(
        convert_html(many_nodes.as_str(), u64::MAX),
        Err(SearchProviderError::TooLarge)
    ) {
        return Err("oversized DOM passed the Markdown node-count gate".into());
    }
    Ok(())
}
