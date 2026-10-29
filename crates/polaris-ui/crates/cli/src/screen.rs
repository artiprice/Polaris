//! Pure Ratatui CLI screen projection; no terminal or filesystem I/O.

#[path = "questions.rs"]
mod questions;

use core::ops::Range;
use polaris_ui_model::completion::{CompletionKind, Suggestion};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::questionnaire::QuestionnaireState;
use polaris_ui_model::view::{EntryKind, Mode, ProofOutcome, ToolStatus, ViewState};
use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};
use std::collections::VecDeque;
use thiserror::Error;
use unicode_width::UnicodeWidthStr;

use crate::editor::EditorGeometry;
pub use crate::layout::PanelPlacement;
use crate::layout::{ScreenLayout, dimensions_supported, screen_layout};
use crate::text::{DisplayCell, wrap_display_text};

const CANVAS: Color = Color::Rgb(0, 0, 0);
const PROMPT_SURFACE: Color = Color::Rgb(20, 20, 20);
const RAISED: Color = Color::Rgb(28, 28, 28);
const TEXT: Color = Color::Rgb(232, 232, 232);
const MUTED: Color = Color::Rgb(133, 133, 133);
const ACCENT: Color = Color::Rgb(118, 185, 0);
const COMMAND_HOVER: Color = Color::Rgb(219, 245, 176);
const GOLD: Color = Color::Rgb(249, 226, 175);
const GOLD_HOVER: Color = Color::Rgb(255, 253, 230);
const DIRECTIVE: Color = Color::Rgb(137, 180, 250);
const DIRECTIVE_HOVER: Color = Color::Rgb(180, 210, 255);
const LATEST_BG: Color = Color::Rgb(38, 38, 38);
const LATEST_HOVER: Color = Color::Rgb(55, 55, 55);
const CONTROL_HOVER: Color = Color::Rgb(24, 24, 24);
const CONTROL_FOCUS: Color = Color::Rgb(25, 25, 25);
const QUESTION_HOVER: Color = Color::Rgb(35, 56, 0);
const MINT: Color = Color::Rgb(192, 229, 140);
const ERROR: Color = Color::Rgb(172, 66, 66);
const WARNING: Color = Color::Rgb(229, 180, 84);
const RULE: Color = Color::Rgb(52, 52, 52);
const ADD_BG: Color = Color::Rgb(7, 35, 12);
const ADD_FG: Color = Color::Rgb(145, 205, 102);
const REMOVE_BG: Color = Color::Rgb(50, 7, 9);
const REMOVE_FG: Color = Color::Rgb(245, 150, 150);
const MAX_RENDERED_ENTRIES: usize = 128;
const MAX_RENDERED_ROWS: usize = 4_096;

/// Palette policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    /// Emit the North Star RGB palette.
    TrueColor,
    /// Map colors to portable named colors.
    Terminal,
    /// Map colors to the standard 16 ANSI colors.
    Ansi,
    /// Omit explicit foreground/background color.
    NoColor,
}

/// Read-only render options.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderOptions {
    /// Color capabilities requested for this output.
    pub colors: ColorMode,
    /// Use plain ASCII glyphs for ornaments.
    pub ascii: bool,
}
impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            colors: ColorMode::TrueColor,
            ascii: false,
        }
    }
}

/// Optional panel state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelVisibility {
    /// Keep the optional cockpit hidden.
    #[default]
    Closed,
    /// Show the optional cockpit when space permits.
    Open,
}

/// Borrowed canonical completion menu.
#[derive(Debug, Clone, Copy)]
pub struct CompletionOverlay<'a> {
    /// Accessible name for the suggestions list.
    pub title: &'a str,
    /// Suggestions supplied by the shared completion model.
    pub items: &'a [Suggestion],
    /// Index selected by the input owner.
    pub selected: Option<usize>,
}

/// Borrowed resolved information sheet.
#[derive(Debug, Clone, Copy)]
pub struct InfoSheet<'a> {
    /// Resolved sheet title.
    pub title: &'a str,
    /// Resolved visible content lines.
    pub lines: &'a [String],
    /// Optional caller-generated action guidance.
    pub footer: Option<&'a str>,
    /// First content line to paint.
    pub scroll: usize,
}

/// Borrowed questionnaire state and caller-owned review phase.
#[derive(Debug, Clone, Copy)]
pub struct QuestionnaireOverlay<'a> {
    /// Editable answer draft rendered by the compact question surface.
    pub state: &'a QuestionnaireState,
    /// Show the complete answer-and-notes review instead of the active question.
    pub review: bool,
    /// First wrapped review line visible in the compact review, clamped during painting.
    pub scroll: usize,
}

/// Local presentation state for a transcript tool call displayed from a real entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresentationTool {
    /// Canonical conversation entry containing `title\nbody`.
    pub entry_index: usize,
    /// Whether the source body is expanded below the title.
    pub expanded: bool,
    /// Local presentation status for the call.
    pub status: PresentationToolStatus,
    /// Optional elapsed duration displayed after completion.
    pub duration_ms: Option<u64>,
}

/// Status labels supported by the local transcript tool presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentationToolStatus {
    /// Tool is currently working.
    Working,
    /// Tool completed successfully.
    Completed,
    /// Tool reported a failure.
    Failed,
}

/// Read-only screen state from the application owner.
#[derive(Debug)]
pub struct ScreenState<'a> {
    /// Canonical shared conversation, status, goals, tools and usage snapshot.
    pub view: &'a ViewState,
    /// Canonical editable prompt buffer and cursor.
    pub composer: &'a Composer,
    /// Caller-provided monotonic time for goal presentation.
    pub now_ms: u64,
    /// Number of wrapped transcript rows to scroll back from the latest row.
    pub history_scroll: usize,
    /// Optional caller-provided workspace label.
    pub workspace: Option<&'a str>,
    /// Current transient notice, if any.
    pub notice: Option<&'a str>,
    /// Live work indicator (spinner, phase, elapsed time) shown when no notice is set.
    pub activity: Option<&'a str>,
    /// Key hints generated by the active shared keymap and capabilities.
    pub footer: Option<&'a str>,
    /// Caller supplied connection profile shown beside the model control.
    pub profile_label: Option<&'a str>,
    /// Caller supplied friendly model label; the canonical identifier is used when absent.
    pub model_label: Option<&'a str>,
    /// Canonical completion suggestions and selection, when open.
    pub completion: Option<CompletionOverlay<'a>>,
    /// Resolved opaque information sheet, when open.
    pub sheet: Option<InfoSheet<'a>>,
    /// Caller-owned visibility of the optional cockpit panel.
    pub panel: PanelVisibility,
    /// Resolved content for the caller-selected cockpit view, when available.
    pub panel_content: Option<InfoSheet<'a>>,
    /// Active compact questionnaire, when the caller has an outstanding request.
    pub questionnaire: Option<QuestionnaireOverlay<'a>>,
    /// Borrowed local presentation states keyed to canonical transcript entries.
    pub presentation_tools: &'a [PresentationTool],
    /// Current pointer target, if the caller is tracking hover.
    pub hover: Option<HitTarget>,
    /// Footer control with keyboard focus, if any.
    pub footer_focus: Option<FooterAction>,
    /// Whether the composer currently owns the terminal caret.
    pub editor_focus: bool,
    /// Optional canonical transcript source range to highlight.
    pub transcript_selection: Option<TranscriptSelection>,
    /// Optional ordered byte endpoints spanning one or more transcript entries.
    pub transcript_span_selection: Option<TranscriptSpanSelection>,
}
impl<'a> ScreenState<'a> {
    /// Create a screen projection with no transient overlays or caller key hints.
    #[must_use]
    pub const fn new(view: &'a ViewState, composer: &'a Composer) -> Self {
        Self {
            view,
            composer,
            now_ms: 0,
            history_scroll: 0,
            workspace: None,
            notice: None,
            activity: None,
            footer: None,
            profile_label: None,
            model_label: None,
            completion: None,
            sheet: None,
            panel: PanelVisibility::Closed,
            panel_content: None,
            questionnaire: None,
            presentation_tools: &[],
            hover: None,
            footer_focus: None,
            editor_focus: true,
            transcript_selection: None,
            transcript_span_selection: None,
        }
    }
    /// Set the caller-supplied monotonic time used for elapsed-goal display.
    #[must_use]
    pub const fn with_now_ms(mut self, now_ms: u64) -> Self {
        self.now_ms = now_ms;
        self
    }
    /// Set the transcript history offset in wrapped rows.
    #[must_use]
    pub const fn with_history_scroll(mut self, history_scroll: usize) -> Self {
        self.history_scroll = history_scroll;
        self
    }

    /// Set the optional workspace label shown in the header.
    #[must_use]
    pub const fn with_workspace(mut self, workspace: Option<&'a str>) -> Self {
        self.workspace = workspace;
        self
    }

    /// Set the transient notice rendered in the status row.
    #[must_use]
    pub const fn with_notice(mut self, notice: Option<&'a str>) -> Self {
        self.notice = notice;
        self
    }

    /// Set the live work indicator rendered in the status row when no notice is shown.
    #[must_use]
    pub const fn with_activity(mut self, activity: Option<&'a str>) -> Self {
        self.activity = activity;
        self
    }

    /// Set key hints generated from the active shared keymap and terminal capabilities.
    #[must_use]
    pub const fn with_footer(mut self, footer: Option<&'a str>) -> Self {
        self.footer = footer;
        self
    }
    /// Set the active profile label shown as a separate footer control.
    #[must_use]
    pub const fn with_profile_label(mut self, profile_label: Option<&'a str>) -> Self {
        self.profile_label = profile_label;
        self
    }

    /// Set the friendly model label shown in the footer without changing the canonical model id.
    #[must_use]
    pub const fn with_model_label(mut self, model_label: Option<&'a str>) -> Self {
        self.model_label = model_label;
        self
    }
    /// Set the caller-owned visibility of the optional cockpit.
    #[must_use]
    pub const fn with_panel(mut self, panel: PanelVisibility) -> Self {
        self.panel = panel;
        self
    }
    /// Set the resolved content for the currently selected cockpit view.
    #[must_use]
    pub const fn with_panel_content(mut self, panel_content: Option<InfoSheet<'a>>) -> Self {
        self.panel_content = panel_content;
        self
    }

    /// Show a compact questionnaire overlay using the supplied draft and review phase.
    #[must_use]
    pub const fn with_questionnaire(mut self, state: &'a QuestionnaireState, review: bool) -> Self {
        self.questionnaire = Some(QuestionnaireOverlay {
            state,
            review,
            scroll: 0,
        });
        self
    }

    /// Set the first wrapped review line shown by the compact review surface.
    #[must_use]
    pub const fn with_questionnaire_scroll(mut self, scroll: usize) -> Self {
        if let Some(mut questionnaire) = self.questionnaire {
            questionnaire.scroll = scroll;
            self.questionnaire = Some(questionnaire);
        }
        self
    }

    /// Attach local display state for real transcript-backed tool entries.
    #[must_use]
    pub const fn with_presentation_tools(mut self, tools: &'a [PresentationTool]) -> Self {
        self.presentation_tools = tools;
        self
    }

    /// Set canonical completion suggestions and selection.
    #[must_use]
    pub const fn with_completion(mut self, completion: Option<CompletionOverlay<'a>>) -> Self {
        self.completion = completion;
        self
    }
    /// Set the resolved opaque information sheet.
    #[must_use]
    pub const fn with_sheet(mut self, sheet: Option<InfoSheet<'a>>) -> Self {
        self.sheet = sheet;
        self
    }

    /// Set the current pointer target for hover feedback.
    #[must_use]
    pub const fn with_hover(mut self, hover: Option<HitTarget>) -> Self {
        self.hover = hover;
        self
    }

    /// Set keyboard focus for a footer control.
    #[must_use]
    pub const fn with_footer_focus(mut self, footer_focus: Option<FooterAction>) -> Self {
        self.footer_focus = footer_focus;
        self
    }

    /// Highlight a byte range in one canonical conversation entry.
    #[must_use]
    pub fn with_transcript_selection(mut self, selection: Option<TranscriptSelection>) -> Self {
        self.transcript_selection = selection;
        self
    }

    /// Highlight between two canonical transcript positions, including intermediate entries.
    #[must_use]
    pub fn with_transcript_span_selection(
        mut self,
        selection: Option<TranscriptSpanSelection>,
    ) -> Self {
        self.transcript_span_selection = selection;
        self
    }

    /// Set whether the composer currently owns the terminal caret.
    #[must_use]
    pub const fn with_editor_focus(mut self, editor_focus: bool) -> Self {
        self.editor_focus = editor_focus;
        self
    }
}

/// Semantic mouse target exposed by the pure screen hit map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    /// A current footer control.
    Footer(FooterAction),
    /// A visible completion suggestion index.
    Completion(usize),
    /// Opaque completion-menu background outside suggestion rows.
    CompletionBackdrop,
    /// The editable composer text area.
    Composer,
    /// The optional cockpit panel.
    Panel,
    /// The opaque information sheet.
    Sheet,
    /// The main transcript and welcome surface.
    Transcript,
    /// Return the transcript to its latest rows.
    ToLatest,
    /// A visible tool row in the cockpit.
    Tool(usize),
    /// A collapsed or expanded transcript tool title.
    PresentationTool(usize),
    /// A visible questionnaire choice row, including the final none option.
    QuestionChoice(usize),
    /// A question tab in the compact questionnaire overlay.
    QuestionTab(usize),
    /// A free-text answer editor in the compact questionnaire overlay.
    QuestionAnswer,
    /// The questionnaire notes editor.
    QuestionNotes,
    /// The questionnaire review/submit control.
    QuestionSubmit,
}

/// One visible cell rectangle associated with a semantic pointer target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HitRegion {
    /// Terminal-cell rectangle for this target.
    pub rect: Rect,
    /// Action or content represented by the rectangle.
    pub target: HitTarget,
}

/// A byte range selected in one canonical transcript entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptSelection {
    /// Index of the canonical conversation entry.
    pub entry_index: usize,
    /// UTF-8 source byte range to highlight.
    pub source_bytes: Range<usize>,
}

/// A grapheme-aligned position in canonical transcript content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TranscriptPoint {
    /// Index of the canonical conversation entry.
    pub entry_index: usize,
    /// UTF-8 source byte boundary within that entry.
    pub source_byte: usize,
}

/// Anchor and active endpoint for cross-entry transcript selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TranscriptSpanSelection {
    /// Fixed selection anchor.
    pub anchor: TranscriptPoint,
    /// Moving selection endpoint.
    pub head: TranscriptPoint,
}

/// Caret cell returned to the input owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaretPoint {
    /// Viewport column of the prompt cursor.
    pub x: u16,
    /// Viewport row of the prompt cursor.
    pub y: u16,
}

/// Mouse hit geometry for the editable composer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerHit {
    /// Nearest legal composer cursor boundary at the hit location.
    pub cursor: usize,
    /// Source grapheme painted at the location, absent on padding or margins.
    pub source_bytes: Option<Range<usize>>,
}

/// Visible source interval for a transcript row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSpan {
    /// Index of the canonical conversation entry.
    pub entry_index: usize,
    /// UTF-8 byte interval backing the displayed row.
    pub source_bytes: Range<usize>,
    /// Visible terminal-cell rectangle for the row.
    pub rect: Rect,
}

/// Source mapping for one visible grapheme in the transcript.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCellSpan {
    /// Index of the canonical conversation entry.
    pub entry_index: usize,
    /// UTF-8 source byte range that produced this visible grapheme.
    pub source_bytes: Range<usize>,
    /// Terminal cells occupied by the grapheme.
    pub rect: Rect,
}

/// Geometry and source maps from one draw call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderMetadata {
    /// Clipped area painted by this renderer.
    pub viewport: Rect,
    /// Main transcript area before overlays.
    pub body: Rect,
    /// The single full-width composer region.
    pub composer_rect: Rect,
    /// Visible prompt caret, absent when an opaque overlay owns focus.
    pub caret: Option<CaretPoint>,
    /// Cockpit rail or responsive sheet region.
    pub panel_rect: Option<Rect>,
    /// Anchored questionnaire area, if one is painted.
    pub questionnaire_rect: Option<Rect>,
    /// Wrapped editor geometry for the active questionnaire text field.
    pub questionnaire_editor: Option<QuestionnaireEditorGeometry>,
    /// Responsive cockpit placement for this draw.
    pub panel_placement: PanelPlacement,
    /// Number of composer instances painted.
    pub composer_count: u8,
    /// Visible source rows after opaque overlays are removed.
    pub source_spans: Vec<SourceSpan>,
    /// Per-grapheme source-to-cell mappings from the same paint pass.
    pub source_cells: Vec<SourceCellSpan>,
    /// Topmost pointer targets in the same screen geometry as the render.
    pub hits: Vec<HitRegion>,
    /// Viewport falls below the live minimum size.
    pub below_minimum: bool,
    /// Viewport exceeds supported rendering limits.
    pub oversized: bool,
}

/// Maximum accounted memory retained by one transcript render cache.
pub const MAX_RENDER_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_RENDER_CACHE_ENTRIES: usize = 128;
/// Maximum accounted exact source snapshot bytes retained for cache validation.
pub const MAX_RENDER_CACHE_SOURCE_BYTES: usize = 1024 * 1024;

/// Caller-owned LRU cache for bounded transcript wrapping results.
#[derive(Debug)]
pub struct RenderCache {
    byte_budget: usize,
    resident_bytes: usize,
    source_snapshot_bytes: usize,
    hits: u64,
    misses: u64,
    evictions: u64,
    directory_bytes: usize,
    max_entries: usize,
    entries: VecDeque<CachedEntryRows>,
}

impl RenderCache {
    /// Maximum accepted cache budget.
    pub const MAX_BYTE_BUDGET: usize = MAX_RENDER_CACHE_BYTES;

    /// Create a cache with the maximum supported retention budget.
    #[must_use]
    pub fn new() -> Self {
        Self::with_byte_budget(Self::MAX_BYTE_BUDGET)
    }

    /// Create a cache with a caller-selected budget capped at 32 MiB.
    #[must_use]
    pub fn with_byte_budget(byte_budget: usize) -> Self {
        let byte_budget = byte_budget.min(Self::MAX_BYTE_BUDGET);
        let directory_budget = byte_budget / 8;
        let requested_entries = (directory_budget / core::mem::size_of::<CachedEntryRows>())
            .min(MAX_RENDER_CACHE_ENTRIES);
        let entries = VecDeque::with_capacity(requested_entries);
        let max_entries = entries.capacity().min(MAX_RENDER_CACHE_ENTRIES);
        let directory_bytes = max_entries.saturating_mul(core::mem::size_of::<CachedEntryRows>());
        Self {
            byte_budget,
            resident_bytes: directory_bytes,
            source_snapshot_bytes: 0,
            hits: 0,
            misses: 0,
            evictions: 0,
            directory_bytes,
            max_entries,
            entries,
        }
    }

    /// Return bounded LRU accounting and hit/miss counters.
    #[must_use]
    pub fn stats(&self) -> RenderCacheStats {
        RenderCacheStats {
            entries: self.entries.len(),
            hits: self.hits,
            misses: self.misses,
            evictions: self.evictions,
            resident_bytes: self.resident_bytes,
            source_snapshot_bytes: self.source_snapshot_bytes,
            byte_budget: self.byte_budget,
        }
    }

    /// Drop retained rows and snapshots while preserving cumulative counters.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.resident_bytes = self.directory_bytes;
        self.source_snapshot_bytes = 0;
    }
}

impl Default for RenderCache {
    fn default() -> Self {
        Self::new()
    }
}

/// Observable cache accounting for tests and operational diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderCacheStats {
    /// Number of retained per-entry row sets.
    pub entries: usize,
    /// Number of exact key-and-content cache hits.
    pub hits: u64,
    /// Number of misses, including entries too large to retain.
    pub misses: u64,
    /// Number of least-recently-used entries removed for budget pressure.
    pub evictions: u64,
    /// Accounted bytes for retained snapshots, rows, and cache entries.
    pub resident_bytes: usize,
    /// Accounted allocation bytes for exact source-prefix snapshots.
    pub source_snapshot_bytes: usize,
    /// Effective byte limit for this cache.
    pub byte_budget: usize,
}

/// Shared wrapping and viewport geometry for questionnaire mouse/editor input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionnaireEditorGeometry {
    /// Notes or free-text field rectangle in terminal cells.
    pub rect: Rect,
    /// Wrapped source geometry shared with rendering and click mapping.
    pub geometry: EditorGeometry,
    /// Visible geometry rows from the wrapped editor.
    pub visible_rows: Range<usize>,
}

impl RenderMetadata {
    /// Resolve a viewport cell to the topmost target at that point.
    #[must_use]
    pub fn hit_at(&self, x: u16, y: u16) -> Option<HitTarget> {
        self.hits
            .iter()
            .rev()
            .find(|hit| hit.rect.contains(Position::new(x, y)))
            .map(|hit| hit.target)
    }

    /// Resolve a transcript cell to the source grapheme that produced it.
    #[must_use]
    pub fn source_at(&self, x: u16, y: u16) -> Option<&SourceCellSpan> {
        self.source_cells
            .iter()
            .find(|span| span.rect.contains(Position::new(x, y)))
    }

    /// Resolve a drag endpoint to a legal boundary on the nearest visible text row.
    ///
    /// Unlike [`Self::source_at`], this clamps horizontal positions before or after
    /// rendered text to that row's start or end. Vertical positions must remain in
    /// the transcript body; rows containing no visible source text are never used.
    #[must_use]
    pub fn transcript_boundary_at(&self, x: u16, y: u16) -> Option<TranscriptPoint> {
        if !self.body.contains(Position::new(x, y))
            || self.hit_at(x, y) == Some(HitTarget::ToLatest)
            || self.source_spans.is_empty()
        {
            return None;
        }
        let row = self
            .source_spans
            .iter()
            .min_by_key(|span| span.rect.y.abs_diff(y))?;
        let row_cells = self
            .source_cells
            .iter()
            .filter(|cell| cell.entry_index == row.entry_index && cell.rect.y == row.rect.y);
        let mut first_cell = None;
        let mut last_cell = None;
        for cell in row_cells {
            first_cell = Some(first_cell.map_or(cell, |current: &SourceCellSpan| {
                if cell.rect.x < current.rect.x {
                    cell
                } else {
                    current
                }
            }));
            last_cell = Some(last_cell.map_or(cell, |current: &SourceCellSpan| {
                if cell.rect.right() > current.rect.right() {
                    cell
                } else {
                    current
                }
            }));
        }
        let (Some(first), Some(last)) = (first_cell, last_cell) else {
            return None;
        };
        let start = TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: row.source_bytes.start,
        };
        let end = TranscriptPoint {
            entry_index: row.entry_index,
            source_byte: row.source_bytes.end,
        };
        if x < first.rect.x {
            return Some(start);
        }
        if x >= last.rect.right() {
            return Some(end);
        }
        let cell = self.source_at(x, row.rect.y)?;
        let midpoint = cell.rect.x.saturating_add(cell.rect.width / 2);
        Some(if x >= midpoint {
            TranscriptPoint {
                entry_index: cell.entry_index,
                source_byte: cell.source_bytes.end,
            }
        } else {
            TranscriptPoint {
                entry_index: cell.entry_index,
                source_byte: cell.source_bytes.start,
            }
        })
    }
}

/// One cell from a deterministic styled dump.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DumpCell {
    /// Cell grapheme or continuation symbol.
    pub symbol: String,
    /// Ratatui foreground color, including indexed ANSI colors.
    pub fg: Color,
    /// Ratatui background color, including indexed ANSI colors.
    pub bg: Color,
    /// Cell attributes, including bold or reverse-video selection fallbacks.
    pub modifiers: Modifier,
    /// Resolved RGB foreground when true-color output is selected.
    pub fg_rgb: Option<(u8, u8, u8)>,
    /// Resolved RGB background when true-color output is selected.
    pub bg_rgb: Option<(u8, u8, u8)>,
}

/// Styled cells and metadata from the same render pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellDump {
    /// Row-major cells from the Ratatui test backend.
    pub cells: Vec<DumpCell>,
    /// Geometry and source mapping returned by the same draw pass.
    pub metadata: RenderMetadata,
    /// Requested dump width.
    pub width: u16,
    /// Requested dump height.
    pub height: u16,
}

/// Renderer/dump failures.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("unsupported render dimensions {width}x{height}")]
    /// Dump dimensions are zero or outside the supported viewport limits.
    InvalidDimensions {
        /// Requested width in cells.
        width: u16,
        /// Requested height in cells.
        height: u16,
    },
    /// The in-memory test backend could not render the frame.
    #[error("test backend rendering failed: {0}")]
    Backend(#[from] std::io::Error),
}

/// Prepare layout and pointer targets without touching a frame or buffer.
#[must_use]
pub fn prepare_metadata(
    state: &ScreenState<'_>,
    options: &RenderOptions,
    area: Rect,
) -> RenderMetadata {
    prepare_metadata_and_rows(state, options, area, true, None).0
}

/// Prepare screen geometry while reusing caller-owned bounded transcript rows.
#[must_use]
pub fn prepare_metadata_with_cache(
    state: &ScreenState<'_>,
    options: &RenderOptions,
    area: Rect,
    cache: &mut RenderCache,
) -> RenderMetadata {
    prepare_metadata_and_rows(state, options, area, true, Some(cache)).0
}

fn prepare_metadata_and_rows(
    state: &ScreenState<'_>,
    options: &RenderOptions,
    area: Rect,
    compute_caret: bool,
    cache: Option<&mut RenderCache>,
) -> (RenderMetadata, Vec<RenderRow>, ScreenLayout) {
    let layout = layout_for_state(state, area);
    let mut metadata = from_layout(layout);
    if compute_caret && state.editor_focus && state.completion.is_none() && state.sheet.is_none() {
        metadata.caret = composer_caret_position(state.composer, layout.composer);
    }
    let rows = if layout.below_minimum || layout.oversized {
        Vec::new()
    } else {
        transcript_rows_with_cache(
            state.view,
            layout.body.width.saturating_sub(2),
            state.presentation_tools,
            cache,
        )
    };
    append_transcript_source_cells(&mut metadata.source_cells, &rows, state, layout.body);
    append_transcript_source_spans(&mut metadata.source_spans, &rows, state, layout.body);
    if state.history_scroll > 0
        && let Some(rect) = to_latest_rect(layout.body)
    {
        occlude_source_cells(&mut metadata.source_cells, rect);
    }
    if layout.below_minimum || layout.oversized {
        return (metadata, rows, layout);
    }

    if layout.body.width > 0 && layout.body.height > 0 {
        metadata.hits.push(HitRegion {
            rect: layout.body,
            target: HitTarget::Transcript,
        });
    }
    let visible_rows = transcript_visible_range(&rows, state, layout.body);
    for (row_offset, row) in rows
        .iter()
        .skip(visible_rows.start)
        .take(visible_rows.len())
        .enumerate()
    {
        if let Some(entry_index) = row.presentation_tool {
            let Ok(dy) = u16::try_from(row_offset) else {
                continue;
            };
            metadata.hits.push(HitRegion {
                rect: Rect::new(
                    layout.body.x.saturating_add(1),
                    layout.body.y.saturating_add(dy),
                    layout.body.width.saturating_sub(2),
                    1,
                ),
                target: HitTarget::PresentationTool(entry_index),
            });
        }
    }
    if state.history_scroll > 0
        && let Some(rect) = to_latest_rect(layout.body)
    {
        metadata.hits.push(HitRegion {
            rect,
            target: HitTarget::ToLatest,
        });
    }
    if let Some(panel) = layout.panel {
        let visible_panel = if layout.panel_placement == PanelPlacement::BottomSheet {
            overlay_rect(panel, panel.height.min(10))
        } else {
            panel
        };
        if visible_panel.width > 0 && visible_panel.height > 0 {
            metadata.hits.push(HitRegion {
                rect: visible_panel,
                target: HitTarget::Panel,
            });
            if layout.panel_placement == PanelPlacement::SideRail && state.panel_content.is_none() {
                append_tool_hits(&mut metadata.hits, visible_panel, state);
            }
        }
        if layout.panel_placement == PanelPlacement::BottomSheet {
            occlude_source_cells(&mut metadata.source_cells, visible_panel);
            remove_occluded_spans(&mut metadata.source_spans, visible_panel);
        }
    }

    let composer_text = Rect::new(
        layout.composer.x.saturating_add(2),
        layout.composer.y.saturating_add(1),
        layout.composer.width.saturating_sub(4),
        layout.composer.height.saturating_sub(2),
    );
    if composer_text.width > 0 && composer_text.height > 0 {
        metadata.hits.push(HitRegion {
            rect: composer_text,
            target: HitTarget::Composer,
        });
    }

    if state.sheet.is_some() {
        let rect = overlay_rect(layout.body, layout.body.height.min(16));
        if rect.width >= 4 && rect.height >= 5 {
            occlude_hits(&mut metadata.hits, rect);
            occlude_source_cells(&mut metadata.source_cells, rect);
            remove_occluded_spans(&mut metadata.source_spans, rect);
            metadata.hits.push(HitRegion {
                rect,
                target: HitTarget::Sheet,
            });
        }
    }

    if let Some(menu) = state.completion {
        let body_rect = overlay_rect(layout.body, layout.body.height.min(12));
        let rect = Rect::new(
            layout.viewport.x,
            body_rect.y,
            layout.viewport.width,
            body_rect.height,
        );
        if rect.width > 0 && rect.height >= 2 {
            // The completion backdrop is opaque across the full width. Item rows
            // are added afterward so they resolve to their own stable indexes.
            occlude_hits(&mut metadata.hits, rect);
            occlude_source_cells(&mut metadata.source_cells, rect);
            remove_occluded_spans(&mut metadata.source_spans, rect);
            metadata.hits.push(HitRegion {
                rect,
                target: HitTarget::CompletionBackdrop,
            });
            let selected = menu
                .selected
                .map(|i| i.min(menu.items.len().saturating_sub(1)));
            let visible = usize::from(rect.height.saturating_sub(1));
            let first = selected.map_or(0, |i| i.saturating_add(1).saturating_sub(visible));
            for (offset, index) in (first..menu.items.len()).take(visible).enumerate() {
                if let Ok(dy) = u16::try_from(offset) {
                    metadata.hits.push(HitRegion {
                        rect: Rect::new(
                            rect.x,
                            rect.y.saturating_add(1).saturating_add(dy),
                            rect.width,
                            1,
                        ),
                        target: HitTarget::Completion(index),
                    });
                }
            }
        }
    }

    if let Some(questionnaire) = state.questionnaire {
        if let Some(rect) = questions::overlay_rect(layout.composer, layout.viewport, questionnaire)
        {
            occlude_hits(&mut metadata.hits, rect);
            occlude_source_cells(&mut metadata.source_cells, rect);
            remove_occluded_spans(&mut metadata.source_spans, rect);
            metadata.questionnaire_rect = Some(rect);
            metadata.composer_count = 0;
            metadata.questionnaire_editor =
                questions::append_hits(&mut metadata.hits, rect, questionnaire, *options);
        }
    }

    for (rect, _label, action) in footer_controls(
        state.view,
        state.profile_label,
        state.model_label,
        Rect::new(
            layout.viewport.x.saturating_add(4),
            layout.viewport.bottom().saturating_sub(1),
            layout.viewport.width.saturating_sub(8),
            1,
        ),
    ) {
        metadata.hits.push(HitRegion {
            rect,
            target: HitTarget::Footer(action),
        });
    }
    (metadata, rows, layout)
}

fn layout_for_state(state: &ScreenState<'_>, area: Rect) -> ScreenLayout {
    screen_layout(
        area,
        crate::layout::LayoutRequest {
            panel_open: state.panel == PanelVisibility::Open,
            draft_lines: visible_draft_lines(state.composer, area.width),
            status_visible: state.notice.is_some_and(|n| !n.is_empty())
                || state.activity.is_some_and(|a| !a.is_empty())
                || state.view.goal.is_some()
                || state.view.status.proof_of_done.is_some(),
        },
    )
}

/// Return the composer rectangle using layout inputs only, without preparing transcript rows.
#[must_use]
pub fn composer_rect(state: &ScreenState<'_>, area: Rect) -> Rect {
    layout_for_state(state, area).composer
}

fn transcript_visible_range(
    rows: &[RenderRow],
    state: &ScreenState<'_>,
    area: Rect,
) -> Range<usize> {
    if area.width < 3
        || area.height == 0
        || (rows.is_empty() && state.view.goal.is_none() && state.view.tools.is_empty())
    {
        return 0..0;
    }
    let count = usize::from(area.height);
    let max_scroll = rows.len().saturating_sub(count);
    let end = rows
        .len()
        .saturating_sub(state.history_scroll.min(max_scroll));
    end.saturating_sub(count)..end
}

fn append_transcript_source_cells(
    output: &mut Vec<SourceCellSpan>,
    rows: &[RenderRow],
    state: &ScreenState<'_>,
    area: Rect,
) {
    let visible = transcript_visible_range(rows, state, area);
    for (row_offset, item) in rows
        .iter()
        .skip(visible.start)
        .take(visible.len())
        .enumerate()
    {
        let Ok(dy) = u16::try_from(row_offset) else {
            continue;
        };
        let y = area.y.saturating_add(dy);
        let x = area.x.saturating_add(1).saturating_add(item.gutter_width());
        for cell in &item.cell_sources {
            let width = cell.cells.end.saturating_sub(cell.cells.start);
            if width == 0 {
                continue;
            }
            if let (Ok(dx), Ok(width)) = (u16::try_from(cell.cells.start), u16::try_from(width)) {
                output.push(SourceCellSpan {
                    entry_index: item.entry_index,
                    source_bytes: cell.source_bytes.clone(),
                    rect: Rect::new(x.saturating_add(dx), y, width, 1),
                });
            }
        }
    }
}

fn append_transcript_source_spans(
    output: &mut Vec<SourceSpan>,
    rows: &[RenderRow],
    state: &ScreenState<'_>,
    area: Rect,
) {
    if area.width < 3
        || area.height == 0
        || (rows.is_empty() && state.view.goal.is_none() && state.view.tools.is_empty())
    {
        return;
    }
    let visible = transcript_visible_range(rows, state, area);
    for (row_offset, item) in rows
        .iter()
        .skip(visible.start)
        .take(visible.len())
        .enumerate()
    {
        if item.source_bytes.start >= item.source_bytes.end {
            continue;
        }
        let Ok(dy) = u16::try_from(row_offset) else {
            continue;
        };
        let source_x = area.x.saturating_add(1).saturating_add(item.gutter_width());
        output.push(SourceSpan {
            entry_index: item.entry_index,
            source_bytes: item.source_bytes.clone(),
            rect: Rect::new(
                source_x,
                area.y.saturating_add(dy),
                area.width
                    .saturating_sub(2)
                    .saturating_sub(item.gutter_width()),
                1,
            ),
        });
    }
}

fn occlude_source_cells(source_cells: &mut Vec<SourceCellSpan>, opaque: Rect) {
    source_cells.retain(|span| !rects_overlap(span.rect, opaque));
}

fn rects_overlap(left: Rect, right: Rect) -> bool {
    left.x < right.right()
        && right.x < left.right()
        && left.y < right.bottom()
        && right.y < left.bottom()
}

fn append_tool_hits(hits: &mut Vec<HitRegion>, panel: Rect, state: &ScreenState<'_>) {
    if state.view.tools.is_empty() || panel.height < 2 || panel.width < 4 {
        return;
    }
    let usage_rows = if state.view.hud.requests > 0 {
        3_usize
    } else {
        1
    };
    let first_tool_row = 3_usize.saturating_add(usage_rows);
    for (offset, (index, _tool)) in state
        .view
        .tools
        .iter()
        .enumerate()
        .rev()
        .take(4)
        .enumerate()
    {
        let row = first_tool_row.saturating_add(offset);
        if let Ok(dy) = u16::try_from(row) {
            let y = panel.y.saturating_add(1).saturating_add(dy);
            if y < panel.bottom() {
                hits.push(HitRegion {
                    rect: Rect::new(
                        panel.x.saturating_add(2),
                        y,
                        panel.width.saturating_sub(3),
                        1,
                    ),
                    target: HitTarget::Tool(index),
                });
            }
        }
    }
}

fn occlude_hits(hits: &mut Vec<HitRegion>, opaque: Rect) {
    let prior = core::mem::take(hits);
    for hit in prior {
        for rect in subtract_rect(hit.rect, opaque) {
            if rect.width > 0 && rect.height > 0 {
                hits.push(HitRegion {
                    rect,
                    target: hit.target,
                });
            }
        }
    }
}

fn subtract_rect(source: Rect, opaque: Rect) -> Vec<Rect> {
    let left = source.x.max(opaque.x);
    let top = source.y.max(opaque.y);
    let right = source.right().min(opaque.right());
    let bottom = source.bottom().min(opaque.bottom());
    if left >= right || top >= bottom {
        return vec![source];
    }
    let mut result = Vec::with_capacity(4);
    if top > source.y {
        result.push(Rect::new(
            source.x,
            source.y,
            source.width,
            top.saturating_sub(source.y),
        ));
    }
    if bottom < source.bottom() {
        result.push(Rect::new(
            source.x,
            bottom,
            source.width,
            source.bottom().saturating_sub(bottom),
        ));
    }
    if left > source.x {
        result.push(Rect::new(
            source.x,
            top,
            left.saturating_sub(source.x),
            bottom.saturating_sub(top),
        ));
    }
    if right < source.right() {
        result.push(Rect::new(
            right,
            top,
            source.right().saturating_sub(right),
            bottom.saturating_sub(top),
        ));
    }
    result
}

/// Render a bounded frame and return the geometry used by this paint.
pub fn render(
    frame: &mut Frame<'_>,
    state: &ScreenState<'_>,
    options: &RenderOptions,
) -> RenderMetadata {
    render_inner(frame, state, options, None)
}

/// Render a frame while reusing caller-owned bounded transcript wrapping rows.
pub fn render_with_cache(
    frame: &mut Frame<'_>,
    state: &ScreenState<'_>,
    options: &RenderOptions,
    cache: &mut RenderCache,
) -> RenderMetadata {
    render_inner(frame, state, options, Some(cache))
}

fn render_inner(
    frame: &mut Frame<'_>,
    state: &ScreenState<'_>,
    options: &RenderOptions,
    cache: Option<&mut RenderCache>,
) -> RenderMetadata {
    let (mut metadata, rows, layout) =
        prepare_metadata_and_rows(state, options, frame.area(), false, cache);
    let buffer = frame.buffer_mut();
    let canvas = match options.colors {
        ColorMode::TrueColor => CANVAS,
        ColorMode::Ansi => Color::Indexed(0),
        ColorMode::Terminal | ColorMode::NoColor => Color::Reset,
    };
    buffer.set_style(layout.viewport, Style::default().bg(canvas));
    if layout.oversized {
        draw_notice(
            buffer,
            layout.viewport,
            "Viewport limited to 400x200 cells",
            *options,
        );
        return metadata;
    }
    if layout.below_minimum {
        draw_notice(
            buffer,
            layout.viewport,
            "Resize to at least 60x20 to continue",
            *options,
        );
        return metadata;
    }
    draw_header(buffer, layout, state, *options);
    draw_status(buffer, layout.status, state, *options);
    draw_footer(
        buffer,
        layout.footer,
        state.view,
        state.profile_label,
        state.model_label,
        *options,
    );
    draw_body(buffer, layout.body, &rows, state, *options);
    if state.history_scroll > 0
        && let Some(rect) = to_latest_rect(layout.body)
    {
        buffer.set_style(rect, latest_base_style(*options));
        buffer.set_stringn(
            rect.x,
            rect.y,
            "[to latest]",
            usize::from(rect.width),
            fg(*options, ACCENT),
        );
    }
    if let Some(rect) = layout.panel {
        if layout.panel_placement == PanelPlacement::SideRail {
            draw_panel(buffer, rect, state, *options);
        } else {
            draw_panel_sheet(buffer, rect, state, *options);
        }
    }
    if let Some(sheet) = state.sheet {
        draw_info_sheet(buffer, layout.body, sheet, *options);
    }
    if let Some(menu) = state.completion {
        draw_completion(buffer, layout.body, menu, *options);
    }
    if layout.panel_placement == PanelPlacement::BottomSheet
        && let Some(panel) = layout.panel
    {
        remove_occluded_spans(
            &mut metadata.source_spans,
            overlay_rect(panel, panel.height.min(10)),
        );
    }
    if state.sheet.is_some() {
        remove_occluded_spans(
            &mut metadata.source_spans,
            overlay_rect(layout.body, layout.body.height.min(16)),
        );
    }
    if state.completion.is_some() {
        remove_occluded_spans(
            &mut metadata.source_spans,
            overlay_rect(layout.body, layout.body.height.min(12)),
        );
    }
    let caret = if let (Some(questionnaire), Some(rect)) =
        (state.questionnaire, metadata.questionnaire_rect)
    {
        questions::draw(buffer, rect, questionnaire, *options)
    } else {
        draw_composer(buffer, layout.composer, state.composer, *options)
    };
    if metadata.questionnaire_rect.is_some() {
        metadata.caret = caret;
    } else if state.editor_focus && state.completion.is_none() && state.sheet.is_none() {
        metadata.caret = caret;
    } else {
        metadata.caret = None;
    }
    if let Some(action) = state.footer_focus {
        for region in metadata
            .hits
            .iter()
            .filter(|hit| hit.target == HitTarget::Footer(action))
        {
            buffer.set_style(region.rect, control_focus_style(*options));
        }
    }
    if let Some(target) = state.hover {
        if let Some(style) = hover_style(target, state, *options) {
            for region in metadata.hits.iter().filter(|hit| hit.target == target) {
                buffer.set_style(region.rect, style);
            }
        }
    }
    if let Some(point) = metadata.caret {
        frame.set_cursor_position(Position::new(point.x, point.y));
    }
    metadata
}

/// Render a bounded styled-cell snapshot through the same Ratatui draw path.
pub fn dump_cells(
    state: &ScreenState<'_>,
    options: &RenderOptions,
    width: u16,
    height: u16,
) -> Result<CellDump, RenderError> {
    if !dimensions_supported(width, height) {
        return Err(RenderError::InvalidDimensions { width, height });
    }
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = match ratatui::Terminal::new(backend) {
        Ok(value) => value,
        Err(never) => match never {},
    };
    let mut metadata = None;
    match terminal.draw(|frame| metadata = Some(render(frame, state, options))) {
        Ok(_) => {}
        Err(never) => match never {},
    }
    let metadata = match metadata {
        Some(value) => value,
        None => {
            return Err(RenderError::Backend(std::io::Error::other(
                "render callback did not run",
            )));
        }
    };
    let cells = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|cell| DumpCell {
            symbol: cell.symbol().to_owned(),
            fg: cell.fg,
            bg: cell.bg,
            modifiers: cell.modifier,
            fg_rgb: color_rgb(cell.fg),
            bg_rgb: color_rgb(cell.bg),
        })
        .collect();
    Ok(CellDump {
        cells,
        metadata,
        width,
        height,
    })
}

/// Return plain cell text from the same Ratatui render path.
pub fn dump_text(
    state: &ScreenState<'_>,
    options: &RenderOptions,
    width: u16,
    height: u16,
) -> Result<String, RenderError> {
    let dump = dump_cells(state, options, width, height)?;
    let mut output = String::new();
    for row in 0..usize::from(dump.height) {
        if row > 0 {
            output.push('\n');
        }
        let start = row.saturating_mul(usize::from(dump.width));
        let end = start.saturating_add(usize::from(dump.width));
        for cell in dump
            .cells
            .iter()
            .skip(start)
            .take(end.saturating_sub(start))
        {
            output.push_str(&cell.symbol);
        }
    }
    Ok(output)
}

fn from_layout(layout: ScreenLayout) -> RenderMetadata {
    RenderMetadata {
        viewport: layout.viewport,
        body: layout.body,
        composer_rect: layout.composer,
        caret: None,
        panel_rect: layout.panel,
        questionnaire_rect: None,
        questionnaire_editor: None,
        panel_placement: layout.panel_placement,
        composer_count: 1,
        source_spans: Vec::new(),
        source_cells: Vec::new(),
        hits: Vec::new(),
        below_minimum: layout.below_minimum,
        oversized: layout.oversized,
    }
}

fn draw_notice(buffer: &mut Buffer, area: Rect, text: &str, options: RenderOptions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let style = fg(options, WARNING).add_modifier(Modifier::BOLD);
    buffer.set_stringn(
        area.x,
        area.y,
        crate::text::fit_display_text(text, area.width),
        usize::from(area.width),
        style,
    );
}

fn draw_header(
    buffer: &mut Buffer,
    layout: ScreenLayout,
    state: &ScreenState<'_>,
    options: RenderOptions,
) {
    let area = layout.header;
    let mark = if options.ascii { "*" } else { "✦" };
    buffer.set_stringn(area.x, area.y, mark, 1, fg(options, MINT));
    buffer.set_stringn(
        area.x.saturating_add(2),
        area.y,
        "Polaris",
        7,
        fg(options, TEXT),
    );
    let version = format!("v{}", env!("CARGO_PKG_VERSION").replace("-alpha", "alpha"));
    let version_width =
        u16::try_from(UnicodeWidthStr::width(version.as_str())).map_or(u16::MAX, |width| width);
    buffer.set_stringn(
        area.x.saturating_add(10),
        area.y,
        &version,
        usize::from(version_width),
        fg(options, MUTED),
    );
    let demo_badge = state.profile_label.filter(|label| *label == "demo");
    let demo_badge_width = if demo_badge.is_some() { 7 } else { 0 };
    if demo_badge.is_some() {
        buffer.set_stringn(
            area.x.saturating_add(10).saturating_add(version_width),
            area.y,
            " · DEMO",
            usize::from(demo_badge_width),
            fg(options, MUTED),
        );
    }
    let brand_width = 10_u16
        .saturating_add(version_width)
        .saturating_add(demo_badge_width);
    let path_width = area.width.saturating_sub(brand_width.saturating_add(4));
    if path_width >= 12 {
        if let Some(workspace) = state.workspace {
            let display = workspace.strip_prefix(r"\\?\").map_or(workspace, |p| p);
            let clipped = crate::text::fit_display_text(display, path_width);
            Paragraph::new(clipped)
                .alignment(ratatui::layout::Alignment::Right)
                .style(fg(options, MUTED))
                .render(
                    Rect::new(
                        area.right().saturating_sub(path_width),
                        area.y,
                        path_width,
                        1,
                    ),
                    buffer,
                );
        }
    }
}

fn draw_status(buffer: &mut Buffer, area: Rect, state: &ScreenState<'_>, options: RenderOptions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let (status, tone) = if let Some(notice) = state.notice.filter(|n| !n.is_empty()) {
        (crate::text::fit_display_text(notice, area.width), WARNING)
    } else if let Some(activity) = state.activity.filter(|a| !a.is_empty()) {
        (crate::text::fit_display_text(activity, area.width), ACCENT)
    } else if let Some(goal) = state.view.goal.as_ref() {
        (
            crate::text::fit_display_text(&goal.label(state.now_ms), area.width),
            MUTED,
        )
    } else if state.view.status.proof_of_done == Some(ProofOutcome::Blocked) {
        (
            "Proof of Done: BLOCKED — required evidence is missing".to_owned(),
            ERROR,
        )
    } else if state.view.status.proof_of_done == Some(ProofOutcome::Passed) {
        (
            "Proof of Done: passed (observed gate result)".to_owned(),
            MUTED,
        )
    } else {
        (String::new(), MUTED)
    };
    buffer.set_stringn(
        area.x,
        area.y,
        status,
        usize::from(area.width),
        fg(options, tone),
    );
}

fn draw_footer(
    buffer: &mut Buffer,
    area: Rect,
    view: &ViewState,
    profile_label: Option<&str>,
    model_label: Option<&str>,
    options: RenderOptions,
) {
    let controls = footer_controls(view, profile_label, model_label, area);
    for (rect, label, action) in controls {
        match action {
            FooterAction::Help => {
                buffer.set_stringn(
                    rect.x,
                    rect.y,
                    label,
                    usize::from(rect.width),
                    fg(options, MUTED),
                );
            }
            _ => {
                buffer.set_stringn(rect.x, rect.y, "[", 1, fg(options, RULE));
                let color = match action {
                    FooterAction::ToggleMode => match view.mode {
                        Mode::Auto => Color::Rgb(245, 194, 231),
                        Mode::Plan => Color::Rgb(249, 226, 175),
                    },
                    FooterAction::Model => TEXT,
                    FooterAction::Profile => MUTED,
                    _ => MUTED,
                };
                buffer.set_stringn(
                    rect.x.saturating_add(2),
                    rect.y,
                    label,
                    usize::from(rect.width.saturating_sub(4)),
                    fg(options, color),
                );
                buffer.set_stringn(
                    rect.right().saturating_sub(1),
                    rect.y,
                    "]",
                    1,
                    fg(options, RULE),
                );
            }
        }
    }
}

/// A functional footer control shared by painting and mouse dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FooterAction {
    /// Open the active keymap help.
    Help,
    /// Toggle the local auto/plan preference.
    ToggleMode,
    /// Inspect the active model/profile.
    Model,
    /// Open the connection profile picker.
    Profile,
    /// Inspect or change local effort preferences.
    Effort,
}

fn footer_controls(
    view: &ViewState,
    profile_label: Option<&str>,
    model_label: Option<&str>,
    area: Rect,
) -> Vec<(Rect, String, FooterAction)> {
    if area.height == 0 || area.width < 24 {
        return Vec::new();
    }
    let mode = match view.mode {
        Mode::Auto => "auto mode on",
        Mode::Plan => "plan mode on",
    };
    let effort = view.effort.name();
    let mode_width = u16::try_from(mode.len())
        .map_or(u16::MAX, |n| n)
        .saturating_add(4);
    let effort_width = u16::try_from(effort.len())
        .map_or(u16::MAX, |n| n)
        .saturating_add(4);
    let profile = profile_label.map(|label| crate::text::fit_display_text(label, 12));
    let profile_width = profile.as_ref().map_or(0, |label| {
        u16::try_from(UnicodeWidthStr::width(label.as_str()))
            .map_or(u16::MAX, |n| n)
            .saturating_add(4)
    });
    let reserved = mode_width
        .saturating_add(effort_width)
        .saturating_add(profile_width)
        .saturating_add(8);
    let model_width = area.width.saturating_sub(reserved).max(5);
    let model = match model_label {
        Some(label) => crate::text::fit_display_text(label, model_width.saturating_sub(4)),
        None => crate::text::fit_display_text(view.model.as_str(), model_width.saturating_sub(4)),
    };
    let model_width = u16::try_from(UnicodeWidthStr::width(model.as_str()))
        .map_or(u16::MAX, |n| n)
        .saturating_add(4);
    let total = mode_width
        .saturating_add(model_width)
        .saturating_add(effort_width)
        .saturating_add(profile_width)
        .saturating_add(3);
    let mut x = area.right().saturating_sub(total);
    let mut controls = Vec::with_capacity(5);
    if x.saturating_sub(area.x) >= 17 {
        controls.push((
            Rect::new(area.x, area.y, 15, 1),
            "? for shortcuts".to_owned(),
            FooterAction::Help,
        ));
    }
    let mut labels = vec![
        (mode.to_owned(), mode_width, FooterAction::ToggleMode),
        (model, model_width, FooterAction::Model),
    ];
    if let Some(profile) = profile {
        labels.push((profile, profile_width, FooterAction::Profile));
    }
    labels.push((effort.to_owned(), effort_width, FooterAction::Effort));
    for (label, width, action) in labels {
        controls.push((Rect::new(x, area.y, width, 1), label, action));
        x = x.saturating_add(width).saturating_add(1);
    }
    controls
}

/// Resolve a footer click using the same current-view geometry as painting.
#[must_use]
pub fn footer_action_at(view: &ViewState, viewport: Rect, x: u16, y: u16) -> Option<FooterAction> {
    footer_action_at_with_labels(view, None, None, viewport, x, y)
}

/// Resolve a footer click using screen state that includes the profile control.
#[must_use]
pub fn footer_action_at_with_profile(
    view: &ViewState,
    profile_label: Option<&str>,
    viewport: Rect,
    x: u16,
    y: u16,
) -> Option<FooterAction> {
    footer_action_at_with_labels(view, profile_label, None, viewport, x, y)
}

/// Resolve a footer click using the same profile and friendly-model labels as rendering.
#[must_use]
pub fn footer_action_at_with_labels(
    view: &ViewState,
    profile_label: Option<&str>,
    model_label: Option<&str>,
    viewport: Rect,
    x: u16,
    y: u16,
) -> Option<FooterAction> {
    if viewport.width < crate::layout::MIN_WIDTH
        || viewport.height < crate::layout::MIN_HEIGHT
        || !dimensions_supported(viewport.width, viewport.height)
    {
        return None;
    }
    let area = Rect::new(
        viewport.x.saturating_add(4),
        viewport.bottom().saturating_sub(1),
        viewport.width.saturating_sub(8),
        1,
    );
    footer_controls(view, profile_label, model_label, area)
        .into_iter()
        .find(|(r, _, _)| r.contains(Position::new(x, y)))
        .map(|(_, _, action)| action)
}

fn visible_draft_lines(composer: &Composer, terminal_width: u16) -> u16 {
    u16::try_from(
        composer_geometry(composer, terminal_width.saturating_sub(10).max(1))
            .rows()
            .len(),
    )
    .map_or(u16::MAX, |n| n)
}

#[derive(Debug, Clone)]
struct RenderRow {
    text: String,
    cell_sources: Vec<DisplayCell>,
    decorations: Vec<InlineDecoration>,
    gutter: Option<String>,
    fg: Color,
    bg: Option<Color>,
    entry_index: usize,
    source_bytes: Range<usize>,
    /// Entry index for a title row that toggles a local presentation tool.
    presentation_tool: Option<usize>,
}

impl RenderRow {
    fn gutter_width(&self) -> u16 {
        match self.gutter.as_ref() {
            Some(gutter) => u16::try_from(UnicodeWidthStr::width(gutter.as_str()))
                .map_or(u16::MAX, |width| width),
            None => 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineDecoration {
    source_bytes: Range<usize>,
    kind: InlineDecorationKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineDecorationKind {
    Command,
    Directive,
    Mention,
    Link,
    Keyword,
    String,
    Number,
    Type,
    Comment,
}

fn transcript_rows_with_cache(
    view: &ViewState,
    width: u16,
    presentation_tools: &[PresentationTool],
    mut cache: Option<&mut RenderCache>,
) -> Vec<RenderRow> {
    if width == 0 {
        return Vec::new();
    }
    let mut entries: Vec<_> = view
        .conversation
        .iter()
        .enumerate()
        .rev()
        .take(MAX_RENDERED_ENTRIES)
        .collect();
    entries.reverse();
    let first_index = view.conversation.len().saturating_sub(entries.len());
    let mut rows = Vec::new();
    let mut total_bytes = 0_usize;
    if first_index > 0 {
        rows.push(RenderRow {
            text: "Earlier conversation not shown".to_owned(),
            cell_sources: Vec::new(),
            decorations: Vec::new(),
            gutter: None,
            fg: MUTED,
            bg: None,
            entry_index: first_index,
            source_bytes: 0..0,
            presentation_tool: None,
        });
    }
    for (entry_index, entry) in entries {
        if rows.len() >= MAX_RENDERED_ROWS || total_bytes >= MAX_TRANSCRIPT_SOURCE_BYTES {
            break;
        }
        if !rows.is_empty() {
            rows.push(RenderRow {
                text: String::new(),
                cell_sources: Vec::new(),
                decorations: Vec::new(),
                gutter: None,
                fg: TEXT,
                bg: None,
                entry_index,
                source_bytes: 0..0,
                presentation_tool: None,
            });
        }
        let source_limit = MAX_TRANSCRIPT_SOURCE_BYTES.saturating_sub(total_bytes);
        let row_limit = MAX_RENDERED_ROWS.saturating_sub(rows.len());
        let presentation_tool = presentation_tools
            .iter()
            .find(|tool| tool.entry_index == entry_index)
            .copied();
        let key = RenderCacheKey {
            entry_index,
            kind: entry.kind,
            width,
            source_limit,
            row_limit,
            presentation_tool,
            presentation_title_len: presentation_tool.map(|_| {
                entry
                    .text
                    .split_once('\n')
                    .map_or(entry.text.len(), |(title, _)| title.len())
            }),
        };
        let snapshot = entry_cache_snapshot(entry, source_limit, presentation_tool);
        let rendered = match cache.as_deref_mut() {
            Some(render_cache) => render_cache.entry_rows(key, snapshot, || {
                render_entry_rows(
                    entry,
                    entry_index,
                    width,
                    source_limit,
                    row_limit,
                    presentation_tool,
                )
            }),
            None => render_entry_rows(
                entry,
                entry_index,
                width,
                source_limit,
                row_limit,
                presentation_tool,
            ),
        };
        total_bytes = total_bytes.saturating_add(rendered.source_bytes);
        rows.extend(rendered.rows);
    }
    rows
}

const MAX_TRANSCRIPT_SOURCE_BYTES: usize = 262_144;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RenderCacheKey {
    entry_index: usize,
    kind: EntryKind,
    width: u16,
    source_limit: usize,
    row_limit: usize,
    presentation_tool: Option<PresentationTool>,
    presentation_title_len: Option<usize>,
}

#[derive(Debug)]
struct CachedEntryRows {
    key: RenderCacheKey,
    source_snapshot: String,
    rows: Vec<RenderRow>,
    source_bytes: usize,
    accounted_bytes: usize,
}

#[derive(Debug)]
struct EntryRender {
    rows: Vec<RenderRow>,
    source_bytes: usize,
}

impl RenderCache {
    fn entry_rows(
        &mut self,
        key: RenderCacheKey,
        source_snapshot: String,
        render: impl FnOnce() -> EntryRender,
    ) -> EntryRender {
        if let Some(position) = self
            .entries
            .iter()
            .position(|cached| cached.key == key && cached.source_snapshot == source_snapshot)
        {
            if let Some(cached) = self.entries.remove(position) {
                self.hits = self.hits.saturating_add(1);
                let result = EntryRender {
                    rows: cached.rows.clone(),
                    source_bytes: cached.source_bytes,
                };
                self.entries.push_back(cached);
                return result;
            }
        }
        self.misses = self.misses.saturating_add(1);
        let rendered = render();
        let accounted_bytes = cache_entry_size(
            source_snapshot.capacity(),
            rendered.rows.capacity(),
            &rendered.rows,
        );
        let snapshot_bytes = source_snapshot.capacity();
        if self.max_entries > 0
            && accounted_bytes <= self.byte_budget
            && snapshot_bytes <= MAX_RENDER_CACHE_SOURCE_BYTES
        {
            while self.resident_bytes.saturating_add(accounted_bytes) > self.byte_budget
                || self.source_snapshot_bytes.saturating_add(snapshot_bytes)
                    > MAX_RENDER_CACHE_SOURCE_BYTES
            {
                let Some(evicted) = self.entries.pop_front() else {
                    break;
                };
                self.resident_bytes = self.resident_bytes.saturating_sub(evicted.accounted_bytes);
                self.source_snapshot_bytes = self
                    .source_snapshot_bytes
                    .saturating_sub(evicted.source_snapshot.capacity());
                self.evictions = self.evictions.saturating_add(1);
            }
            while self.entries.len() >= self.max_entries {
                let Some(evicted) = self.entries.pop_front() else {
                    break;
                };
                self.resident_bytes = self.resident_bytes.saturating_sub(evicted.accounted_bytes);
                self.source_snapshot_bytes = self
                    .source_snapshot_bytes
                    .saturating_sub(evicted.source_snapshot.capacity());
                self.evictions = self.evictions.saturating_add(1);
            }
            if self.resident_bytes.saturating_add(accounted_bytes) <= self.byte_budget
                && self.source_snapshot_bytes.saturating_add(snapshot_bytes)
                    <= MAX_RENDER_CACHE_SOURCE_BYTES
            {
                self.resident_bytes = self.resident_bytes.saturating_add(accounted_bytes);
                self.source_snapshot_bytes =
                    self.source_snapshot_bytes.saturating_add(snapshot_bytes);
                self.entries.push_back(CachedEntryRows {
                    key,
                    source_snapshot,
                    rows: rendered.rows.clone(),
                    source_bytes: rendered.source_bytes,
                    accounted_bytes,
                });
            }
        }
        rendered
    }
}

fn cache_entry_size(snapshot_capacity: usize, rows_capacity: usize, rows: &[RenderRow]) -> usize {
    let row_bytes = rows.iter().fold(0_usize, |total, row| {
        total
            .saturating_add(row.text.capacity())
            .saturating_add(row.gutter.as_ref().map_or(0, String::capacity))
            .saturating_add(
                row.cell_sources
                    .capacity()
                    .saturating_mul(core::mem::size_of::<DisplayCell>()),
            )
            .saturating_add(
                row.decorations
                    .capacity()
                    .saturating_mul(core::mem::size_of::<InlineDecoration>()),
            )
    });
    snapshot_capacity
        .saturating_add(rows_capacity.saturating_mul(core::mem::size_of::<RenderRow>()))
        .saturating_add(row_bytes)
}

fn entry_cache_snapshot(
    entry: &polaris_ui_model::view::Entry,
    source_limit: usize,
    presentation_tool: Option<PresentationTool>,
) -> String {
    let Some(tool) = presentation_tool else {
        return bounded_text(
            &entry.text,
            source_limit.min(crate::text::MAX_ENTRY_SOURCE_BYTES),
        )
        .to_owned();
    };
    let (title, body) = entry
        .text
        .split_once('\n')
        .map_or((entry.text.as_str(), ""), |parts| parts);
    let title_len = title.len();
    let title = bounded_text(title, crate::text::MAX_ENTRY_SOURCE_BYTES);
    let body_limit = if tool.expanded {
        source_limit
            .saturating_sub(title_len.saturating_add(1))
            .min(crate::text::MAX_ENTRY_SOURCE_BYTES)
    } else {
        0
    };
    let body = bounded_text(body, body_limit);
    let mut snapshot =
        String::with_capacity(title.len().saturating_add(body.len()).saturating_add(1));
    snapshot.push_str(title);
    snapshot.push('\n');
    snapshot.push_str(body);
    snapshot
}

fn render_entry_rows(
    entry: &polaris_ui_model::view::Entry,
    entry_index: usize,
    width: u16,
    source_limit: usize,
    row_limit: usize,
    presentation_tool: Option<PresentationTool>,
) -> EntryRender {
    if width == 0 || row_limit == 0 {
        return EntryRender {
            rows: Vec::new(),
            source_bytes: 0,
        };
    }
    let mut rows = Vec::new();
    // User, assistant and system notices carry no role heading (reference design); system
    // notices are distinguished by their muted body color instead.
    let (label, color) = match entry.kind {
        EntryKind::User | EntryKind::Assistant | EntryKind::System => ("", TEXT),
        EntryKind::Warning => ("Warning", WARNING),
        EntryKind::Error => ("Error", ERROR),
    };
    if !label.is_empty() {
        rows.push(RenderRow {
            text: label.to_owned(),
            cell_sources: Vec::new(),
            decorations: Vec::new(),
            gutter: None,
            fg: color,
            bg: None,
            entry_index,
            source_bytes: 0..0,
            presentation_tool: None,
        });
    }
    let mut source_bytes = 0_usize;
    if let Some(tool) = presentation_tool {
        let (title, body) = entry
            .text
            .split_once('\n')
            .map_or((entry.text.as_str(), ""), |parts| parts);
        let status = match tool.status {
            PresentationToolStatus::Working => "Working",
            PresentationToolStatus::Completed => "Completed",
            PresentationToolStatus::Failed => "Failed",
        };
        let duration = tool
            .duration_ms
            .map_or_else(String::new, |ms| format!(" {ms}ms"));
        let suffix = format!(
            "{status}{duration} · Ctrl+O {}",
            if tool.expanded { "collapse" } else { "expand" }
        );
        let suffix_width = u16::try_from(UnicodeWidthStr::width(suffix.as_str()))
            .map_or(u16::MAX, |n| n)
            .saturating_add(3);
        let title_width = width.saturating_sub(suffix_width).max(1);
        if let Some(mut display) = wrap_display_text(title, title_width, 0).into_iter().next() {
            for cell in &mut display.cells {
                cell.cells.start = cell.cells.start.saturating_add(1);
                cell.cells.end = cell.cells.end.saturating_add(1);
            }
            rows.push(RenderRow {
                text: format!("[{}] {suffix}", display.text),
                cell_sources: display.cells,
                decorations: Vec::new(),
                gutter: None,
                fg: GOLD,
                bg: None,
                entry_index,
                source_bytes: display.source_bytes,
                presentation_tool: Some(entry_index),
            });
        }
        source_bytes = source_bytes.saturating_add(title.len()).saturating_add(1);
        if !tool.expanded || body.is_empty() || rows.len() >= row_limit {
            return EntryRender { rows, source_bytes };
        }
        let remaining = source_limit
            .saturating_sub(title.len().saturating_add(1))
            .min(crate::text::MAX_ENTRY_SOURCE_BYTES);
        let body = bounded_text(body, remaining);
        let mut source_base = title.len().saturating_add(1);
        for raw_with_newline in body.split_inclusive('\n') {
            if rows.len() >= row_limit {
                break;
            }
            let raw_line = raw_with_newline
                .strip_suffix('\n')
                .map_or(raw_with_newline, |line| line);
            let decorations = source_decorations(raw_line, source_base, false);
            for display in wrap_display_text(raw_line, width, source_base) {
                if rows.len() >= row_limit {
                    break;
                }
                rows.push(RenderRow {
                    text: display.text,
                    cell_sources: display.cells,
                    decorations: decorations
                        .iter()
                        .filter(|decoration| {
                            decoration.source_bytes.start < display.source_bytes.end
                                && display.source_bytes.start < decoration.source_bytes.end
                        })
                        .cloned()
                        .collect(),
                    gutter: None,
                    fg: match entry.kind {
                        EntryKind::Warning => WARNING,
                        EntryKind::System => MUTED,
                        EntryKind::User | EntryKind::Assistant | EntryKind::Error => TEXT,
                    },
                    bg: None,
                    entry_index,
                    source_bytes: display.source_bytes,
                    presentation_tool: None,
                });
            }
            source_bytes = source_bytes.saturating_add(raw_with_newline.len());
            source_base = source_base.saturating_add(raw_with_newline.len());
        }
        return EntryRender { rows, source_bytes };
    }
    let source = bounded_text(
        &entry.text,
        source_limit.min(crate::text::MAX_ENTRY_SOURCE_BYTES),
    );
    let mut source_base = 0_usize;
    let mut code_language: Option<String> = None;
    let mut code_line = 1_usize;
    for raw_with_newline in source.split_inclusive('\n') {
        if rows.len() >= row_limit {
            break;
        }
        let raw_line = raw_with_newline
            .strip_suffix('\n')
            .map_or(raw_with_newline, |line| line);
        let trimmed = raw_line.trim_start();
        if let Some(fence) = trimmed.strip_prefix("\x60\x60\x60") {
            code_language = match code_language.take() {
                Some(_) => None,
                None => Some(fence.trim().to_ascii_lowercase()),
            };
            code_line = 1;
            source_bytes = source_bytes.saturating_add(raw_with_newline.len());
            source_base = source_base.saturating_add(raw_with_newline.len());
            continue;
        }
        let in_fence = code_language.is_some();
        let gutter = if in_fence {
            Some(format!("  {code_line:>4} "))
        } else {
            None
        };
        let gutter_width = match gutter.as_ref() {
            Some(prefix) => u16::try_from(UnicodeWidthStr::width(prefix.as_str()))
                .map_or(width, |value| value.min(width)),
            None => 0,
        };
        let content_width = width.saturating_sub(gutter_width).max(1);
        let diff = if trimmed.starts_with('+') && !trimmed.starts_with("+++") {
            Some((ADD_FG, ADD_BG))
        } else if trimmed.starts_with('-') && !trimmed.starts_with("---") {
            Some((REMOVE_FG, REMOVE_BG))
        } else {
            None
        };
        let (line_fg, line_bg) = match diff {
            Some((fg_color, bg_color)) => (fg_color, Some(bg_color)),
            None if entry.kind == EntryKind::User => (color, Some(PROMPT_SURFACE)),
            None if in_fence => (MINT, Some(RAISED)),
            None => (color, None),
        };
        let decorations = if let Some(language) = code_language.as_deref() {
            source_code_decorations(raw_line, source_base, language)
        } else {
            source_decorations(raw_line, source_base, false)
        };
        for (display_index, mut display) in wrap_display_text(raw_line, content_width, source_base)
            .into_iter()
            .enumerate()
        {
            if rows.len() >= row_limit {
                break;
            }
            let row_gutter = if display_index == 0 {
                gutter.clone()
            } else if in_fence {
                Some("       ".to_owned())
            } else {
                None
            };
            rows.push(RenderRow {
                text: display.text,
                cell_sources: core::mem::take(&mut display.cells),
                decorations: decorations
                    .iter()
                    .filter(|decoration| {
                        decoration.source_bytes.start < display.source_bytes.end
                            && display.source_bytes.start < decoration.source_bytes.end
                    })
                    .cloned()
                    .collect(),
                gutter: row_gutter,
                fg: line_fg,
                bg: line_bg,
                entry_index,
                source_bytes: display.source_bytes,
                presentation_tool: None,
            });
        }
        if in_fence {
            code_line = code_line.saturating_add(1);
        }
        source_bytes = source_bytes.saturating_add(raw_with_newline.len());
        source_base = source_base.saturating_add(raw_with_newline.len());
    }
    EntryRender { rows, source_bytes }
}

fn bounded_text(input: &str, limit: usize) -> &str {
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

fn source_decorations(line: &str, source_base: usize, in_code: bool) -> Vec<InlineDecoration> {
    if in_code {
        return Vec::new();
    }
    let mut decorations = Vec::new();
    for (start, sigil) in line
        .char_indices()
        .filter(|(_, ch)| matches!(ch, '/' | '$' | '@'))
    {
        let boundary = start == 0
            || line
                .get(..start)
                .and_then(|prefix| prefix.chars().next_back())
                .is_some_and(|ch| ch.is_whitespace() || "([{<\"'".contains(ch));
        if !boundary {
            continue;
        }
        let Some(tail) = line.get(start..) else {
            continue;
        };
        let Some(span) = polaris_ui_model::classify::classify(tail)
            .into_iter()
            .next()
        else {
            continue;
        };
        let kind = match span.class {
            polaris_ui_model::classify::TokenClass::Command if sigil == '/' => {
                InlineDecorationKind::Command
            }
            polaris_ui_model::classify::TokenClass::Directive if sigil == '$' => {
                InlineDecorationKind::Directive
            }
            polaris_ui_model::classify::TokenClass::Mention if sigil == '@' => {
                InlineDecorationKind::Mention
            }
            polaris_ui_model::classify::TokenClass::Command
            | polaris_ui_model::classify::TokenClass::Directive
            | polaris_ui_model::classify::TokenClass::Mention
            | polaris_ui_model::classify::TokenClass::CommandUnknown
            | polaris_ui_model::classify::TokenClass::DirectiveUnknown => continue,
        };
        decorations.push(InlineDecoration {
            source_bytes: source_base.saturating_add(start)
                ..source_base.saturating_add(start.saturating_add(span.end)),
            kind,
        });
    }
    decorations.extend(link_decorations(line, source_base));
    decorations.sort_by_key(|span| span.source_bytes.start);
    decorations
}

fn link_decorations(line: &str, source_base: usize) -> Vec<InlineDecoration> {
    let mut decorations = Vec::new();
    let mut cursor = 0_usize;
    while cursor < line.len() {
        let Some(tail) = line.get(cursor..) else {
            break;
        };
        let start = ["https://", "http://", "www."]
            .iter()
            .filter_map(|prefix| {
                tail.find(prefix)
                    .map(|offset| cursor.saturating_add(offset))
            })
            .min();
        let Some(start) = start else {
            break;
        };
        let boundary = start == 0
            || line
                .get(..start)
                .and_then(|prefix| prefix.chars().next_back())
                .is_some_and(|ch| ch.is_whitespace() || "([{<\"'".contains(ch));
        let Some(link_tail) = line.get(start..) else {
            break;
        };
        let length = link_tail
            .char_indices()
            .take_while(|(_, ch)| !(ch.is_whitespace() || ch.is_control() || "<>\"'".contains(*ch)))
            .last()
            .map_or(0, |(offset, ch)| offset.saturating_add(ch.len_utf8()));
        let mut end = start.saturating_add(length);
        while end > start {
            let Some(candidate) = line.get(start..end) else {
                break;
            };
            let Some(last) = candidate.chars().next_back() else {
                break;
            };
            if ".,;:!?)]}".contains(last) {
                end = end.saturating_sub(last.len_utf8());
            } else {
                break;
            }
        }
        let valid_scheme = link_tail.starts_with("https://") || link_tail.starts_with("http://");
        let valid_www = link_tail.starts_with("www.") && end > start.saturating_add(4);
        if boundary && end > start && (valid_scheme || valid_www) {
            decorations.push(InlineDecoration {
                source_bytes: source_base.saturating_add(start)..source_base.saturating_add(end),
                kind: InlineDecorationKind::Link,
            });
            cursor = end;
        } else {
            cursor = start.saturating_add(1);
        }
    }
    decorations
}

fn source_code_decorations(
    line: &str,
    source_base: usize,
    language: &str,
) -> Vec<InlineDecoration> {
    if !matches!(
        language,
        "rust" | "rs" | "python" | "py" | "javascript" | "js" | "typescript" | "ts"
    ) {
        return Vec::new();
    }
    let mut decorations = Vec::new();
    let mut cursor = 0_usize;
    while cursor < line.len() {
        let Some(tail) = line.get(cursor..) else {
            break;
        };
        let Some(ch) = tail.chars().next() else {
            break;
        };
        let mut end = cursor.saturating_add(ch.len_utf8());
        let kind = if tail.starts_with("//") || (ch == '#' && matches!(language, "py" | "python")) {
            end = line.len();
            InlineDecorationKind::Comment
        } else if ch == '"' || ch == '\'' {
            let mut escaped = false;
            for (offset, next) in tail.char_indices().skip(1) {
                end = cursor
                    .saturating_add(offset)
                    .saturating_add(next.len_utf8());
                if next == ch && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
            }
            InlineDecorationKind::String
        } else if ch.is_ascii_digit() {
            end = cursor.saturating_add(
                tail.char_indices()
                    .take_while(|(_, next)| {
                        next.is_ascii_alphanumeric() || matches!(next, '.' | '_')
                    })
                    .last()
                    .map_or(ch.len_utf8(), |(offset, next)| {
                        offset.saturating_add(next.len_utf8())
                    }),
            );
            InlineDecorationKind::Number
        } else if ch.is_alphabetic() || ch == '_' {
            end = cursor.saturating_add(
                tail.char_indices()
                    .take_while(|(_, next)| next.is_alphanumeric() || *next == '_')
                    .last()
                    .map_or(ch.len_utf8(), |(offset, next)| {
                        offset.saturating_add(next.len_utf8())
                    }),
            );
            let word = match line.get(cursor..end) {
                Some(word) => word,
                None => {
                    cursor = end;
                    continue;
                }
            };
            if matches!(
                word,
                "pub"
                    | "fn"
                    | "let"
                    | "mut"
                    | "const"
                    | "static"
                    | "use"
                    | "mod"
                    | "impl"
                    | "trait"
                    | "struct"
                    | "enum"
                    | "match"
                    | "if"
                    | "else"
                    | "return"
                    | "for"
                    | "while"
                    | "loop"
                    | "in"
                    | "as"
                    | "async"
                    | "await"
                    | "move"
                    | "self"
                    | "Self"
                    | "true"
                    | "false"
                    | "None"
                    | "Some"
                    | "def"
                    | "class"
                    | "import"
                    | "from"
                    | "and"
                    | "or"
                    | "not"
                    | "function"
                    | "export"
                    | "default"
                    | "new"
                    | "null"
                    | "undefined"
                    | "throw"
                    | "try"
                    | "type"
                    | "interface"
            ) {
                InlineDecorationKind::Keyword
            } else if ch.is_uppercase() {
                InlineDecorationKind::Type
            } else {
                cursor = end;
                continue;
            }
        } else {
            cursor = end;
            continue;
        };
        decorations.push(InlineDecoration {
            source_bytes: source_base.saturating_add(cursor)..source_base.saturating_add(end),
            kind,
        });
        cursor = end;
    }
    decorations
}

fn decoration_style(options: RenderOptions, kind: InlineDecorationKind) -> Style {
    match kind {
        InlineDecorationKind::Command => fg(options, Color::Rgb(148, 205, 64)),
        InlineDecorationKind::Directive => fg(options, DIRECTIVE),
        InlineDecorationKind::Mention => fg(options, GOLD),
        InlineDecorationKind::Link => Style::default().add_modifier(Modifier::UNDERLINED),
        InlineDecorationKind::Keyword => fg(options, Color::Rgb(203, 166, 247)),
        InlineDecorationKind::String => fg(options, Color::Rgb(182, 227, 91)),
        InlineDecorationKind::Number => fg(options, GOLD),
        InlineDecorationKind::Type => fg(options, Color::Rgb(137, 220, 235)),
        InlineDecorationKind::Comment => fg(options, MUTED),
    }
}

fn draw_body(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[RenderRow],
    state: &ScreenState<'_>,
    options: RenderOptions,
) {
    if area.width < 3 || area.height == 0 {
        return;
    }
    if rows.is_empty() && state.view.goal.is_none() && state.view.tools.is_empty() {
        draw_welcome(buffer, area, options);
        return;
    }
    let count = usize::from(area.height);
    let max_scroll = rows.len().saturating_sub(count);
    let end = rows
        .len()
        .saturating_sub(state.history_scroll.min(max_scroll));
    let start = end.saturating_sub(count);
    for (row, item) in rows
        .iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .enumerate()
    {
        let y = area.y.saturating_add(match u16::try_from(row) {
            Ok(value) => value,
            Err(_) => continue,
        });
        let x = area.x.saturating_add(1);
        let width = area.width.saturating_sub(2);
        let gutter_width = item.gutter_width();
        let source_x = x.saturating_add(gutter_width);
        buffer.set_style(Rect::new(x, y, width, 1), styled(options, item.fg, item.bg));
        buffer.set_stringn(
            source_x,
            y,
            &item.text,
            usize::from(width.saturating_sub(gutter_width)),
            styled(options, item.fg, item.bg),
        );
        if let Some(gutter) = item.gutter.as_ref() {
            buffer.set_stringn(x, y, gutter, usize::from(width), fg(options, MUTED));
        }
        for cell in &item.cell_sources {
            let cell_width = cell.cells.end.saturating_sub(cell.cells.start);
            if cell_width == 0 {
                continue;
            }
            if let (Ok(dx), Ok(cell_width)) =
                (u16::try_from(cell.cells.start), u16::try_from(cell_width))
            {
                let rect = Rect::new(source_x.saturating_add(dx), y, cell_width, 1);
                if let Some(decoration) = item.decorations.iter().find(|decoration| {
                    decoration.source_bytes.start < cell.source_bytes.end
                        && cell.source_bytes.start < decoration.source_bytes.end
                }) {
                    buffer.set_style(rect, decoration_style(options, decoration.kind));
                }
                if transcript_cell_selected(state, item.entry_index, cell.source_bytes.clone()) {
                    shade_selection(buffer, rect, options);
                }
            }
        }
    }
}

fn transcript_cell_selected(
    state: &ScreenState<'_>,
    entry_index: usize,
    source_bytes: Range<usize>,
) -> bool {
    if let Some(selection) = state.transcript_span_selection {
        let (start, end) = if selection.anchor <= selection.head {
            (selection.anchor, selection.head)
        } else {
            (selection.head, selection.anchor)
        };
        if entry_index < start.entry_index || entry_index > end.entry_index {
            return false;
        }
        let selected_start = if entry_index == start.entry_index {
            start.source_byte
        } else {
            0
        };
        let selected_end = if entry_index == end.entry_index {
            end.source_byte
        } else {
            usize::MAX
        };
        return selected_start < source_bytes.end && source_bytes.start < selected_end;
    }
    state
        .transcript_selection
        .as_ref()
        .is_some_and(|selection| {
            selection.entry_index == entry_index
                && selection.source_bytes.start < source_bytes.end
                && source_bytes.start < selection.source_bytes.end
        })
}

fn draw_welcome(buffer: &mut Buffer, area: Rect, options: RenderOptions) {
    let large = [
        "    ╷    ",
        "    │    ",
        "╶───✦───╴",
        "    │    ",
        "    ╵    ",
    ];
    let compact = ["  ╷  ", "╶─✦─╴", "  ╵  "];
    let (art, w, h): (&[&str], u16, u16) = if area.height >= 9 {
        (&large, 9, 5)
    } else if area.height >= 5 {
        (&compact, 5, 3)
    } else {
        return;
    };
    if area.width < w {
        return;
    }
    let x = area
        .x
        .saturating_add(area.width.saturating_sub(w).checked_div(2).map_or(0, |n| n));
    let y = area.y.saturating_add(
        area.height
            .saturating_sub(h)
            .saturating_sub(2)
            .checked_div(2)
            .map_or(0, |n| n),
    );
    for (row, line) in art.iter().enumerate() {
        for (column, glyph) in line.chars().enumerate() {
            if glyph == ' ' {
                continue;
            }
            let near = column
                .abs_diff(usize::from(w.checked_div(2).map_or(0, |n| n)))
                .saturating_add(row.abs_diff(usize::from(h.checked_div(2).map_or(0, |n| n))))
                == 1;
            let tone = if glyph == '✦' {
                MINT
            } else if near {
                ACCENT
            } else {
                Color::Rgb(107, 107, 107)
            };
            let symbol = if options.ascii {
                match glyph {
                    '╷' | '│' | '╵' => "|",
                    '✦' => "*",
                    _ => "-",
                }
            } else {
                match glyph {
                    '╷' => "╷",
                    '│' => "│",
                    '╵' => "╵",
                    '✦' => "✦",
                    '╶' => "╶",
                    '╴' => "╴",
                    _ => "─",
                }
            };
            if let (Ok(dx), Ok(dy)) = (u16::try_from(column), u16::try_from(row)) {
                buffer.set_stringn(
                    x.saturating_add(dx),
                    y.saturating_add(dy),
                    symbol,
                    1,
                    fg(options, tone),
                );
            }
        }
    }
    Paragraph::new(polaris_ui_model::tokens::PLACEHOLDER_TEXT)
        .alignment(ratatui::layout::Alignment::Center)
        .style(fg(options, TEXT))
        .render(
            Rect::new(area.x, y.saturating_add(h).saturating_add(1), area.width, 1),
            buffer,
        );
}

fn draw_panel(buffer: &mut Buffer, area: Rect, state: &ScreenState<'_>, options: RenderOptions) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buffer.set_style(area, styled(options, TEXT, Some(RAISED)));
    buffer.set_stringn(area.x, area.y, "│", 1, fg(options, RULE));
    let content_area = Rect::new(
        area.x.saturating_add(2),
        area.y,
        area.width.saturating_sub(3),
        area.height,
    );
    if let Some(content) = state.panel_content {
        draw_custom_panel_content(buffer, content_area, content, options);
    } else {
        draw_derived_panel_content(buffer, content_area, state, options);
    }
}

fn draw_panel_sheet(
    buffer: &mut Buffer,
    area: Rect,
    state: &ScreenState<'_>,
    options: RenderOptions,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let height = area.height.min(10);
    let sheet = Rect::new(
        area.x,
        area.y.saturating_add(area.height.saturating_sub(height)),
        area.width,
        height,
    );
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(fg(options, RULE))
        .style(styled(options, TEXT, Some(RAISED)));
    let inner = block.inner(sheet);
    block.render(sheet, buffer);
    if let Some(content) = state.panel_content {
        draw_custom_panel_content(buffer, inner, content, options);
    } else {
        draw_derived_panel_content(buffer, inner, state, options);
    }
}

fn draw_custom_panel_content(
    buffer: &mut Buffer,
    area: Rect,
    content: InfoSheet<'_>,
    options: RenderOptions,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buffer.set_stringn(
        area.x,
        area.y,
        crate::text::fit_display_text(content.title, area.width),
        usize::from(area.width),
        fg(options, MINT).add_modifier(Modifier::BOLD),
    );
    let footer_rows = if content.footer.is_some() {
        1_u16
    } else {
        0_u16
    }
    .min(area.height.saturating_sub(1));
    let line_rows = area.height.saturating_sub(1).saturating_sub(footer_rows);
    for (row, line) in content
        .lines
        .iter()
        .skip(content.scroll)
        .take(usize::from(line_rows))
        .enumerate()
    {
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(match u16::try_from(row) {
                Ok(value) => value,
                Err(_) => continue,
            });
        buffer.set_stringn(
            area.x,
            y,
            crate::text::fit_display_text(line, area.width),
            usize::from(area.width),
            fg(options, TEXT),
        );
    }
    if let Some(footer) = content.footer {
        let y = area.y.saturating_add(area.height.saturating_sub(1));
        buffer.set_stringn(
            area.x,
            y,
            crate::text::fit_display_text(footer, area.width),
            usize::from(area.width),
            fg(options, MUTED),
        );
    }
}

fn draw_derived_panel_content(
    buffer: &mut Buffer,
    area: Rect,
    state: &ScreenState<'_>,
    options: RenderOptions,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    buffer.set_stringn(
        area.x,
        area.y,
        "Cockpit",
        usize::from(area.width),
        fg(options, MINT).add_modifier(Modifier::BOLD),
    );
    for (row, line) in panel_lines(state, area.width)
        .iter()
        .take(usize::from(area.height.saturating_sub(1)))
        .enumerate()
    {
        let y = area
            .y
            .saturating_add(1)
            .saturating_add(match u16::try_from(row) {
                Ok(value) => value,
                Err(_) => continue,
            });
        buffer.set_stringn(area.x, y, line, usize::from(area.width), fg(options, TEXT));
    }
}

fn panel_lines(state: &ScreenState<'_>, width: u16) -> Vec<String> {
    let view = state.view;
    let mut lines = Vec::new();
    lines.push(if view.status.session_open {
        "Session: open".to_owned()
    } else {
        "Session: unavailable".to_owned()
    });
    match &view.goal {
        Some(goal) => lines.push(crate::text::fit_display_text(
            &goal.label(state.now_ms),
            width,
        )),
        None => lines.push("Goal: none observed".to_owned()),
    }
    if view.hud.requests > 0 {
        lines.push(format!("Requests: {}", view.hud.requests));
        lines.push(format!("Prompt tokens: {}", view.hud.prompt_tokens));
        lines.push(format!("Completion tokens: {}", view.hud.completion_tokens));
    } else {
        lines.push("Usage: no completed request observed".to_owned());
    }
    if view.tools.is_empty() {
        lines.push("Tools: none observed".to_owned());
    } else {
        lines.push("Tool activity (latest observed)".to_owned());
        for tool in view.tools.iter().rev().take(4) {
            let status = match tool.status {
                ToolStatus::Running => "running",
                ToolStatus::Succeeded => "succeeded",
                ToolStatus::Failed => "failed",
            };
            lines.push(format!(
                "{status}: {}",
                crate::text::fit_display_text(&tool.title_text(), width)
            ));
        }
    }
    lines
}

fn clear_overlay(buffer: &mut Buffer, area: Rect, options: RenderOptions) {
    for y in area.y..area.bottom() {
        for x in area.x..area.right() {
            if let Some(cell) = buffer.cell_mut((x, y)) {
                cell.reset();
                cell.set_style(styled(options, TEXT, Some(CANVAS)));
            }
        }
    }
}
fn draw_info_sheet(buffer: &mut Buffer, body: Rect, sheet: InfoSheet<'_>, options: RenderOptions) {
    let area = overlay_rect(body, body.height.min(16));
    if area.width < 4 || area.height < 5 {
        return;
    }
    clear_overlay(buffer, area, options);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(fg(options, RULE))
        .style(styled(options, TEXT, Some(CANVAS)));
    let inner = block.inner(area);
    block.render(area, buffer);
    buffer.set_stringn(
        inner.x.saturating_add(1),
        inner.y,
        crate::text::fit_display_text(sheet.title, inner.width.saturating_sub(2)),
        usize::from(inner.width.saturating_sub(2)),
        fg(options, TEXT),
    );
    let rule = "─".repeat(usize::from(inner.width));
    buffer.set_stringn(
        inner.x,
        inner.y.saturating_add(1),
        rule,
        usize::from(inner.width),
        fg(options, RULE),
    );
    let footer_rows = u16::from(sheet.footer.is_some());
    let rows = inner.height.saturating_sub(2).saturating_sub(footer_rows);
    for (row, line) in sheet
        .lines
        .iter()
        .skip(sheet.scroll)
        .take(usize::from(rows))
        .enumerate()
    {
        if let Ok(dy) = u16::try_from(row) {
            buffer.set_stringn(
                inner.x.saturating_add(1),
                inner.y.saturating_add(2).saturating_add(dy),
                crate::text::fit_display_text(line, inner.width.saturating_sub(2)),
                usize::from(inner.width.saturating_sub(2)),
                fg(options, TEXT),
            );
        }
    }
    if let Some(footer) = sheet.footer {
        buffer.set_stringn(
            inner.x.saturating_add(1),
            inner.bottom().saturating_sub(1),
            crate::text::fit_display_text(footer, inner.width.saturating_sub(2)),
            usize::from(inner.width.saturating_sub(2)),
            fg(options, MUTED),
        );
    }
}

fn draw_completion(
    buffer: &mut Buffer,
    body: Rect,
    menu: CompletionOverlay<'_>,
    options: RenderOptions,
) {
    let area = overlay_rect(body, body.height.min(12));
    if area.width == 0 || area.height < 2 {
        return;
    }
    // Opaque across terminal width, including side margins, so no underlying glyphs leak.
    clear_overlay(
        buffer,
        Rect::new(buffer.area.x, area.y, buffer.area.width, area.height),
        options,
    );
    let visible = usize::from(area.height.saturating_sub(1));
    let selected = menu
        .selected
        .map(|i| i.min(menu.items.len().saturating_sub(1)));
    let first = selected.map_or(0, |i| i.saturating_add(1).saturating_sub(visible));
    let label_width = menu
        .items
        .iter()
        .map(|item| UnicodeWidthStr::width(item.label.as_str()))
        .max()
        .map_or(0, |n| n)
        .min(usize::from(area.width.checked_div(2).map_or(0, |n| n)));
    for (offset, item) in menu.items.iter().skip(first).take(visible).enumerate() {
        let Ok(dy) = u16::try_from(offset) else {
            continue;
        };
        let index = first.saturating_add(offset);
        let color = if selected == Some(index) {
            semantic_focus_color(item.kind)
        } else {
            MUTED
        };
        let label =
            crate::text::fit_display_text(&item.label, u16::try_from(label_width).map_or(0, |n| n));
        let y = area.y.saturating_add(1).saturating_add(dy);
        buffer.set_stringn(area.x, y, label, label_width, fg(options, color));
        let dx = u16::try_from(label_width.saturating_add(3)).map_or(area.width, |n| n);
        buffer.set_stringn(
            area.x.saturating_add(dx),
            y,
            crate::text::fit_display_text(item.description, area.width.saturating_sub(dx)),
            usize::from(area.width.saturating_sub(dx)),
            fg(options, MUTED),
        );
    }
}

fn semantic_focus_color(kind: CompletionKind) -> Color {
    match kind {
        CompletionKind::Command => Color::Rgb(182, 227, 91),
        CompletionKind::Mention => GOLD,
        CompletionKind::Directive => DIRECTIVE,
    }
}

fn remove_occluded_spans(spans: &mut Vec<SourceSpan>, opaque: Rect) {
    spans.retain(|span| {
        let span_right = span.rect.x.saturating_add(span.rect.width);
        let opaque_right = opaque.x.saturating_add(opaque.width);
        let span_bottom = span.rect.y.saturating_add(span.rect.height);
        let opaque_bottom = opaque.y.saturating_add(opaque.height);
        span_right <= opaque.x
            || opaque_right <= span.rect.x
            || span_bottom <= opaque.y
            || opaque_bottom <= span.rect.y
    });
}

fn overlay_rect(body: Rect, height: u16) -> Rect {
    let height = height.min(body.height);
    Rect::new(
        body.x,
        body.y.saturating_add(body.height.saturating_sub(height)),
        body.width,
        height,
    )
}

fn to_latest_rect(body: Rect) -> Option<Rect> {
    let width = u16::try_from("[to latest]".len()).ok()?;
    if body.width < width || body.height == 0 {
        return None;
    }
    Some(Rect::new(
        body.right().saturating_sub(width),
        body.bottom().saturating_sub(1),
        width,
        1,
    ))
}

fn draw_composer(
    buffer: &mut Buffer,
    area: Rect,
    composer: &Composer,
    options: RenderOptions,
) -> Option<CaretPoint> {
    if area.width < 7 || area.height < 3 {
        return None;
    }
    let rule = if options.ascii { "-" } else { "─" }.repeat(usize::from(area.width));
    buffer.set_stringn(
        area.x,
        area.y,
        &rule,
        usize::from(area.width),
        fg(options, RULE),
    );
    buffer.set_stringn(
        area.x,
        area.bottom().saturating_sub(1),
        &rule,
        usize::from(area.width),
        fg(options, RULE),
    );
    let content_width = area.width.saturating_sub(6);
    let visible_rows = usize::from(area.height.saturating_sub(2));
    let geometry = composer_geometry(composer, content_width);
    let cursor = geometry.caret_position(composer.cursor());
    let visible = geometry.visible_rows(visible_rows, cursor.row);
    let selection = composer.selection();
    for (row, line) in geometry
        .rows()
        .iter()
        .enumerate()
        .skip(visible.start)
        .take(visible.len())
    {
        if let Ok(dy) = u16::try_from(row.saturating_sub(visible.start)) {
            let x = area.x.saturating_add(4);
            let y = area.y.saturating_add(1).saturating_add(dy);
            buffer.set_stringn(
                x,
                y,
                &line.display_text,
                usize::from(content_width),
                fg(options, TEXT),
            );
            if let Some(range) = selection.as_ref() {
                for span in &line.spans {
                    let selected = span.source.start < range.end && range.start < span.source.end;
                    if selected {
                        let cell_width = span.cells.end.saturating_sub(span.cells.start);
                        if cell_width > 0
                            && let (Ok(dx), Ok(width)) =
                                (u16::try_from(span.cells.start), u16::try_from(cell_width))
                        {
                            shade_selection(
                                buffer,
                                Rect::new(x.saturating_add(dx), y, width, 1),
                                options,
                            );
                        }
                    }
                }
            }
        }
    }
    if composer.is_empty() {
        buffer.set_stringn(
            area.x.saturating_add(4),
            area.y.saturating_add(1),
            polaris_ui_model::tokens::PLACEHOLDER_TEXT,
            usize::from(content_width),
            fg(options, Color::Rgb(82, 82, 82)),
        );
    }
    buffer.set_stringn(
        area.x.saturating_add(2),
        area.y.saturating_add(1),
        ">",
        1,
        fg(options, MUTED),
    );
    caret_position_from_geometry(&geometry, composer.cursor(), area)
}

fn composer_geometry(composer: &Composer, width: u16) -> EditorGeometry {
    let chips = composer.chips();
    let ranges: Vec<_> = chips.into_iter().map(|chip| chip.range).collect();
    EditorGeometry::new(composer.text(), &ranges, width)
}

fn caret_position_from_geometry(
    geometry: &EditorGeometry,
    cursor: usize,
    area: Rect,
) -> Option<CaretPoint> {
    let visible_rows = usize::from(area.height.saturating_sub(2));
    if visible_rows == 0 {
        return None;
    }
    let position = geometry.caret_position(cursor);
    let visible = geometry.visible_rows(visible_rows, position.row);
    let visible_row = position.row.saturating_sub(visible.start);
    if visible_row >= visible.len() {
        return None;
    }
    let content_width = area.width.saturating_sub(6);
    let x_offset = 4_usize.saturating_add(
        position
            .column
            .min(usize::from(content_width).saturating_sub(1)),
    );
    Some(CaretPoint {
        x: area.x.saturating_add(u16::try_from(x_offset).ok()?),
        y: area
            .y
            .saturating_add(1)
            .saturating_add(u16::try_from(visible_row).ok()?),
    })
}

/// Return the caret cell using the renderer's Unicode grapheme wrapping rules.
#[must_use]
pub fn composer_caret_position(composer: &Composer, area: Rect) -> Option<CaretPoint> {
    if area.width < 7 || area.height < 3 {
        return None;
    }
    let geometry = composer_geometry(composer, area.width.saturating_sub(6));
    caret_position_from_geometry(&geometry, composer.cursor(), area)
}

/// Map a viewport mouse cell to a legal composer byte boundary using shared wrapping.
#[must_use]
pub fn composer_cursor_at(composer: &Composer, area: Rect, x: u16, y: u16) -> Option<usize> {
    composer_hit_at(composer, area, x, y).map(|hit| hit.cursor)
}

/// Map a viewport cell to a composer caret and, only when it covers rendered text,
/// the corresponding source grapheme.
#[must_use]
pub fn composer_hit_at(composer: &Composer, area: Rect, x: u16, y: u16) -> Option<ComposerHit> {
    if area.width < 7 || area.height < 3 {
        return None;
    }
    let right = area.x.saturating_add(area.width);
    let bottom = area.bottom().saturating_sub(1);
    if x < area.x || x >= right || y <= area.y || y >= bottom {
        return None;
    }
    let content_width = area.width.saturating_sub(6);
    let visible_rows = usize::from(area.height.saturating_sub(2));
    let geometry = composer_geometry(composer, content_width);
    let cursor = geometry.caret_position(composer.cursor());
    let visible = geometry.visible_rows(visible_rows, cursor.row);
    let target_row = visible
        .start
        .saturating_add(usize::from(y.saturating_sub(area.y.saturating_add(1))));
    if target_row >= visible.end {
        return None;
    }
    let text_x = area.x.saturating_add(4);
    let text_right = text_x.saturating_add(content_width);
    let target_column =
        usize::from(x.saturating_sub(text_x)).min(usize::from(content_width).saturating_sub(1));
    let source_bytes = if x >= text_x && x < text_right {
        geometry.source_at_cell(target_row, target_column)
    } else {
        None
    };
    Some(ComposerHit {
        cursor: geometry.cursor_at_cell(target_row, target_column),
        source_bytes,
    })
}

/// Return the byte boundary one wrapped row above or below the composer cursor.
#[must_use]
pub fn composer_vertical_cursor(composer: &Composer, area: Rect, down: bool) -> Option<usize> {
    composer_vertical_cursor_with_goal(composer, area, down, None).map(|(cursor, _goal)| cursor)
}

/// Move by one wrapped row while retaining a caller-owned sticky desired column.
#[must_use]
pub fn composer_vertical_cursor_with_goal(
    composer: &Composer,
    area: Rect,
    down: bool,
    desired_column: Option<usize>,
) -> Option<(usize, usize)> {
    if area.width < 7 {
        return None;
    }
    let geometry = composer_geometry(composer, area.width.saturating_sub(6));
    let current = geometry.caret_position(composer.cursor());
    let row_delta = if down { 1_isize } else { -1_isize };
    let target_row = if down {
        current.row.checked_add(1)?
    } else {
        current.row.checked_sub(1)?
    };
    if target_row >= geometry.rows().len() {
        return None;
    }
    Some(geometry.vertical_target(composer.cursor(), row_delta, desired_column))
}

/// Return whether the cursor is on the first and last actual wrapped draft rows.
#[must_use]
pub fn composer_visual_guards(composer: &Composer, area: Rect) -> (bool, bool) {
    if area.width < 7 {
        return (false, false);
    }
    let geometry = composer_geometry(composer, area.width.saturating_sub(6));
    let cursor = geometry.caret_position(composer.cursor());
    (
        cursor.row == 0,
        cursor.row.saturating_add(1) >= geometry.rows().len(),
    )
}

fn fg(options: RenderOptions, color: Color) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().fg(color),
        ColorMode::Terminal => Style::default().fg(portable_color(color)),
        ColorMode::Ansi => Style::default().fg(ansi_color(color)),
        ColorMode::NoColor => Style::default(),
    }
}

fn hover_style(
    target: HitTarget,
    state: &ScreenState<'_>,
    options: RenderOptions,
) -> Option<Style> {
    let style = match target {
        HitTarget::Footer(_) => control_hover_style(options),
        HitTarget::Completion(index) => {
            let kind = state
                .completion
                .and_then(|menu| menu.items.get(index))
                .map(|item| item.kind)?;
            semantic_hover_foreground(kind, options)
        }
        HitTarget::Tool(_) => semantic_foreground(options, GOLD_HOVER, Color::LightYellow),
        HitTarget::PresentationTool(_) => {
            semantic_foreground(options, GOLD_HOVER, Color::LightYellow)
        }
        HitTarget::ToLatest => latest_hover_style(options),
        HitTarget::QuestionChoice(_) => match options.colors {
            ColorMode::TrueColor => Style::default().bg(QUESTION_HOVER),
            ColorMode::Terminal | ColorMode::Ansi | ColorMode::NoColor => {
                Style::default().add_modifier(Modifier::UNDERLINED)
            }
        },
        HitTarget::QuestionTab(_)
        | HitTarget::QuestionAnswer
        | HitTarget::QuestionNotes
        | HitTarget::QuestionSubmit => control_hover_style(options),
        HitTarget::Transcript
        | HitTarget::Composer
        | HitTarget::Panel
        | HitTarget::Sheet
        | HitTarget::CompletionBackdrop => return None,
    };
    Some(style)
}

fn semantic_hover_foreground(kind: CompletionKind, options: RenderOptions) -> Style {
    let (rgb, portable, ansi) = match kind {
        CompletionKind::Command => (COMMAND_HOVER, Color::LightGreen, Color::LightGreen),
        CompletionKind::Directive => (DIRECTIVE_HOVER, Color::LightBlue, Color::LightBlue),
        CompletionKind::Mention => (GOLD_HOVER, Color::LightYellow, Color::White),
    };
    semantic_foreground(options, rgb, portable).patch(match options.colors {
        ColorMode::Ansi if matches!(kind, CompletionKind::Mention) => Style::default().fg(ansi),
        ColorMode::NoColor => Style::default().add_modifier(Modifier::BOLD),
        _ => Style::default(),
    })
}

fn semantic_foreground(options: RenderOptions, rgb: Color, fallback: Color) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().fg(rgb),
        ColorMode::Terminal => Style::default().fg(fallback),
        ColorMode::Ansi => Style::default().fg(fallback),
        ColorMode::NoColor => Style::default().add_modifier(Modifier::BOLD),
    }
}

fn control_hover_style(options: RenderOptions) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().bg(CONTROL_HOVER),
        ColorMode::Terminal | ColorMode::Ansi | ColorMode::NoColor => {
            Style::default().add_modifier(Modifier::UNDERLINED)
        }
    }
}

fn control_focus_style(options: RenderOptions) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().bg(CONTROL_FOCUS),
        ColorMode::Terminal | ColorMode::Ansi | ColorMode::NoColor => {
            Style::default().add_modifier(Modifier::UNDERLINED)
        }
    }
}

fn latest_base_style(options: RenderOptions) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().bg(LATEST_BG),
        ColorMode::Terminal | ColorMode::Ansi | ColorMode::NoColor => Style::default(),
    }
}

fn latest_hover_style(options: RenderOptions) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().bg(LATEST_HOVER),
        ColorMode::Terminal | ColorMode::Ansi | ColorMode::NoColor => {
            Style::default().add_modifier(Modifier::UNDERLINED)
        }
    }
}

fn shade_selection(buffer: &mut Buffer, rect: Rect, options: RenderOptions) {
    let Some(cell) = buffer.cell((rect.x, rect.y)) else {
        return;
    };
    let whitespace = cell.symbol().trim().is_empty();
    let source = if whitespace {
        TEXT
    } else {
        match cell.fg {
            Color::Rgb(_, _, _) => cell.fg,
            _ => TEXT,
        }
    };
    let background = match cell.bg {
        Color::Rgb(_, _, _) => cell.bg,
        _ => CANVAS,
    };
    let style = match options.colors {
        ColorMode::TrueColor => {
            let (source_red, source_green, source_blue) = match source {
                Color::Rgb(red, green, blue) => (red, green, blue),
                _ => (232, 232, 232),
            };
            let (back_red, back_green, back_blue) = match background {
                Color::Rgb(red, green, blue) => (red, green, blue),
                _ => (0, 0, 0),
            };
            let tint = polaris_ui_model::tokens::selection_tint(
                polaris_ui_model::tokens::Rgb(source_red, source_green, source_blue),
                polaris_ui_model::tokens::Rgb(back_red, back_green, back_blue),
            );
            Style::default().bg(Color::Rgb(tint.0, tint.1, tint.2))
        }
        ColorMode::Terminal | ColorMode::Ansi => Style::default().bg(Color::Indexed(237)),
        ColorMode::NoColor => Style::default().add_modifier(Modifier::BOLD),
    };
    buffer.set_style(rect, style);
}

fn styled(options: RenderOptions, foreground: Color, background: Option<Color>) -> Style {
    match options.colors {
        ColorMode::TrueColor => Style::default().fg(foreground).bg(match background {
            Some(color) => color,
            None => CANVAS,
        }),
        ColorMode::Terminal => Style::default()
            .fg(portable_color(foreground))
            .bg(Color::Reset),
        ColorMode::Ansi => Style::default()
            .fg(ansi_color(foreground))
            .bg(background.map_or(Color::Indexed(0), ansi_color)),
        ColorMode::NoColor => Style::default(),
    }
}

fn ansi_color(color: Color) -> Color {
    match color {
        Color::Rgb(0, 0, 0) => Color::Indexed(0),
        Color::Rgb(7, 35, 12) => Color::Indexed(2),
        Color::Rgb(50, 7, 9) => Color::Indexed(1),
        Color::Rgb(20, 20, 20) | Color::Rgb(28, 28, 28) | Color::Rgb(52, 52, 52) => {
            Color::Indexed(8)
        }
        Color::Rgb(118, 185, 0) | Color::Rgb(145, 205, 102) | Color::Rgb(182, 227, 91) => {
            Color::Indexed(10)
        }
        Color::Rgb(82, 82, 82) | Color::Rgb(107, 107, 107) => Color::Indexed(8),
        Color::Rgb(245, 194, 231) => Color::Indexed(13),
        Color::Rgb(249, 226, 175) => Color::Indexed(11),
        Color::Rgb(133, 133, 133) => Color::Indexed(7),
        Color::Rgb(172, 66, 66) | Color::Rgb(245, 150, 150) => Color::Indexed(9),
        Color::Rgb(192, 229, 140) | Color::Rgb(232, 232, 232) => Color::Indexed(15),
        Color::Rgb(229, 180, 84) => Color::Indexed(11),
        _ => Color::Reset,
    }
}

fn portable_color(color: Color) -> Color {
    match color {
        Color::Rgb(118, 185, 0) | Color::Rgb(145, 205, 102) => Color::Green,
        Color::Rgb(182, 227, 91) => Color::LightGreen,
        Color::Rgb(245, 194, 231) => Color::LightMagenta,
        Color::Rgb(249, 226, 175) => Color::LightYellow,
        Color::Rgb(192, 229, 140) | Color::Rgb(232, 232, 232) => Color::White,
        Color::Rgb(133, 133, 133)
        | Color::Rgb(52, 52, 52)
        | Color::Rgb(82, 82, 82)
        | Color::Rgb(107, 107, 107) => Color::DarkGray,
        Color::Rgb(172, 66, 66) | Color::Rgb(245, 150, 150) => Color::Red,
        Color::Rgb(229, 180, 84) => Color::Yellow,
        _ => Color::Reset,
    }
}

fn color_rgb(color: Color) -> Option<(u8, u8, u8)> {
    match color {
        Color::Rgb(red, green, blue) => Some((red, green, blue)),
        _ => None,
    }
}
