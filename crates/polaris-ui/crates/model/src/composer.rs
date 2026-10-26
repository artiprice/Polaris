//! Grapheme-aware composer with atomic paste chips and session-local prompt history.

use crate::error::ComposerError;
use core::ops::Range;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

/// Maximum expanded draft text, in bytes.
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Maximum bytes of one image's RGBA data.
pub const MAX_IMAGE_RGBA: usize = 64 << 20;
/// Maximum image bytes attached to a draft.
pub const MAX_ATTACHMENTS_TOTAL: usize = 128 << 20;
pub const MAX_UNDO_ENTRIES: usize = 256;
pub const MAX_HISTORY_ENTRIES: usize = 256;
pub const MAX_RETAINED_BYTES: usize = 8 << 20;

/// The payload hidden behind an atomic editor chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Normalised pasted text.
    Text {
        /// Unicode scalar count, used in the chip label.
        chars: usize,
        /// Text expanded on submission.
        payload: Arc<str>,
    },
    /// An image whose pixel storage is owned by the application.
    Image {
        /// Draft-local image label.
        index: u32,
        /// Number of RGBA bytes in the application's image storage.
        rgba_len: usize,
    },
}

/// Image metadata carried by a submission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageRef {
    /// Draft-local image label.
    pub index: u32,
    /// Image size in bytes.
    pub rgba_len: usize,
}

/// A borrowed chip for rendering and attachment previews.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chip<'a> {
    /// Stable identifier within this draft.
    pub id: u64,
    /// Display label's byte range.
    pub range: Range<usize>,
    /// Associated payload.
    pub attachment: &'a Attachment,
}

/// Expanded prompt and the application's referenced images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Submission {
    /// Text with paste chips expanded and image labels retained.
    pub text: String,
    /// Attached images, in display order.
    pub images: Vec<ImageRef>,
}

/// An immutable send candidate tied to the composer revision it was prepared from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedSubmission {
    revision: u64,
    submission: Submission,
}
impl PreparedSubmission {
    #[must_use]
    pub fn submission(&self) -> &Submission {
        &self.submission
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedChip {
    id: u64,
    range: Range<usize>,
    attachment: Attachment,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Draft {
    text: String,
    cursor: usize,
    selection_anchor: Option<usize>,
    chips: Vec<OwnedChip>,
    next_id: u64,
    next_image: u32,
}

/// Editable draft. Image bytes remain in the frontend; this type owns only references and budgets.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Composer {
    draft: Draft,
    revision: u64,
    undo: Vec<Draft>,
    redo: Vec<Draft>,
    history: Vec<String>,
    history_cursor: Option<usize>,
    saved_draft: Option<Draft>,
    group_space: bool,
}

fn normalise(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .collect()
}

fn trim_blank_rows(text: &str) -> String {
    let mut lines: Vec<&str> = text.split('\n').collect();
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        let _last = lines.pop();
    }
    lines.join("\n")
}

/// Return the complete grapheme containing a canonical source byte offset.
///
/// Offsets inside a grapheme snap to its start. Offsets at or beyond the source end select
/// the final grapheme; empty text has no selectable grapheme.
#[must_use]
pub fn source_grapheme_range(text: &str, at: usize) -> Option<Range<usize>> {
    let at = at.min(text.len());
    let mut last = None;
    for (start, grapheme) in text.grapheme_indices(true) {
        let end = start.saturating_add(grapheme.len());
        if start <= at && at < end {
            return Some(start..end);
        }
        last = Some(start..end);
    }
    if at == text.len() { last } else { None }
}

/// Return the Unicode word, whitespace run, or punctuation grapheme under a byte offset.
///
/// Alphanumeric and underscore graphemes form words, adjacent whitespace graphemes form one
/// selectable run, and punctuation selects one grapheme. Offsets inside a grapheme snap to it;
/// offsets at or beyond the source end select the final grapheme.
#[must_use]
pub fn source_word_range(text: &str, at: usize) -> Option<Range<usize>> {
    let spans: Vec<_> = text
        .grapheme_indices(true)
        .map(|(start, grapheme)| start..start.saturating_add(grapheme.len()))
        .collect();
    let at = at.min(text.len());
    let clicked = spans
        .iter()
        .position(|span| span.start <= at && at < span.end)
        .or_else(|| spans.len().checked_sub(1).filter(|_| at == text.len()))?;
    let selected = spans.get(clicked)?;
    let class = source_grapheme_class(text, selected)?;
    if class == 3 {
        return Some(selected.clone());
    }
    let mut start = selected.start;
    let mut end = selected.end;
    for span in spans.iter().take(clicked).rev() {
        if source_grapheme_class(text, span) == Some(class) {
            start = span.start;
        } else {
            break;
        }
    }
    for span in spans.iter().skip(clicked.saturating_add(1)) {
        if source_grapheme_class(text, span) == Some(class) {
            end = span.end;
        } else {
            break;
        }
    }
    Some(start..end)
}

/// Return the logical source line under a byte offset, including its existing LF or CRLF.
///
/// A byte offset on a line terminator belongs to the preceding line. An offset at EOF selects
/// the final line only when that line contains source bytes; empty text and the synthetic empty
/// line after a terminal newline have no range.
#[must_use]
pub fn source_line_range(text: &str, at: usize) -> Option<Range<usize>> {
    if text.is_empty() {
        return None;
    }
    let at = at.min(text.len());
    let mut start = 0;
    for (offset, ch) in text.char_indices() {
        if ch == '\n' {
            let end = offset.saturating_add(ch.len_utf8());
            if start <= at && at < end {
                return Some(start..end);
            }
            start = end;
        }
    }
    if start < text.len() && start <= at && at <= text.len() {
        Some(start..text.len())
    } else {
        None
    }
}

fn source_grapheme_class(text: &str, span: &Range<usize>) -> Option<u8> {
    let segment = text.get(span.clone())?;
    let first = segment.chars().next()?;
    Some(if first.is_alphanumeric() || first == '_' {
        1
    } else if first.is_whitespace() {
        2
    } else {
        3
    })
}

impl Composer {
    /// Start an empty session-local composer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Display text, including chip labels.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.draft.text
    }

    /// Cursor byte offset on a grapheme or chip boundary.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.draft.cursor
    }

    /// Current monotonic edit and cursor revision for validating asynchronous work.
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Whether there is no text or attachment.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.draft.text.is_empty()
    }

    /// Chips in display order.
    #[must_use]
    pub fn chips(&self) -> Vec<Chip<'_>> {
        self.draft
            .chips
            .iter()
            .map(|chip| Chip {
                id: chip.id,
                range: chip.range.clone(),
                attachment: &chip.attachment,
            })
            .collect()
    }

    /// Expanded text bytes, excluding image labels.
    #[must_use]
    pub fn logical_len(&self) -> usize {
        self.draft
            .chips
            .iter()
            .fold(self.draft.text.len(), |len, chip| {
                let payload_len = match &chip.attachment {
                    Attachment::Text { payload, .. } => payload.len(),
                    Attachment::Image { .. } => 0,
                };
                len.saturating_sub(chip.range.end.saturating_sub(chip.range.start))
                    .saturating_add(payload_len)
            })
    }

    /// Total image bytes referenced by the draft.
    #[must_use]
    pub fn image_bytes_total(&self) -> usize {
        self.draft
            .chips
            .iter()
            .fold(0usize, |total, chip| match chip.attachment {
                Attachment::Image { rgba_len, .. } => total.saturating_add(rgba_len),
                Attachment::Text { .. } => total,
            })
    }

    fn remember(&mut self, merge_space: bool) {
        if !merge_space {
            self.undo.push(self.draft.clone());
        }
        self.redo.clear();
        self.trim_undo();
        self.revision = self.revision.saturating_add(1);
    }
    fn trim_undo(&mut self) {
        while self.undo.len().saturating_add(self.redo.len()) > MAX_UNDO_ENTRIES
            || self.undo_bytes() > MAX_RETAINED_BYTES
        {
            if !self.undo.is_empty() {
                self.undo.remove(0);
            } else if !self.redo.is_empty() {
                self.redo.remove(0);
            } else {
                break;
            }
        }
    }
    fn undo_bytes(&self) -> usize {
        self.undo
            .iter()
            .chain(self.redo.iter())
            .fold(0usize, |total, draft| {
                let chips = draft.chips.iter().fold(0usize, |sum, chip| {
                    sum.saturating_add(core::mem::size_of::<OwnedChip>())
                        .saturating_add(match &chip.attachment {
                            Attachment::Text { payload, .. } => payload.len(),
                            Attachment::Image { .. } => 0,
                        })
                });
                total
                    .saturating_add(core::mem::size_of::<Draft>())
                    .saturating_add(draft.text.len())
                    .saturating_add(chips)
            })
    }

    fn validate_range(&self, range: &Range<usize>) -> Result<(), ComposerError> {
        if self.draft.text.get(range.clone()).is_none()
            || self.draft.chips.iter().any(|chip| {
                (range.start > chip.range.start && range.start < chip.range.end)
                    || (range.end > chip.range.start && range.end < chip.range.end)
            })
        {
            Err(ComposerError::InvalidRange)
        } else {
            Ok(())
        }
    }

    fn replace(&mut self, range: Range<usize>, insert: &str) {
        let removed = range.end.saturating_sub(range.start);
        self.draft.chips.retain_mut(|chip| {
            if chip.range.end <= range.start {
                return true;
            }
            if chip.range.start >= range.end {
                chip.range.start = chip
                    .range
                    .start
                    .saturating_sub(removed)
                    .saturating_add(insert.len());
                chip.range.end = chip
                    .range
                    .end
                    .saturating_sub(removed)
                    .saturating_add(insert.len());
                return true;
            }
            false
        });
        self.draft.text.replace_range(range.clone(), insert);
        self.draft.cursor = range.start.saturating_add(insert.len());
        self.draft.selection_anchor = None;
        self.snap_cursor();
    }

    fn check_size(&self, attempted: usize) -> Result<(), ComposerError> {
        if attempted > MAX_TEXT_BYTES {
            Err(ComposerError::TextTooLarge {
                limit: MAX_TEXT_BYTES,
                attempted,
            })
        } else {
            Ok(())
        }
    }

    /// Insert normalised text; long pastes become chips. Failure leaves the draft unchanged.
    ///
    /// # Errors
    /// Returns `TextTooLarge` if the expanded text exceeds the byte budget.
    pub fn insert(&mut self, text: &str) -> Result<(), ComposerError> {
        let payload = normalise(text);
        if payload.is_empty() {
            return Ok(());
        }
        let selected = self.selection();
        let removed = match selected.clone() {
            Some(range) => self.expanded(range, false).len(),
            None => 0,
        };
        self.check_size(
            self.logical_len()
                .saturating_sub(removed)
                .saturating_add(payload.len()),
        )?;
        let chars = payload.chars().count();
        let attachment = if payload.lines().count() > 5 || chars > 500 {
            Some(Attachment::Text {
                chars,
                payload: Arc::<str>::from(payload.clone()),
            })
        } else {
            None
        };
        let label = if attachment.is_some() {
            format!("[Pasted Content {chars} chars]")
        } else {
            payload.clone()
        };
        let merge_space = self.group_space
            && selected.is_none()
            && attachment.is_none()
            && payload.chars().all(|c| c == ' ' || c == '\t');
        self.remember(merge_space);
        let range = match selected {
            Some(range) => range,
            None => self.draft.cursor..self.draft.cursor,
        };
        let start = range.start;
        self.replace(range, &label);
        if let Some(attachment) = attachment {
            self.push_chip(start, label.len(), attachment);
            self.group_space = false;
        } else {
            self.group_space = !payload.chars().all(char::is_whitespace);
        }
        Ok(())
    }

    fn push_chip(&mut self, start: usize, len: usize, attachment: Attachment) {
        self.draft.next_id = self.draft.next_id.saturating_add(1);
        self.draft.chips.push(OwnedChip {
            id: self.draft.next_id,
            range: start..start.saturating_add(len),
            attachment,
        });
        self.draft.chips.sort_by_key(|chip| chip.range.start);
        // A following combining mark can merge with the label's final bracket. Chip boundaries
        // take priority over grapheme boundaries, so restore the intended insertion endpoint.
        self.draft.cursor = start.saturating_add(len);
    }

    /// Insert a literal line break.
    ///
    /// # Errors
    /// Returns `TextTooLarge` if the expanded draft is at its cap.
    pub fn newline(&mut self) -> Result<(), ComposerError> {
        self.insert("\n")
    }

    /// Attach a frontend-owned image by byte size.
    ///
    /// # Errors
    /// Rejects empty images and individual or cumulative byte-budget overflow.
    pub fn add_image(&mut self, rgba_len: usize) -> Result<u32, ComposerError> {
        if rgba_len == 0 {
            return Err(ComposerError::ImageEmpty);
        }
        if rgba_len > MAX_IMAGE_RGBA {
            return Err(ComposerError::ImageTooLarge {
                len: rgba_len,
                limit: MAX_IMAGE_RGBA,
            });
        }
        let total = self.image_bytes_total().saturating_add(rgba_len);
        if total > MAX_ATTACHMENTS_TOTAL {
            return Err(ComposerError::AttachmentsTooLarge {
                total,
                limit: MAX_ATTACHMENTS_TOTAL,
            });
        }
        self.remember(false);
        self.draft.next_image = self.draft.next_image.saturating_add(1);
        let index = self.draft.next_image;
        let label = format!("[Image {index}]");
        let start = self.draft.cursor;
        self.replace(start..start, &label);
        self.push_chip(start, label.len(), Attachment::Image { index, rgba_len });
        self.group_space = false;
        Ok(index)
    }

    fn boundaries(&self) -> Vec<usize> {
        let mut positions: Vec<_> = self
            .draft
            .text
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .filter(|at| {
                !self
                    .draft
                    .chips
                    .iter()
                    .any(|chip| *at > chip.range.start && *at < chip.range.end)
            })
            .collect();
        positions.extend(
            self.draft
                .chips
                .iter()
                .flat_map(|chip| [chip.range.start, chip.range.end]),
        );
        positions.push(self.draft.text.len());
        positions.sort_unstable();
        positions.dedup();
        positions
    }

    fn snap_cursor(&mut self) {
        self.draft.cursor = match self
            .boundaries()
            .into_iter()
            .take_while(|at| *at <= self.draft.cursor)
            .last()
        {
            Some(at) => at,
            None => 0,
        };
    }

    fn snap_position(&self, at: usize) -> usize {
        match self
            .boundaries()
            .into_iter()
            .take_while(|boundary| *boundary <= at.min(self.draft.text.len()))
            .last()
        {
            Some(boundary) => boundary,
            None => 0,
        }
    }

    fn move_to(&mut self, at: usize, extend: bool) {
        let before = self.draft.cursor;
        let target = self.snap_position(at);
        if extend {
            if self.draft.selection_anchor.is_none() {
                self.draft.selection_anchor = Some(self.draft.cursor);
            }
            self.draft.cursor = target;
        } else {
            self.draft.cursor = target;
            self.draft.selection_anchor = None;
        }
        if before != self.draft.cursor {
            self.revision = self.revision.saturating_add(1);
        }
        self.group_space = false;
    }

    /// Current selected byte range, if its endpoints differ.
    #[must_use]
    pub fn selection(&self) -> Option<Range<usize>> {
        let anchor = self.draft.selection_anchor?;
        if anchor == self.draft.cursor {
            None
        } else if anchor < self.draft.cursor {
            Some(anchor..self.draft.cursor)
        } else {
            Some(self.draft.cursor..anchor)
        }
    }

    /// Selected text with text-chip payloads expanded and image labels retained.
    #[must_use]
    pub fn selected_text(&self) -> Option<String> {
        self.selection().map(|range| self.expanded(range, true))
    }

    /// Anchor endpoint used by shift navigation, if a selection gesture has started.
    #[must_use]
    pub const fn selection_anchor(&self) -> Option<usize> {
        self.draft.selection_anchor
    }

    /// Select a complete range, snapping both endpoints to legal grapheme/chip boundaries.
    pub fn set_selection(&mut self, range: Range<usize>) {
        let start = self.snap_position(range.start);
        let end = self.snap_position(range.end);
        self.draft.selection_anchor = Some(start);
        self.draft.cursor = end;
        self.revision = self.revision.saturating_add(1);
        self.group_space = false;
    }

    /// Select the Unicode word or atomic chip at a byte position.
    ///
    /// Whitespace double-clicks select the contiguous whitespace run; punctuation
    /// selects one grapheme. A position inside a chip selects the whole chip.
    pub fn select_word_at(&mut self, at: usize) -> bool {
        let at = at.min(self.draft.text.len());
        if let Some(chip) = self
            .draft
            .chips
            .iter()
            .find(|chip| at >= chip.range.start && at <= chip.range.end)
        {
            self.set_selection(chip.range.clone());
            return true;
        }

        let boundaries = self.boundaries();
        let spans: Vec<_> = boundaries
            .windows(2)
            .filter_map(|pair| Some(*pair.first()?..*pair.get(1)?))
            .collect();
        let clicked = spans
            .iter()
            .position(|span| at >= span.start && at < span.end)
            .or_else(|| {
                spans
                    .len()
                    .checked_sub(1)
                    .filter(|_| at == self.draft.text.len())
            });
        let Some(clicked) = clicked else {
            return false;
        };
        let Some(selected) = spans.get(clicked) else {
            return false;
        };
        let Some(segment) = self.draft.text.get(selected.clone()) else {
            return false;
        };
        let Some(first) = segment.chars().next() else {
            return false;
        };
        let class = if first.is_alphanumeric() || first == '_' {
            1_u8
        } else if first.is_whitespace() {
            2_u8
        } else {
            3_u8
        };
        let mut start = selected.start;
        let mut end = selected.end;
        if class < 3 {
            for span in spans.iter().take(clicked).rev() {
                if self.segment_class(span) == Some(class) {
                    start = span.start;
                } else {
                    break;
                }
            }
            for span in spans.iter().skip(clicked.saturating_add(1)) {
                if self.segment_class(span) == Some(class) {
                    end = span.end;
                } else {
                    break;
                }
            }
        }
        self.set_selection(start..end);
        true
    }

    /// Select the complete draft.
    pub fn select_all(&mut self) {
        self.set_selection(0..self.draft.text.len());
    }

    /// Remove the active selection and return its expanded text.
    pub fn cut(&mut self) -> Option<String> {
        let range = self.selection()?;
        let copied = self.expanded(range.clone(), true);
        self.remember(false);
        self.replace(range, "");
        self.group_space = false;
        Some(copied)
    }

    /// Set the cursor, snapping backward to a grapheme/chip boundary and clamping to the draft.
    pub fn set_cursor(&mut self, at: usize) {
        self.move_to(at, false);
    }
    /// Move over one grapheme or chip to the left.
    pub fn move_left(&mut self) {
        if let Some(range) = self.selection() {
            self.move_to(range.start, false);
            return;
        }
        self.move_left_with_selection(false);
    }
    /// Move left by one grapheme or chip, optionally extending the selection.
    pub fn move_left_with_selection(&mut self, extend: bool) {
        let before = self.draft.cursor;
        if let Some(at) = self
            .boundaries()
            .into_iter()
            .take_while(|at| *at < self.draft.cursor)
            .last()
        {
            self.draft.cursor = at;
        }
        if extend {
            if self.draft.selection_anchor.is_none() {
                self.draft.selection_anchor = Some(before);
            }
        } else {
            self.draft.selection_anchor = None;
        }
        if before != self.draft.cursor {
            self.revision = self.revision.saturating_add(1);
        }
        self.group_space = false;
    }
    /// Move over one grapheme or chip to the right.
    pub fn move_right(&mut self) {
        if let Some(range) = self.selection() {
            self.move_to(range.end, false);
            return;
        }
        self.move_right_with_selection(false);
    }
    /// Move right by one grapheme or chip, optionally extending the selection.
    pub fn move_right_with_selection(&mut self, extend: bool) {
        let before = self.draft.cursor;
        if let Some(at) = self
            .boundaries()
            .into_iter()
            .find(|at| *at > self.draft.cursor)
        {
            self.draft.cursor = at;
        }
        if extend {
            if self.draft.selection_anchor.is_none() {
                self.draft.selection_anchor = Some(before);
            }
        } else {
            self.draft.selection_anchor = None;
        }
        if before != self.draft.cursor {
            self.revision = self.revision.saturating_add(1);
        }
        self.group_space = false;
    }
    /// Move to the start of the draft.
    pub fn move_home(&mut self) {
        self.set_cursor(0);
    }
    /// Move to the end of the draft.
    pub fn move_end(&mut self) {
        self.set_cursor(self.draft.text.len());
    }

    /// Move to the current logical line start.
    pub fn move_line_home(&mut self) {
        self.move_line_home_with_selection(false);
    }
    /// Move to the current logical line start, optionally extending the selection.
    pub fn move_line_home_with_selection(&mut self, extend: bool) {
        let target = match self
            .draft
            .text
            .get(..self.draft.cursor)
            .and_then(|before| before.rfind('\n'))
        {
            Some(newline) => newline.saturating_add(1),
            None => 0,
        };
        self.move_to(target, extend);
    }
    /// Move to the current logical line end.
    pub fn move_line_end(&mut self) {
        self.move_line_end_with_selection(false);
    }
    /// Move to the current logical line end, optionally extending the selection.
    pub fn move_line_end_with_selection(&mut self, extend: bool) {
        let target = match self
            .draft
            .text
            .get(self.draft.cursor..)
            .and_then(|after| after.find('\n'))
        {
            Some(offset) => self.draft.cursor.saturating_add(offset),
            None => self.draft.text.len(),
        };
        self.move_to(target, extend);
    }
    /// Move to the beginning of the complete document.
    pub fn move_document_home(&mut self) {
        self.move_to(0, false);
    }
    /// Move to the end of the complete document.
    pub fn move_document_end(&mut self) {
        self.move_to(self.draft.text.len(), false);
    }
    /// Move to document start and extend the selection.
    pub fn move_document_home_with_selection(&mut self) {
        self.move_to(0, true);
    }
    /// Move to document end and extend the selection.
    pub fn move_document_end_with_selection(&mut self) {
        self.move_to(self.draft.text.len(), true);
    }

    fn word_target(&self, left: bool) -> usize {
        let boundaries = self.boundaries();
        let mut position = self.draft.cursor;
        let origin = position;
        let mut saw_word = false;
        let mut saw_whitespace = false;
        if left {
            for at in boundaries.into_iter().rev().filter(|at| *at < origin) {
                let whitespace = self
                    .draft
                    .text
                    .get(at..position)
                    .is_some_and(|part| part.chars().all(char::is_whitespace));
                if whitespace {
                    if saw_word {
                        break;
                    }
                } else {
                    saw_word = true;
                }
                position = at;
            }
        } else {
            for at in boundaries.into_iter().filter(|at| *at > origin) {
                let whitespace = self
                    .draft
                    .text
                    .get(position..at)
                    .is_some_and(|part| part.chars().all(char::is_whitespace));
                if whitespace {
                    saw_whitespace = true;
                } else if saw_whitespace {
                    break;
                }
                position = at;
            }
        }
        position
    }

    fn segment_class(&self, span: &Range<usize>) -> Option<u8> {
        let segment = self.draft.text.get(span.clone())?;
        let first = segment.chars().next()?;
        Some(if first.is_alphanumeric() || first == '_' {
            1
        } else if first.is_whitespace() {
            2
        } else {
            3
        })
    }

    /// Move left across preceding whitespace and the preceding word.
    pub fn move_word_left(&mut self) {
        self.move_word_left_with_selection(false);
    }
    /// Move left across a word, optionally extending the selection.
    pub fn move_word_left_with_selection(&mut self, extend: bool) {
        let target = self.word_target(true);
        self.move_to(target, extend);
    }
    /// Move right across the current word and its following whitespace.
    pub fn move_word_right(&mut self) {
        self.move_word_right_with_selection(false);
    }
    /// Move right across a word, optionally extending the selection.
    pub fn move_word_right_with_selection(&mut self, extend: bool) {
        let target = self.word_target(false);
        self.move_to(target, extend);
    }

    /// Delete one preceding grapheme or chip; return whether anything changed.
    pub fn backspace(&mut self) -> bool {
        if let Some(range) = self.selection() {
            self.remember(false);
            self.replace(range, "");
            self.group_space = false;
            return true;
        }
        let end = self.draft.cursor;
        let Some(start) = self
            .boundaries()
            .into_iter()
            .take_while(|at| *at < end)
            .last()
        else {
            return false;
        };
        self.remember(false);
        self.replace(start..end, "");
        self.group_space = false;
        true
    }
    /// Delete one following grapheme or chip; return whether anything changed.
    pub fn delete(&mut self) -> bool {
        if let Some(range) = self.selection() {
            self.remember(false);
            self.replace(range, "");
            self.group_space = false;
            return true;
        }
        let start = self.draft.cursor;
        let Some(end) = self.boundaries().into_iter().find(|at| *at > start) else {
            return false;
        };
        self.remember(false);
        self.replace(start..end, "");
        self.group_space = false;
        true
    }

    /// Delete backward through preceding whitespace and the previous word.
    pub fn delete_word_backward(&mut self) -> bool {
        if self.selection().is_some() {
            return self.backspace();
        }
        let start = self.word_target(true);
        if start == self.draft.cursor {
            return false;
        }
        self.remember(false);
        self.replace(start..self.draft.cursor, "");
        self.group_space = false;
        true
    }

    /// Delete forward through the current word and following whitespace.
    pub fn delete_word_forward(&mut self) -> bool {
        if self.selection().is_some() {
            return self.delete();
        }
        let end = self.word_target(false);
        if end == self.draft.cursor {
            return false;
        }
        self.remember(false);
        self.replace(self.draft.cursor..end, "");
        self.group_space = false;
        true
    }
    /// Restore the previous edit, including its cursor and chip payloads.
    pub fn undo(&mut self) -> bool {
        let Some(draft) = self.undo.pop() else {
            return false;
        };
        self.redo.push(core::mem::replace(&mut self.draft, draft));
        self.trim_undo();
        self.revision = self.revision.saturating_add(1);
        self.group_space = false;
        true
    }
    /// Reapply the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(draft) = self.redo.pop() else {
            return false;
        };
        self.undo.push(core::mem::replace(&mut self.draft, draft));
        self.trim_undo();
        self.revision = self.revision.saturating_add(1);
        self.group_space = false;
        true
    }

    /// Replace a complete text/chip range with literal, normalised text.
    ///
    /// # Errors
    /// Rejects invalid UTF-8 offsets, partial chips and expanded text exceeding the byte budget.
    pub fn apply_completion(
        &mut self,
        range: Range<usize>,
        insert: &str,
    ) -> Result<(), ComposerError> {
        self.validate_range(&range)?;
        let insert = normalise(insert);
        let removed = self.expanded(range.clone(), false);
        self.check_size(
            self.logical_len()
                .saturating_sub(removed.len())
                .saturating_add(insert.len()),
        )?;
        self.remember(false);
        self.replace(range, &insert);
        self.group_space = false;
        Ok(())
    }

    /// Expand a text chip into literal editable text.
    ///
    /// # Errors
    /// Returns `NoSuchChip` if the id is absent or names an image.
    pub fn expand_chip(&mut self, id: u64) -> Result<(), ComposerError> {
        let Some((range, payload)) = self.draft.chips.iter().find_map(|chip| {
            if chip.id != id {
                return None;
            }
            match &chip.attachment {
                Attachment::Text { payload, .. } => Some((chip.range.clone(), payload.to_string())),
                Attachment::Image { .. } => None,
            }
        }) else {
            return Err(ComposerError::NoSuchChip);
        };
        self.apply_completion(range, &payload)
    }

    fn expanded(&self, range: Range<usize>, images: bool) -> String {
        let mut text = String::new();
        let mut offset = range.start;
        for chip in self
            .draft
            .chips
            .iter()
            .filter(|chip| chip.range.start >= range.start && chip.range.end <= range.end)
        {
            if let Some(part) = self.draft.text.get(offset..chip.range.start) {
                text.push_str(part);
            }
            match &chip.attachment {
                Attachment::Text { payload, .. } => text.push_str(payload),
                Attachment::Image { .. } if images => {
                    if let Some(label) = self.draft.text.get(chip.range.clone()) {
                        text.push_str(label);
                    }
                }
                Attachment::Image { .. } => {}
            }
            offset = chip.range.end;
        }
        if let Some(part) = self.draft.text.get(offset..range.end) {
            text.push_str(part);
        }
        text
    }

    /// Submit an expanded prompt, record text-only history, then clear draft and undo state.
    ///
    /// # Errors
    /// Returns `EmptySubmission` without mutation for blank drafts with no images.
    pub fn prepare_submit(&self) -> Result<PreparedSubmission, ComposerError> {
        let text = trim_blank_rows(&self.expanded(0..self.draft.text.len(), true));
        let images: Vec<_> = self
            .draft
            .chips
            .iter()
            .filter_map(|chip| match chip.attachment {
                Attachment::Image { index, rgba_len } => Some(ImageRef { index, rgba_len }),
                Attachment::Text { .. } => None,
            })
            .collect();
        if text.trim().is_empty() && images.is_empty() {
            return Err(ComposerError::EmptySubmission);
        }
        Ok(PreparedSubmission {
            revision: self.revision,
            submission: Submission { text, images },
        })
    }

    /// Commit only when no intervening edit or cursor move changed the composer revision.
    pub fn commit_submit(
        &mut self,
        prepared: PreparedSubmission,
    ) -> Result<Submission, ComposerError> {
        if prepared.revision != self.revision {
            return Err(ComposerError::StalePreparedSubmission);
        }
        let history_text = self
            .expanded(0..self.draft.text.len(), false)
            .trim_end()
            .to_owned();
        if !history_text.trim().is_empty() && self.history.last() != Some(&history_text) {
            self.history.push(history_text);
            while self.history.len() > MAX_HISTORY_ENTRIES
                || self.history_bytes() > MAX_RETAINED_BYTES
            {
                if self.history.is_empty() {
                    break;
                }
                self.history.remove(0);
            }
        }
        self.draft = Draft::default();
        self.undo.clear();
        self.redo.clear();
        self.history_cursor = None;
        self.saved_draft = None;
        self.group_space = false;
        self.revision = self.revision.saturating_add(1);
        Ok(prepared.submission)
    }

    /// Discard a prepared send without changing the composer.
    pub fn cancel_submit(&mut self, _prepared: PreparedSubmission) {}

    /// Compatibility operation for callers that have already accepted responsibility for sending.
    pub fn submit(&mut self) -> Result<Submission, ComposerError> {
        let prepared = self.prepare_submit()?;
        self.commit_submit(prepared)
    }

    fn history_bytes(&self) -> usize {
        self.history.iter().fold(0usize, |total, item| {
            total
                .saturating_add(core::mem::size_of::<String>())
                .saturating_add(item.len())
        })
    }

    /// Number of undo snapshots retained.
    #[must_use]
    pub fn undo_len(&self) -> usize {
        self.undo.len()
    }
    #[must_use]
    pub fn undo_retained_bytes(&self) -> usize {
        self.undo_bytes()
    }
    #[must_use]
    pub fn history_retained_bytes(&self) -> usize {
        self.history_bytes()
    }

    /// Number of distinct consecutive text prompts stored in this session.
    #[must_use]
    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    fn load_history(&mut self, at: usize) -> bool {
        let Some(text) = self.history.get(at) else {
            return false;
        };
        self.draft = Draft {
            text: text.clone(),
            cursor: text.len(),
            ..Draft::default()
        };
        self.history_cursor = Some(at);
        self.revision = self.revision.saturating_add(1);
        self.undo.clear();
        self.redo.clear();
        self.group_space = false;
        true
    }

    /// Recall an older prompt only when the frontend reports the first visual row.
    pub fn history_prev(&mut self, first_row: bool) -> bool {
        if !first_row || self.history.is_empty() {
            return false;
        }
        let at = match self.history_cursor {
            Some(0) => return false,
            Some(at) => at.saturating_sub(1),
            None => {
                self.saved_draft = Some(self.draft.clone());
                self.history.len().saturating_sub(1)
            }
        };
        self.load_history(at)
    }

    /// Recall a newer prompt, then restore the unsent draft and cursor after the newest prompt.
    pub fn history_next(&mut self, last_row: bool) -> bool {
        if !last_row {
            return false;
        }
        let Some(at) = self.history_cursor else {
            return false;
        };
        let next = at.saturating_add(1);
        if next < self.history.len() {
            return self.load_history(next);
        }
        let Some(draft) = self.saved_draft.take() else {
            return false;
        };
        self.draft = draft;
        self.history_cursor = None;
        self.revision = self.revision.saturating_add(1);
        self.undo.clear();
        self.redo.clear();
        self.group_space = false;
        true
    }
}
