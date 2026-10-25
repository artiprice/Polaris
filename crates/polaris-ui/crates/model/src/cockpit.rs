//! Pure state for optional cockpit side panels.

/// Optional side panel shown alongside the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Panel {
    /// Agent activity and presence.
    #[default]
    Agents,
    /// Open or recent sessions.
    Sessions,
    /// Running and completed jobs.
    Jobs,
    /// Outstanding questions for the user.
    Questions,
}
/// Which part of the cockpit currently receives navigation input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CockpitFocus {
    /// The primary conversation composer and transcript.
    #[default]
    Main,
    /// The selected side panel.
    Panel,
}
/// Layout mode derived from terminal dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewportClass {
    /// Enough width for a persistent conversation and side panel.
    #[default]
    Wide,
    /// Compact dimensions where panels become temporary sheets.
    Narrow,
    /// Dimensions too small to render the normal interaction safely.
    TooSmall,
}
/// Concrete rendering behavior selected for the current panel and viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelPresentation {
    /// No panel is currently displayed.
    Closed,
    /// Panel occupies a side region beside the conversation.
    Sidebar,
    /// Panel temporarily covers the narrow viewport.
    Sheet,
    /// Panel remains open but is represented by a compact indicator.
    Indicator,
    /// The terminal should show its resize notice.
    ResizeNotice,
}

/// Pure state for panel selection, focus, pinning, and responsive presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CockpitState {
    open: bool,
    pinned: bool,
    focus: CockpitFocus,
    selected: Panel,
    viewport: ViewportClass,
}
impl Default for CockpitState {
    fn default() -> Self {
        Self {
            open: false,
            pinned: false,
            focus: CockpitFocus::Main,
            selected: Panel::Agents,
            viewport: ViewportClass::Wide,
        }
    }
}
impl CockpitState {
    #[must_use]
    /// Whether the side panel is logically open.
    pub const fn is_open(&self) -> bool {
        self.open
    }
    #[must_use]
    /// Whether the selected panel should remain represented on narrow screens.
    pub const fn is_pinned(&self) -> bool {
        self.pinned
    }
    #[must_use]
    /// Current keyboard focus destination.
    pub const fn focus(&self) -> CockpitFocus {
        self.focus
    }
    #[must_use]
    /// Currently selected panel.
    pub const fn selected(&self) -> Panel {
        self.selected
    }
    #[must_use]
    /// Current responsive viewport class.
    pub const fn viewport(&self) -> ViewportClass {
        self.viewport
    }
    #[must_use]
    /// The conversation remains the target for message submission in every panel state.
    pub const fn submission_target_is_main(&self) -> bool {
        true
    }
    #[must_use]
    /// Rendering mode derived from the current open, focus, pin, and viewport state.
    pub const fn presentation(&self) -> PanelPresentation {
        if matches!(self.viewport, ViewportClass::TooSmall) {
            return PanelPresentation::ResizeNotice;
        }
        if !self.open {
            return PanelPresentation::Closed;
        }
        match (self.viewport, self.focus) {
            (ViewportClass::Wide, _) => PanelPresentation::Sidebar,
            (ViewportClass::Narrow, CockpitFocus::Panel) => PanelPresentation::Sheet,
            (ViewportClass::Narrow, CockpitFocus::Main) if self.pinned => {
                PanelPresentation::Indicator
            }
            (ViewportClass::Narrow, CockpitFocus::Main) => PanelPresentation::Closed,
            (ViewportClass::TooSmall, _) => PanelPresentation::ResizeNotice,
        }
    }
    /// Select a panel without changing whether it is open or where focus is held.
    pub fn select(&mut self, panel: Panel) {
        self.selected = panel;
    }
    /// Toggle panel visibility and update the viewport used for presentation.
    pub fn toggle_panel(&mut self, viewport: ViewportClass) {
        self.viewport = viewport;
        self.open = !self.open;
        if !self.open {
            self.focus = CockpitFocus::Main;
        }
    }
    /// Move focus between the conversation and an open panel.
    pub fn cycle_focus(&mut self) {
        self.focus = match (self.focus, self.open) {
            (CockpitFocus::Main, true) => CockpitFocus::Panel,
            _ => CockpitFocus::Main,
        };
    }
    /// Pin or unpin the panel indicator used on narrow screens.
    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }
    /// Update responsive behavior after a terminal resize.
    pub fn set_viewport(&mut self, viewport: ViewportClass) {
        self.viewport = viewport;
        if viewport == ViewportClass::Narrow && self.focus == CockpitFocus::Main && !self.pinned {
            self.open = false;
        }
        if matches!(viewport, ViewportClass::TooSmall) {
            self.focus = CockpitFocus::Main;
        }
    }
}
