//! # polaris-ingest
//!
//! Bounded document parsing for text, Markdown, HTML, CSV, DOCX, XLSX, PPTX, and text-layer PDF.
//!
//! | Property | Value |
//! |---|---|
//! | Layer | L2 — adapters & infrastructure |
//! | Kind | lib |
//! | Phase | P5 |
//! | Parent facade | — |
//! | Status | concrete parsers implemented; format/resource acceptance remains partial |
//!
//! The parser consumes supplied bytes without opening paths. It parses text, Markdown, HTML,
//! CSV, DOCX, XLSX, PPTX, and text-layer PDF using concrete parsers. Image OCR and PDF scan
//! OCR are not configured and return typed `Unsupported` errors. Rich local
//! [`IngestedDocument`] output records available provenance; the SDK `Document` adapter drops
//! locations because that SDK type has no location fields. Archive, XML, HTML DOM, page,
//! output, and elapsed-time limits are applied, but synchronous PDF/OOXML libraries can allocate
//! internally before post-parse checks. The elapsed-time limit is checked after parsing and
//! cannot preempt synchronous work. Worker process memory/time isolation is still required;
//! this crate does not provide a total RSS or CPU-time bound.
//! CSV decoding accepts UTF-8 (with BOM removal) and deterministically falls back to Windows-1252
//! when UTF-8 decoding reports errors; it does not claim to detect arbitrary legacy encodings.

#![forbid(unsafe_code)]

use std::time::{Duration, Instant};

use polaris_sdk::registry::{DocBlock, DocParser, DocSource, Document, IngestError};

mod archive;
mod parsers;

/// Resource limits applied by [`DefaultDocParser`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IngestLimits {
    /// Maximum input size in bytes.
    pub max_bytes: u64,
    /// Maximum total decompressed OOXML member bytes.
    pub max_uncompressed_bytes: u64,
    /// Maximum number of archive members.
    pub max_archive_entries: usize,
    /// Maximum uncompressed bytes for one member.
    pub max_entry_bytes: u64,
    /// Maximum compression ratio, comparing declared uncompressed and compressed sizes.
    pub max_compression_ratio: u64,
    /// Maximum pages in PDF inputs.
    pub max_pages: usize,
    /// Maximum non-empty spreadsheet cells before workbook parsing.
    pub max_cells: u64,
    /// Maximum XML nodes in each OOXML XML part.
    pub max_xml_nodes: u64,
    /// Maximum XML nesting depth in OOXML XML parts.
    pub max_xml_depth: usize,
    /// Maximum HTML DOM nodes before conversion.
    pub max_dom_nodes: usize,
    /// Maximum HTML DOM depth before conversion.
    pub max_dom_depth: usize,
    /// Maximum normalized output blocks.
    pub max_blocks: u64,
    /// Maximum normalized UTF-8 output bytes.
    pub max_output_bytes: u64,
    /// Maximum synchronous parse duration, checked after each parser returns.
    pub max_elapsed: Duration,
}

impl Default for IngestLimits {
    fn default() -> Self {
        Self {
            max_bytes: 32 * 1024 * 1024,
            max_uncompressed_bytes: 128 * 1024 * 1024,
            max_archive_entries: 4_096,
            max_entry_bytes: 32 * 1024 * 1024,
            max_compression_ratio: 200,
            max_pages: 2_000,
            max_cells: 1_000_000,
            max_xml_nodes: 2_000_000,
            max_xml_depth: 128,
            max_dom_nodes: 10_000,
            max_dom_depth: 128,
            max_blocks: 100_000,
            max_output_bytes: 16 * 1024 * 1024,
            max_elapsed: Duration::from_secs(30),
        }
    }
}

/// SDK implementation for bounded document ingestion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DefaultDocParser {
    /// Limits used before and during parsing.
    pub limits: IngestLimits,
}

/// Detected format, based on signatures for binary files and validated text for text files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    /// Microsoft Word Open XML.
    Docx,
    /// Microsoft Excel Open XML.
    Xlsx,
    /// Comma-separated values.
    Csv,
    /// Portable Document Format.
    Pdf,
    /// Microsoft PowerPoint Open XML.
    Pptx,
    /// Hypertext Markup Language.
    Html,
    /// CommonMark Markdown.
    Markdown,
    /// Plain UTF-8 text.
    Text,
    /// Raster image requiring OCR.
    Image,
}

/// A source location available to the richer local ingest API.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceLocation {
    /// One-based page number where available.
    pub page: Option<u32>,
    /// One-based source line where available.
    pub line: Option<u32>,
    /// Workbook sheet name where available.
    pub sheet: Option<String>,
    /// Cell range or coordinate where available.
    pub cells: Option<String>,
}

/// One normalized SDK block with its source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocatedBlock {
    /// Normalized content.
    pub block: DocBlock,
    /// Original source location, when supplied by the format parser.
    pub location: SourceLocation,
}

/// Trust marker for content originating in an untrusted document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Content comes from an untrusted external document.
    Untrusted,
}

/// Rich parser output retaining provenance unavailable in the SDK document type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestedDocument {
    /// Detected format.
    pub kind: DocKind,
    /// Document title when it can be extracted without executing embedded content.
    pub title: Option<String>,
    /// Normalized blocks with source locations.
    pub blocks: Vec<LocatedBlock>,
    /// All ingested file content remains untrusted until screened by downstream policy.
    pub trust: Trust,
}

impl DefaultDocParser {
    /// Parse bytes and retain source locations in a local result.
    ///
    /// # Errors
    /// Returns a typed SDK ingest error for malformed, unsupported, or oversized input.
    pub fn ingest_bytes(
        &self,
        name: &str,
        mime: Option<&str>,
        bytes: &[u8],
    ) -> Result<IngestedDocument, IngestError> {
        let actual = u64::try_from(bytes.len())
            .map_err(|_| IngestError::Malformed("input size does not fit u64".to_owned()))?;
        if actual > self.limits.max_bytes {
            return Err(IngestError::TooLarge {
                bytes: actual,
                limit: self.limits.max_bytes,
            });
        }
        let started = Instant::now();
        let result = parsers::dispatch(name, mime, bytes, self.limits)?;
        if started.elapsed() > self.limits.max_elapsed {
            return Err(IngestError::Malformed("parse exceeded configured elapsed-time limit; synchronous parsing cannot be preempted".to_owned()));
        }
        let block_count = u64::try_from(result.blocks.len()).map_err(|_| {
            IngestError::Malformed("output block count does not fit u64".to_owned())
        })?;
        if block_count > self.limits.max_blocks {
            return Err(IngestError::TooLarge {
                bytes: block_count,
                limit: self.limits.max_blocks,
            });
        }
        let output_bytes = parsers::output_size(&result.blocks)?;
        if output_bytes > self.limits.max_output_bytes {
            return Err(IngestError::TooLarge {
                bytes: output_bytes,
                limit: self.limits.max_output_bytes,
            });
        }
        Ok(result)
    }
}

impl DocParser for DefaultDocParser {
    fn parse(&self, src: DocSource<'_>) -> Result<Document, IngestError> {
        let parsed = self.ingest_bytes(src.name, src.mime, src.bytes)?;
        Ok(Document {
            title: parsed.title,
            blocks: parsed
                .blocks
                .into_iter()
                .map(|located| located.block)
                .collect(),
        })
    }
}

/// Render SDK document blocks as normalized Markdown.
#[must_use]
pub fn to_markdown(document: &Document) -> String {
    to_markdown_blocks(document.blocks.iter())
}

/// Render borrowed SDK blocks as normalized Markdown without copying them into a document.
#[must_use]
pub fn to_markdown_blocks<'a>(blocks: impl IntoIterator<Item = &'a DocBlock>) -> String {
    let mut output = String::new();
    for block in blocks {
        match block {
            DocBlock::Heading { level, text } => {
                let count = usize::from(*level).clamp(1, 6);
                output.push_str(&"#".repeat(count));
                output.push(' ');
                output.push_str(text);
                output.push_str("\n\n");
            }
            DocBlock::Paragraph { text } => {
                output.push_str(text);
                output.push_str("\n\n");
            }
            DocBlock::Code { lang, text } => {
                let mut fence_len = 3_usize;
                let mut run_len = 0_usize;
                for character in text.chars() {
                    if character == '`' {
                        run_len = run_len.saturating_add(1);
                        fence_len = fence_len.max(run_len.saturating_add(1));
                    } else {
                        run_len = 0;
                    }
                }
                let fence = "`".repeat(fence_len);
                output.push_str(&fence);
                if let Some(language) = lang
                    .as_ref()
                    .and_then(|value| value.split_whitespace().next())
                {
                    output.push_str(language);
                }
                output.push('\n');
                output.push_str(text);
                if !text.ends_with('\n') {
                    output.push('\n');
                }
                output.push_str(&fence);
                output.push_str("\n\n");
            }
            DocBlock::Table { rows } => {
                for (row_index, row) in rows.iter().enumerate() {
                    output.push('|');
                    for cell in row {
                        output.push(' ');
                        output.push_str(&cell.replace('|', "\\|").replace('\n', " "));
                        output.push_str(" |");
                    }
                    output.push('\n');
                    if row_index == 0 {
                        output.push('|');
                        for _ in row {
                            output.push_str(" --- |");
                        }
                        output.push('\n');
                    }
                }
                output.push('\n');
            }
            DocBlock::Image { alt, .. } => {
                if let Some(text) = alt {
                    output.push_str("[Image: ");
                    output.push_str(text);
                    output.push_str("]\n\n");
                }
            }
        }
    }
    output
}

#[cfg(test)]
mod tests;
