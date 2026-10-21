//! Grapheme-aware composer with atomic paste chips and session-local prompt history.

use crate::error::ComposerError;
use core::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

/// Maximum expanded draft text, in bytes.
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Maximum bytes of one image's RGBA data.
pub const MAX_IMAGE_RGBA: usize = 64 << 20;
/// Maximum image bytes attached to a draft.
pub const MAX_ATTACHMENTS_TOTAL: usize = 128 << 20;

/// The payload hidden behind an atomic editor chip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attachment {
    /// Normalised pasted text.
    Text {
        /// Unicode scalar count, used in the chip label.
        chars: usize,
        /// Text expanded on submission.
        payload: String,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnedChip {
    id: u64,
    range: Range<usize>,
    attachment: Attachment,
}

#[derive(Debug, Clone, Default)]
struct Draft {
    text: String,
    cursor: usize,
    chips: Vec<OwnedChip>,
    next_id: u64,
    next_image: u32,
}

/// Editable draft. Image bytes remain in the frontend; this type owns only references and budgets.
#[derive(Debug, Clone, Default)]
pub struct Composer {
    draft: Draft,
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
        self.check_size(self.logical_len().saturating_add(payload.len()))?;
        let chars = payload.chars().count();
        let attachment = if payload.lines().count() > 5 || chars > 500 {
            Some(Attachment::Text {
                chars,
                payload: payload.clone(),
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
            && attachment.is_none()
            && payload.chars().all(|c| c == ' ' || c == '\t');
        self.remember(merge_space);
        let start = self.draft.cursor;
        self.replace(start..start, &label);
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

    /// Set the cursor, snapping backward to a grapheme/chip boundary and clamping to the draft.
    pub fn set_cursor(&mut self, at: usize) {
        self.draft.cursor = at.min(self.draft.text.len());
        self.snap_cursor();
        self.group_space = false;
    }
    /// Move over one grapheme or chip to the left.
    pub fn move_left(&mut self) {
        if let Some(at) = self
            .boundaries()
            .into_iter()
            .take_while(|at| *at < self.draft.cursor)
            .last()
        {
            self.draft.cursor = at;
        }
        self.group_space = false;
    }
    /// Move over one grapheme or chip to the right.
    pub fn move_right(&mut self) {
        if let Some(at) = self
            .boundaries()
            .into_iter()
            .find(|at| *at > self.draft.cursor)
        {
            self.draft.cursor = at;
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

    /// Delete one preceding grapheme or chip; return whether anything changed.
    pub fn backspace(&mut self) -> bool {
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
        let start = self.draft.cursor;
        let Some(end) = self.boundaries().into_iter().find(|at| *at > start) else {
            return false;
        };
        self.remember(false);
        self.replace(start..end, "");
        self.group_space = false;
        true
    }
    /// Restore the previous edit, including its cursor and chip payloads.
    pub fn undo(&mut self) -> bool {
        let Some(draft) = self.undo.pop() else {
            return false;
        };
        self.redo.push(core::mem::replace(&mut self.draft, draft));
        self.group_space = false;
        true
    }
    /// Reapply the last undone edit.
    pub fn redo(&mut self) -> bool {
        let Some(draft) = self.redo.pop() else {
            return false;
        };
        self.undo.push(core::mem::replace(&mut self.draft, draft));
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
                Attachment::Text { payload, .. } => Some((chip.range.clone(), payload.clone())),
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
    pub fn submit(&mut self) -> Result<Submission, ComposerError> {
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
        let history_text = self
            .expanded(0..self.draft.text.len(), false)
            .trim_end()
            .to_owned();
        if !history_text.trim().is_empty() && self.history.last() != Some(&history_text) {
            self.history.push(history_text);
        }
        self.draft = Draft::default();
        self.undo.clear();
        self.redo.clear();
        self.history_cursor = None;
        self.saved_draft = None;
        self.group_space = false;
        Ok(Submission { text, images })
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
        self.undo.clear();
        self.redo.clear();
        self.group_space = false;
        true
    }
}
