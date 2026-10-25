//! Bounded responsive geometry shared by live rendering and deterministic dumps.

use ratatui::layout::Rect;

/// Maximum supported live and dump width.
pub const MAX_WIDTH: u16 = 400;
/// Maximum supported live and dump height.
pub const MAX_HEIGHT: u16 = 200;
/// Maximum supported cells in one render.
pub const MAX_CELLS: u32 = 80_000;
/// Minimum useful live terminal width.
pub const MIN_WIDTH: u16 = 60;
/// Minimum useful live terminal height.
pub const MIN_HEIGHT: u16 = 20;
/// Responsive side-panel width, including its divider.
pub const PANEL_WIDTH: u16 = 34;

/// How an optional panel is represented at the current terminal size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPlacement {
    /// Hidden while the panel state remains available to the caller.
    Hidden,
    /// A fixed-width rail beside the conversation body.
    SideRail,
    /// An opaque sheet over the conversation body.
    BottomSheet,
}

/// The pure geometry for one screen render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenLayout {
    /// Validated/clamped viewport used for painting.
    pub viewport: Rect,
    /// Header row(s).
    pub header: Rect,
    /// Main transcript and welcome region.
    pub body: Rect,
    /// Optional side rail or sheet region.
    pub panel: Option<Rect>,
    /// Panel presentation mode.
    pub panel_placement: PanelPlacement,
    /// One-line status/goal region.
    pub status: Rect,
    /// Main composer region; always singular and full-width.
    pub composer: Rect,
    /// Footer region.
    pub footer: Rect,
    /// Whether live geometry needs the compact resize notice.
    pub below_minimum: bool,
    /// Whether the original terminal exceeded the supported viewport.
    pub oversized: bool,
}

/// Geometry request for an opened/closed optional panel and current draft height.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutRequest {
    /// Whether the user has opened the optional panel.
    pub panel_open: bool,
    /// Visible draft line count used to grow the composer, bounded by eight rows.
    pub draft_lines: u16,
}

/// Return whether dimensions fit the supported viewport and cell limits.
#[must_use]
pub fn dimensions_supported(width: u16, height: u16) -> bool {
    width > 0
        && height > 0
        && width <= MAX_WIDTH
        && height <= MAX_HEIGHT
        && u32::from(width).saturating_mul(u32::from(height)) <= MAX_CELLS
}

/// Compute clipped display geometry without allocating a new cell buffer.
#[must_use]
pub fn screen_layout(area: Rect, request: LayoutRequest) -> ScreenLayout {
    let width = area.width.min(MAX_WIDTH);
    let height = area.height.min(MAX_HEIGHT);
    let viewport = Rect::new(area.x, area.y, width, height);
    let oversized = area.width > MAX_WIDTH
        || area.height > MAX_HEIGHT
        || u32::from(area.width).saturating_mul(u32::from(area.height)) > MAX_CELLS;
    let below_minimum =
        area.width < MIN_WIDTH || area.height < MIN_HEIGHT || width == 0 || height == 0;
    if below_minimum || oversized {
        let empty = Rect::new(viewport.x, viewport.y, 0, 0);
        return ScreenLayout {
            viewport,
            header: empty,
            body: viewport,
            panel: None,
            panel_placement: PanelPlacement::Hidden,
            status: empty,
            composer: empty,
            footer: empty,
            below_minimum,
            oversized,
        };
    }

    let margin_x = 2_u16;
    let margin_y = 2_u16;
    let inner = Rect::new(
        viewport.x.saturating_add(margin_x),
        viewport.y.saturating_add(margin_y),
        width.saturating_sub(margin_x.saturating_mul(2)),
        height.saturating_sub(margin_y.saturating_mul(2)),
    );
    let header_height = 2_u16.min(inner.height);
    let footer_height = 1_u16.min(inner.height.saturating_sub(header_height));
    let usable = inner
        .height
        .saturating_sub(header_height)
        .saturating_sub(footer_height);
    let status_height = 1_u16.min(usable);
    let composer_ceiling = 8_u16.min(usable.saturating_sub(status_height));
    let requested_composer = request.draft_lines.saturating_add(2).clamp(3, 8);
    let composer_height = requested_composer.min(composer_ceiling);
    let body_height = usable
        .saturating_sub(status_height)
        .saturating_sub(composer_height)
        .saturating_sub(1);
    let header = Rect::new(inner.x, inner.y, inner.width, header_height);
    let body_y = header.y.saturating_add(header.height);
    let body_width = if request.panel_open && width >= 120 && height >= 28 {
        inner.width.saturating_sub(PANEL_WIDTH)
    } else {
        inner.width
    };
    let body = Rect::new(inner.x, body_y, body_width, body_height);
    let panel_placement = if request.panel_open {
        if width >= 120 && height >= 28 {
            PanelPlacement::SideRail
        } else {
            PanelPlacement::BottomSheet
        }
    } else {
        PanelPlacement::Hidden
    };
    let panel = match panel_placement {
        PanelPlacement::Hidden => None,
        PanelPlacement::SideRail => Some(Rect::new(
            inner.x.saturating_add(body_width),
            body_y,
            PANEL_WIDTH,
            body_height,
        )),
        PanelPlacement::BottomSheet => Some(body),
    };
    let status_y = body_y.saturating_add(body_height);
    let status = Rect::new(inner.x, status_y, inner.width, status_height);
    let composer_y = status_y.saturating_add(status_height).saturating_add(1);
    let composer = Rect::new(inner.x, composer_y, inner.width, composer_height);
    let footer_y = composer_y.saturating_add(composer_height).saturating_add(1);
    let footer = Rect::new(inner.x, footer_y, inner.width, footer_height);
    ScreenLayout {
        viewport,
        header,
        body,
        panel,
        panel_placement,
        status,
        composer,
        footer,
        below_minimum,
        oversized,
    }
}
