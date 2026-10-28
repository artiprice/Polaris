use ratatui::layout::Rect;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextRegionId {
    Composer,
    Conversation,
    DraftAttachment(u64),
    Entry(usize),
    SubmittedAttachment(u64),
    AttachmentPreview(u64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewSelection {
    pub region: TextRegionId,
    pub anchor: usize,
    pub end: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HitAction {
    Composer,
    Usage(crate::usage::UsageAction),
    Stats(crate::stats::StatsAction),
    Settings(crate::settings::SettingsAction),
    Agent(crate::agents::AgentAction),
    ToLatest,
    ToggleTool(usize),
    Suggestion(usize),
    Attachment(u64),
    PreviewClose,
    PreviewCopy,
    PreviewExpand,
    PopupSelect(usize),
    PopupTab(usize),
    PopupEffort(bool),
    PopupEffortValue(usize),
    PopupField(usize),
    Approval(usize),
    FooterHelp,
    FooterModel,
    FooterMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HitTarget {
    pub rect: Rect,
    pub action: HitAction,
}

/// Maps a displayed character cell to its source UTF-8 byte boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TextCell {
    pub x: u16,
    pub y: u16,
    pub byte_offset: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextRegion {
    /// The interactive text rectangle, excluding prompts, padding and decorative rows.
    pub id: TextRegionId,
    /// Exact logical source, including real whitespace and line breaks.
    pub rect: Rect,
    pub source: std::sync::Arc<str>,
    /// Only source characters are mapped. Display-only labels have no cells.
    pub cells: Vec<TextCell>,
    /// Attachment labels expand to their actual text when copied.
    pub replacements: Vec<(
        std::ops::Range<usize>,
        std::sync::Arc<crate::composer::Attachment>,
    )>,
}

impl TextRegion {
    pub fn new(id: TextRegionId, rect: Rect, source: impl Into<String>) -> Self {
        Self {
            id,
            rect,
            source: source.into().into(),
            cells: Vec::new(),
            replacements: Vec::new(),
        }
    }

    pub fn copy_range(&self, range: std::ops::Range<usize>) -> String {
        let mut out = String::new();
        let mut cursor = range.start;
        for (span, text) in &self.replacements {
            if span.start >= range.end || span.end <= range.start {
                continue;
            }
            if cursor < span.start {
                out.push_str(
                    self.source
                        .get(cursor..span.start.min(range.end))
                        .unwrap_or(""),
                );
            }
            if let crate::composer::AttachmentPayload::Text(text) = &text.payload {
                out.push_str(text);
            }
            cursor = span.end;
        }
        if cursor < range.end {
            out.push_str(self.source.get(cursor..range.end).unwrap_or(""));
        }
        out
    }

    pub fn push_cell(&mut self, x: u16, y: u16, byte_offset: usize) {
        if byte_offset <= self.source.len() && self.source.is_char_boundary(byte_offset) {
            self.cells.push(TextCell { x, y, byte_offset });
        }
    }

    /// Resolve a drag/click cell to the nearest source boundary on that displayed row.
    /// Empty layout rows and cells outside mapped source text return None.
    pub fn position(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.rect.x || x > self.rect.right() || y < self.rect.y || y >= self.rect.bottom() {
            return None;
        }
        self.cells
            .iter()
            .filter(|cell| cell.y == y && cell.x >= self.rect.x && cell.x <= self.rect.right())
            .min_by_key(|cell| cell.x.abs_diff(x))
            .map(|cell| cell.byte_offset)
    }

    pub fn contains(&self, x: u16, y: u16) -> bool {
        self.position(x, y).is_some()
    }

    pub fn rect_for_wheel(&self) -> Rect {
        self.rect
    }
}

#[derive(Clone, Debug, Default)]
pub struct UiMap {
    pub targets: Vec<HitTarget>,
    pub text_regions: Vec<TextRegion>,
    /// Active scrollable image or attachment preview overlay.
    pub preview_rect: Option<Rect>,
    /// Full suggestion panel, including the blank margin and horizontal padding.
    pub completion_rect: Option<Rect>,
    pub agent_rect: Option<Rect>,
}

impl UiMap {
    pub fn add_target(&mut self, rect: Rect, action: HitAction) {
        if rect.width > 0 && rect.height > 0 {
            self.targets.push(HitTarget { rect, action });
        }
    }

    pub fn add_text_region(&mut self, region: TextRegion) {
        self.text_regions.push(region);
    }

    /// Remove interaction from content covered by an opaque overlay.
    pub fn occlude(&mut self, rect: Rect) {
        self.targets.retain(|target| !intersects(target.rect, rect));
        for region in &mut self.text_regions {
            region.cells.retain(|cell| {
                cell.x < rect.x
                    || cell.x >= rect.right()
                    || cell.y < rect.y
                    || cell.y >= rect.bottom()
            });
        }
    }

    /// Later targets take precedence, so controls and chips can sit above broad panes.
    pub fn hit(&self, x: u16, y: u16) -> Option<&HitAction> {
        self.targets
            .iter()
            .rev()
            .find(|target| contains(target.rect, x, y))
            .map(|target| &target.action)
    }

    pub fn text_position(&self, x: u16, y: u16) -> Option<(TextRegionId, usize)> {
        self.text_regions.iter().rev().find_map(|region| {
            region
                .position(x, y)
                .map(|offset| (region.id.clone(), offset))
        })
    }

    pub fn region(&self, id: &TextRegionId) -> Option<&TextRegion> {
        self.text_regions.iter().find(|region| &region.id == id)
    }

    pub fn wheel_region(&self, x: u16, y: u16) -> Option<TextRegionId> {
        self.text_regions
            .iter()
            .rev()
            .find(|region| region.position(x, y).is_some())
            .map(|region| region.id.clone())
    }
}

fn intersects(a: Rect, b: Rect) -> bool {
    a.x < b.right() && b.x < a.right() && a.y < b.bottom() && b.y < a.bottom()
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x && x < rect.right() && y >= rect.y && y < rect.bottom()
}
