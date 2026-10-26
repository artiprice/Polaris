//! Safe terminal text preparation and Unicode-cell wrapping.

use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Hard per-entry input bound for one screen pass.
pub const MAX_ENTRY_SOURCE_BYTES: usize = 65_536;
/// Hard wrapped-row bound for one screen pass.
pub const MAX_ENTRY_ROWS: usize = 2_048;

/// A terminal-safe line and the byte interval it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayLine {
    /// Sanitized visible text.
    pub text: String,
    /// Approximate source byte range; removed bidi controls may create gaps.
    pub source_bytes: Range<usize>,
    /// Per-grapheme source ranges and terminal-cell intervals for mouse selection.
    pub cells: Vec<DisplayCell>,
}

/// Cell interval for one visible source grapheme.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayCell {
    /// Relative terminal cells occupied by this grapheme.
    pub cells: Range<usize>,
    /// Source byte interval that produced the grapheme.
    pub source_bytes: Range<usize>,
}

/// Convert a source string to bounded, inert text lines and wrap on terminal cells.
#[must_use]
pub fn wrap_display_text(input: &str, width: u16, source_base: usize) -> Vec<DisplayLine> {
    wrap_display_text_window(
        input,
        width,
        source_base,
        0,
        MAX_ENTRY_ROWS,
        MAX_ENTRY_SOURCE_BYTES,
    )
}

/// Wrap only a bounded row window while scanning a bounded source prefix.
///
/// The renderer uses this to keep the active composer cursor visible without
/// retaining every wrapped row of a long draft.
pub(crate) fn wrap_display_text_window(
    input: &str,
    width: u16,
    source_base: usize,
    skip_rows: usize,
    max_rows: usize,
    source_limit: usize,
) -> Vec<DisplayLine> {
    if width == 0 || max_rows == 0 {
        return Vec::new();
    }
    let limited = bounded_prefix(input, source_limit);
    let mut result = Vec::new();
    let mut source_offset = source_base;
    let mut display_row = 0_usize;
    'source: for raw_line in limited.split('\n') {
        let mut text = String::new();
        let mut line_start = source_offset;
        let mut line_end = source_offset;
        let mut used_cells = 0_usize;
        let mut cells = Vec::new();
        for (relative, grapheme) in raw_line.grapheme_indices(true) {
            let grapheme_end = relative.saturating_add(grapheme.len());
            let source_start = source_offset.saturating_add(relative);
            let source_end = source_offset.saturating_add(grapheme_end);
            let visible = safe_grapheme(grapheme);
            if visible.is_empty() {
                line_end = source_end;
                continue;
            }
            let glyph_width = UnicodeWidthStr::width(visible.as_str());
            let max_cells = usize::from(width);
            if !text.is_empty() && used_cells.saturating_add(glyph_width) > max_cells {
                let wrapped_text = core::mem::take(&mut text);
                let row_cells = core::mem::take(&mut cells);
                if display_row >= skip_rows {
                    result.push(DisplayLine {
                        text: wrapped_text,
                        source_bytes: line_start..line_end,
                        cells: row_cells,
                    });
                    if result.len() >= max_rows {
                        break 'source;
                    }
                }
                display_row = display_row.saturating_add(1);
                line_start = source_start;
                used_cells = 0;
            }
            let cell_start = used_cells;
            if glyph_width > max_cells {
                if text.is_empty() {
                    text.push('�');
                    used_cells = 1;
                }
            } else {
                text.push_str(&visible);
                used_cells = used_cells.saturating_add(glyph_width);
            }
            line_end = source_end;
            if used_cells > cell_start {
                cells.push(DisplayCell {
                    cells: cell_start..used_cells,
                    source_bytes: source_start..source_end,
                });
            }
        }
        if display_row >= skip_rows {
            result.push(DisplayLine {
                text,
                source_bytes: line_start..line_end.max(line_start),
                cells,
            });
            if result.len() >= max_rows {
                break;
            }
        }
        display_row = display_row.saturating_add(1);
        source_offset = source_offset
            .saturating_add(raw_line.len())
            .saturating_add(1);
    }
    result
}

/// Replace terminal controls and bidi formatting marks; ordinary Unicode stays intact.
#[must_use]
pub fn safe_display_text(input: &str) -> String {
    input.chars().filter_map(safe_char).collect()
}

fn bounded_prefix(input: &str, limit: usize) -> &str {
    if input.len() <= limit {
        return input;
    }
    let end = input
        .char_indices()
        .take_while(|(start, ch)| start.saturating_add(ch.len_utf8()) <= limit)
        .last()
        .map_or(0, |(start, ch)| start.saturating_add(ch.len_utf8()));
    match input.get(..end) {
        Some(prefix) => prefix,
        None => "",
    }
}

fn safe_grapheme(grapheme: &str) -> String {
    grapheme.chars().filter_map(safe_char).collect()
}

fn safe_char(ch: char) -> Option<char> {
    if is_bidi_control(ch) {
        None
    } else if ch == '\n' {
        None
    } else if ch == '\t' {
        Some(' ')
    } else if ch.is_control() {
        Some('�')
    } else {
        Some(ch)
    }
}

fn is_bidi_control(ch: char) -> bool {
    matches!(
        u32::from(ch),
        0x061c | 0x200e | 0x200f | 0x202a..=0x202e | 0x2066..=0x2069
    )
}

/// Return the first safe wrapped row, bounding work and output by terminal cells.
#[must_use]
pub fn fit_display_text(input: &str, width: u16) -> String {
    match wrap_display_text(input, width, 0).into_iter().next() {
        Some(line) => line.text,
        None => String::new(),
    }
}
