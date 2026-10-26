//! Pure chip-aware cell geometry for the editable composer.

use core::ops::Range;

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// A terminal cell coordinate in the wrapped draft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellPosition {
    /// Zero-based wrapped row.
    pub row: usize,
    /// Zero-based terminal column.
    pub column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Boundary {
    byte: usize,
    column: usize,
}

/// Source interval and display-cell interval occupied by one safe rendered atom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplaySpan {
    /// Composer bytes represented by this span.
    pub source: Range<usize>,
    /// Cell columns occupied on this row.
    pub cells: Range<usize>,
}

/// One wrapped row and its legal cursor stops.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisualRow {
    /// First source byte displayed on this row.
    pub start_byte: usize,
    /// Byte offset immediately after the last source grapheme on this row.
    pub end_byte: usize,
    /// Number of terminal cells occupied on this row.
    pub cells: usize,
    /// Sanitized text to paint for this row.
    pub display_text: String,
    /// Source-to-cell spans used for selection styling and hit mapping.
    pub spans: Vec<DisplaySpan>,
    boundaries: Vec<Boundary>,
}

/// Precomputed wrapped cell geometry shared by rendering and editor input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorGeometry {
    rows: Vec<VisualRow>,
    width: usize,
}

impl VisualRow {
    fn new(start_byte: usize) -> Self {
        Self {
            start_byte,
            end_byte: start_byte,
            cells: 0,
            display_text: String::new(),
            spans: Vec::new(),
            boundaries: Vec::new(),
        }
    }
}

impl EditorGeometry {
    /// Build wrapped geometry from display text and byte ranges for atomic chips.
    #[must_use]
    pub fn new(text: &str, chip_ranges: &[Range<usize>], width: u16) -> Self {
        let width = usize::from(width.max(1));
        let mut geometry = Self {
            rows: vec![VisualRow::new(0)],
            width,
        };
        geometry.set_boundary(0, 0, 0);
        let mut row = 0usize;
        let mut column = 0usize;

        for (start, grapheme) in text.grapheme_indices(true) {
            let end = start.saturating_add(grapheme.len());
            let chip = chip_ranges
                .iter()
                .find(|range| range.start <= start && end <= range.end);
            let chip_start = chip.is_some_and(|range| range.start == start);
            let chip_end = chip.is_some_and(|range| range.end == end);
            let newline = grapheme == "\n";
            let tab = grapheme == "\t";
            let visible = crate::text::safe_display_text(grapheme);
            if tab {
                let mut spaces = 4usize.saturating_sub(column % 4);
                if column > 0 && column.saturating_add(spaces) > width {
                    geometry.finish_row(row, start, column);
                    row = row.saturating_add(1);
                    column = 0;
                    geometry.rows.push(VisualRow::new(start));
                    spaces = 4;
                }
                geometry.set_boundary(row, start, column);
                while spaces > 0 {
                    let available = width.saturating_sub(column);
                    let consumed = spaces.min(available);
                    let span_start = column;
                    column = column.saturating_add(consumed);
                    spaces = spaces.saturating_sub(consumed);
                    geometry.push_display(
                        row,
                        start..end,
                        span_start..column,
                        " ".repeat(consumed),
                    );
                    geometry.finish_row(row, end, column);
                    if spaces > 0 {
                        row = row.saturating_add(1);
                        column = 0;
                        geometry.rows.push(VisualRow::new(start));
                    }
                }
                if column == width {
                    row = row.saturating_add(1);
                    column = 0;
                    geometry.rows.push(VisualRow::new(end));
                }
                geometry.set_boundary(row, end, column);
                continue;
            }
            let measured = if newline {
                0
            } else {
                UnicodeWidthStr::width(visible.as_str())
            };
            let cell_width = if measured > width { 1 } else { measured };

            if !newline
                && !visible.is_empty()
                && column > 0
                && column.saturating_add(cell_width) > width
            {
                geometry.finish_row(row, start, column);
                row = row.saturating_add(1);
                column = 0;
                geometry.rows.push(VisualRow::new(start));
            }

            if !chip.is_some() || chip_start {
                geometry.set_boundary(row, start, column);
            }
            if newline {
                geometry.finish_row(row, end, column);
                row = row.saturating_add(1);
                column = 0;
                geometry.rows.push(VisualRow::new(end));
                geometry.set_boundary(row, end, 0);
            } else {
                let span_start = column;
                let display = if measured > width {
                    "�".to_owned()
                } else {
                    visible
                };
                geometry.push_display(
                    row,
                    start..end,
                    span_start..span_start.saturating_add(cell_width),
                    display,
                );
                column = column.saturating_add(cell_width).min(width);
                geometry.finish_row(row, end, column);
                if !chip.is_some() || chip_end {
                    if column == width {
                        row = row.saturating_add(1);
                        column = 0;
                        geometry.rows.push(VisualRow::new(end));
                    }
                    geometry.set_boundary(row, end, column);
                }
            }
        }
        geometry
    }

    fn finish_row(&mut self, row: usize, end_byte: usize, cells: usize) {
        if let Some(target) = self.rows.get_mut(row) {
            target.end_byte = end_byte;
            target.cells = cells;
        }
    }

    fn push_display(
        &mut self,
        row: usize,
        source: Range<usize>,
        cells: Range<usize>,
        text: String,
    ) {
        if let Some(target) = self.rows.get_mut(row) {
            target.display_text.push_str(&text);
            target.spans.push(DisplaySpan { source, cells });
        }
    }

    fn set_boundary(&mut self, row: usize, byte: usize, column: usize) {
        if let Some(target) = self.rows.get_mut(row) {
            target.boundaries.retain(|boundary| boundary.byte != byte);
            target.boundaries.push(Boundary { byte, column });
        }
    }

    /// Wrapped rows, including an empty final row after a trailing line break.
    #[must_use]
    pub fn rows(&self) -> &[VisualRow] {
        &self.rows
    }

    /// Cell width used to build this geometry.
    #[must_use]
    pub const fn width(&self) -> usize {
        self.width
    }

    /// Map a legal source byte boundary to its wrapped cell position.
    #[must_use]
    pub fn caret_position(&self, cursor: usize) -> CellPosition {
        for (row_index, row) in self.rows.iter().enumerate() {
            if let Some(boundary) = row
                .boundaries
                .iter()
                .find(|boundary| boundary.byte == cursor)
            {
                return CellPosition {
                    row: row_index,
                    column: boundary.column,
                };
            }
        }
        let mut nearest = CellPosition { row: 0, column: 0 };
        for (row_index, row) in self.rows.iter().enumerate() {
            for boundary in &row.boundaries {
                if boundary.byte <= cursor {
                    nearest = CellPosition {
                        row: row_index,
                        column: boundary.column,
                    };
                }
            }
        }
        nearest
    }

    /// Map a wrapped row and cell to the nearest legal cursor boundary.
    #[must_use]
    pub fn cursor_at_cell(&self, row: usize, column: usize) -> usize {
        let row = row.min(self.rows.len().saturating_sub(1));
        let Some(target) = self.rows.get(row) else {
            return 0;
        };
        let mut best = target.boundaries.first().copied();
        for boundary in &target.boundaries {
            if best.is_none_or(|current| {
                boundary.column.abs_diff(column) < current.column.abs_diff(column)
            }) {
                best = Some(*boundary);
            }
        }
        if best.is_none() {
            let mut nearest_distance = usize::MAX;
            for (candidate_row, visual_row) in self.rows.iter().enumerate() {
                for boundary in &visual_row.boundaries {
                    let distance = candidate_row
                        .abs_diff(row)
                        .saturating_add(boundary.column.abs_diff(column));
                    if distance < nearest_distance {
                        nearest_distance = distance;
                        best = Some(*boundary);
                    }
                }
            }
        }
        match best {
            Some(boundary) => boundary.byte,
            None => 0,
        }
    }

    /// Return the source bytes painted at a wrapped row and terminal cell.
    #[must_use]
    pub fn source_at_cell(&self, row: usize, column: usize) -> Option<Range<usize>> {
        self.rows
            .get(row)?
            .spans
            .iter()
            .find(|span| span.cells.contains(&column))
            .map(|span| span.source.clone())
    }

    /// Move by visual rows while retaining the requested sticky column.
    #[must_use]
    pub fn vertical_target(
        &self,
        cursor: usize,
        row_delta: isize,
        desired_column: Option<usize>,
    ) -> (usize, usize) {
        let current = self.caret_position(cursor);
        let desired = desired_column.unwrap_or(current.column);
        let last = self.rows.len().saturating_sub(1);
        let target_row = match current.row.checked_add_signed(row_delta) {
            Some(row) => row.min(last),
            None if row_delta.is_negative() => 0,
            None => last,
        };
        (self.cursor_at_cell(target_row, desired), desired)
    }

    /// Choose a bounded row range that keeps the cursor visible in the viewport.
    #[must_use]
    pub fn visible_rows(&self, viewport_height: usize, cursor_row: usize) -> Range<usize> {
        let count = viewport_height.max(1).min(self.rows.len());
        let last_start = self.rows.len().saturating_sub(count);
        let start = cursor_row
            .saturating_add(1)
            .saturating_sub(count)
            .min(last_start);
        start..start.saturating_add(count)
    }
}
