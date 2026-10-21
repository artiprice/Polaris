//! The microchip logo as cell art: [`WELCOME`] (15x7), [`COMPACT`] (11x5) and [`EMBLEM`] (11x1).
//!
//! Every glyph is one terminal cell wide; the middle dot of the design notes is an ordinary
//! space. A renderer asks [`LogoGrid::role_at`] which colour role a cell uses and
//! [`LogoGrid::ascii_rows`] for the plain-ASCII fallback (frame glyphs become `+ - |`, the star
//! becomes `*`).
//!
//! The cell-to-role rules in [`LogoGrid::role_at`] are derived from the design description
//! ("upper/left frame light, lower/right frame and pins dark, traces green, package surface");
//! the design notes do not list the role of every single cell.

/// The rectangle covered by the chip package, inclusive on all sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackageRect {
    /// Left column.
    pub x0: usize,
    /// Right column.
    pub x1: usize,
    /// Top row.
    pub y0: usize,
    /// Bottom row.
    pub y1: usize,
}

/// The colour role of one logo cell (see [`crate::tokens`] for the hex values).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoRole {
    /// Outside the package, empty: the canvas colour.
    Canvas,
    /// Inside the package, empty: `logo-package`.
    Package,
    /// The star: `logo-star`.
    Star,
    /// An inner trace: `logo-trace`.
    Trace,
    /// The single mint highlight: `logo-mint`.
    Mint,
    /// Upper or left frame: `logo-frame-light`.
    FrameLight,
    /// Lower or right frame, and the pins outside the package: `logo-frame-dark`.
    FrameDark,
}

/// A logo as rows of cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogoGrid {
    rows: &'static [&'static str],
    mint: Option<(usize, usize)>,
    package: Option<PackageRect>,
}

/// The welcome logo, 15x7.
pub const WELCOME: LogoGrid = LogoGrid {
    rows: &[
        "    │  │  │    ",
        "  ┌─┴──┴──┴─┐  ",
        "  │    ╷    │  ",
        "──┤  ╶─✦─╴  ├──",
        "  │    ╵    │  ",
        "  └─┬──┬──┬─┘  ",
        "    │  │  │    ",
    ],
    mint: Some((7, 2)),
    package: Some(PackageRect {
        x0: 2,
        x1: 12,
        y0: 1,
        y1: 5,
    }),
};

/// The compact logo, 11x5.
pub const COMPACT: LogoGrid = LogoGrid {
    rows: &[
        "   │ │ │   ",
        " ┌─┴─┴─┴─┐ ",
        "─┤  ─✦─  ├─",
        " └─┬─┬─┬─┘ ",
        "   │ │ │   ",
    ],
    mint: Some((5, 0)),
    package: Some(PackageRect {
        x0: 1,
        x1: 9,
        y0: 1,
        y1: 3,
    }),
};

/// The emblem fallback for very short terminals, 11x1: a centred `─✦─` between four spaces.
pub const EMBLEM: LogoGrid = LogoGrid {
    rows: &["    ─✦─    "],
    mint: None,
    package: None,
};

/// The ASCII stand-in for one logo glyph: frame and traces become `+ - |`, the star `*`, anything
/// else (including a space) is kept when it is ASCII and becomes a space otherwise.
#[must_use]
pub const fn ascii_glyph(glyph: char) -> char {
    match glyph {
        '│' | '╷' | '╵' => '|',
        '─' | '╶' | '╴' => '-',
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' => '+',
        '✦' => '*',
        other if other.is_ascii() => other,
        _ => ' ',
    }
}

impl LogoGrid {
    /// The rows, each `width()` cells wide.
    #[must_use]
    pub const fn rows(&self) -> &'static [&'static str] {
        self.rows
    }

    /// Number of rows.
    #[must_use]
    pub const fn height(&self) -> usize {
        self.rows.len()
    }

    /// Number of cells per row.
    #[must_use]
    pub fn width(&self) -> usize {
        self.rows.first().map_or(0, |row| row.chars().count())
    }

    /// The glyph at column `x`, row `y`, or `None` outside the grid.
    #[must_use]
    pub fn glyph(&self, x: usize, y: usize) -> Option<char> {
        self.rows.get(y)?.chars().nth(x)
    }

    /// Where the mint highlight sits (column, row), if the logo has one.
    #[must_use]
    pub const fn mint(&self) -> Option<(usize, usize)> {
        self.mint
    }

    /// The package rectangle, if the logo has a package.
    #[must_use]
    pub const fn package(&self) -> Option<PackageRect> {
        self.package
    }

    /// The rows with every glyph replaced by [`ascii_glyph`].
    #[must_use]
    pub fn ascii_rows(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| row.chars().map(ascii_glyph).collect())
            .collect()
    }

    /// The colour role of the cell at (`x`, `y`), or `None` outside the grid.
    #[must_use]
    pub fn role_at(&self, x: usize, y: usize) -> Option<LogoRole> {
        let glyph = self.glyph(x, y)?;
        if self.mint == Some((x, y)) {
            return Some(LogoRole::Mint);
        }
        if glyph == '✦' {
            return Some(LogoRole::Star);
        }
        let Some(pkg) = self.package else {
            return Some(if glyph == ' ' {
                LogoRole::Canvas
            } else {
                LogoRole::Trace
            });
        };
        let inside = (pkg.x0..=pkg.x1).contains(&x) && (pkg.y0..=pkg.y1).contains(&y);
        Some(match (glyph == ' ', inside) {
            (true, true) => LogoRole::Package,
            (true, false) => LogoRole::Canvas,
            (false, false) => LogoRole::FrameDark,
            (false, true) => {
                let on_border = x == pkg.x0 || x == pkg.x1 || y == pkg.y0 || y == pkg.y1;
                if !on_border {
                    LogoRole::Trace
                } else if x == pkg.x0 || y == pkg.y0 {
                    LogoRole::FrameLight
                } else {
                    LogoRole::FrameDark
                }
            }
        })
    }
}
