//! Compact North Star cell art for welcome, compact and one-cell layouts.

/// Semantic colour of one current-demo logo cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoRole {
    Canvas,
    Star,
    Ray,
    Tip,
}

/// A statically-sized cell-art logo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoGrid {
    rows: &'static [&'static str],
}

pub const WELCOME: LogoGrid = LogoGrid {
    rows: &[
        "    ╷    ",
        "    │    ",
        "╶───✦───╴",
        "    │    ",
        "    ╵    ",
    ],
};
pub const COMPACT: LogoGrid = LogoGrid {
    rows: &["  ╷  ", "╶─✦─╴", "  ╵  "],
};
pub const EMBLEM: LogoGrid = LogoGrid { rows: &["✦"] };

#[must_use]
pub const fn ascii_glyph(glyph: char) -> char {
    match glyph {
        '│' | '╷' | '╵' => '|',
        '─' | '╶' | '╴' => '-',
        '✦' => '*',
        c if c.is_ascii() => c,
        _ => ' ',
    }
}
impl LogoGrid {
    #[must_use]
    pub const fn rows(&self) -> &'static [&'static str] {
        self.rows
    }
    #[must_use]
    pub const fn height(&self) -> usize {
        self.rows.len()
    }
    #[must_use]
    pub fn width(&self) -> usize {
        self.rows.first().map_or(0, |row| row.chars().count())
    }
    #[must_use]
    pub fn glyph(&self, x: usize, y: usize) -> Option<char> {
        self.rows.get(y)?.chars().nth(x)
    }
    #[must_use]
    pub fn ascii_rows(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.chars().map(ascii_glyph).collect())
            .collect()
    }
    #[must_use]
    pub fn role_at(&self, x: usize, y: usize) -> Option<LogoRole> {
        let glyph = self.glyph(x, y)?;
        Some(match glyph {
            '✦' => LogoRole::Star,
            '╷' | '╵' | '╶' | '╴' => LogoRole::Tip,
            '│' | '─' => LogoRole::Ray,
            _ => LogoRole::Canvas,
        })
    }
}
