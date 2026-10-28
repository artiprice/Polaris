// Boundary regressions use the shared palette policy. Exact tint values and
// preservation of source colors are covered independently in selection_tints.rs.
use polaris::theme::Theme;
use ratatui::{buffer::Cell, style::Color};
pub fn background(theme: Theme, cell: &Cell) -> Color {
    theme
        .copy_selection(cell.fg, cell.bg, cell.symbol().trim().is_empty())
        .bg
        .unwrap_or(cell.bg)
}
