//! Unicode draft editing and atomic attachment spans.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{ops::Range, sync::Arc};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
pub const MAX_TEXT: usize = 1_048_576;
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AttachmentPayload {
    Text(String),
    Image {
        width: usize,
        height: usize,
        rgba: Vec<u8>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: u64,
    pub label: String,
    pub payload: AttachmentPayload,
}
impl Attachment {
    pub fn bytes(&self) -> usize {
        match &self.payload {
            AttachmentPayload::Text(s) => s.len(),
            AttachmentPayload::Image { rgba, .. } => rgba.len(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentSpan {
    pub range: Range<usize>,
    pub attachment: Arc<Attachment>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct State {
    text: String,
    cursor: usize,
    anchor: Option<usize>,
    spans: Vec<AttachmentSpan>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Composer {
    pub text: String,
    pub cursor: usize,
    pub anchor: Option<usize>,
    pub spans: Vec<AttachmentSpan>,
    bookmarks: std::collections::BTreeMap<u64, Range<usize>>,
    undo: Vec<State>,
    redo: Vec<State>,
    preferred_column: Option<usize>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutRow {
    pub start: usize,
    pub end: usize,
    pub cells: Vec<(usize, usize)>,
}
pub fn clean_input(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}
pub fn display_width(text: &str) -> usize {
    text.graphemes(true).fold(0, |col, g| {
        col + if g == "\t" { 4 - col % 4 } else { g.width() }
    })
}
pub fn layout_rows(text: &str, width: usize) -> Vec<LayoutRow> {
    let width = width.max(1);
    let mut rows = vec![LayoutRow {
        start: 0,
        end: 0,
        cells: vec![],
    }];
    let mut col = 0;
    for (i, g) in text.grapheme_indices(true) {
        if g == "\n" {
            rows.last_mut().unwrap().end = i;
            rows.push(LayoutRow {
                start: i + 1,
                end: i + 1,
                cells: vec![],
            });
            col = 0;
            continue;
        }
        let w = if g == "\t" { 4 - col % 4 } else { g.width() };
        if col + w > width && col > 0 {
            rows.last_mut().unwrap().end = i;
            rows.push(LayoutRow {
                start: i,
                end: i,
                cells: vec![],
            });
            col = 0;
        }
        rows.last_mut().unwrap().cells.push((col, i));
        col += w;
        rows.last_mut().unwrap().end = i + g.len();
    }
    if col >= width {
        rows.push(LayoutRow {
            start: text.len(),
            end: text.len(),
            cells: vec![],
        });
    }
    rows
}
impl Composer {
    fn state(&self) -> State {
        State {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: self.anchor,
            spans: self.spans.clone(),
        }
    }
    fn restore(&mut self, s: State) {
        self.text = s.text;
        self.cursor = s.cursor;
        self.anchor = s.anchor;
        self.spans = s.spans;
        self.preferred_column = None;
        self.bookmarks.clear();
    }
    fn checkpoint(&mut self) {
        self.undo.push(self.state());
        if self.undo.len() > 100 {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
    pub fn bookmark(&mut self, id: u64) {
        self.bookmarks
            .insert(id, self.selection().unwrap_or(self.cursor..self.cursor));
    }
    pub fn take_bookmark(&mut self, id: u64) -> Option<Range<usize>> {
        self.bookmarks.remove(&id)
    }
    pub fn undo(&mut self) {
        if let Some(s) = self.undo.pop() {
            self.redo.push(self.state());
            self.restore(s);
        }
    }
    pub fn redo(&mut self) {
        if let Some(s) = self.redo.pop() {
            self.undo.push(self.state());
            self.restore(s);
        }
    }
    pub fn set(&mut self, text: &str) {
        self.bookmarks.clear();
        self.text = clean_input(text);
        self.cursor = self.text.len();
        self.anchor = None;
        self.spans.clear();
        self.undo.clear();
        self.redo.clear();
        self.preferred_column = None;
    }
    pub fn snapshot(&self) -> Self {
        Self {
            text: self.text.clone(),
            cursor: self.cursor,
            anchor: None,
            spans: self.spans.clone(),
            ..Self::default()
        }
    }
    /// Submitted prompts keep interior whitespace but omit empty rows at the end.
    /// Work on a snapshot so rejected submissions never change the live editor.
    pub fn submission_snapshot(&self) -> Self {
        let mut draft = self.snapshot();
        let mut end = draft.text.len();
        while let Some(newline) = draft.text[..end].rfind('\n') {
            if !draft.text[newline + 1..end].trim().is_empty() {
                break;
            }
            end = newline;
        }
        draft.text.truncate(end);
        draft.cursor = draft.cursor.min(end);
        debug_assert!(draft.spans.iter().all(|span| span.range.end <= end));
        draft
    }
    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.bookmarks.clear();
        self.text.clear();
        self.cursor = 0;
        self.anchor = None;
        self.spans.clear();
        self.preferred_column = None;
    }
    pub fn selection(&self) -> Option<Range<usize>> {
        let a = self.anchor?;
        if a == self.cursor {
            None
        } else {
            Some(a.min(self.cursor)..a.max(self.cursor))
        }
    }
    pub fn attachment_at(&self, pos: usize) -> Option<Arc<Attachment>> {
        self.spans
            .iter()
            .find(|s| s.range.contains(&pos))
            .map(|s| s.attachment.clone())
    }
    pub fn selected_text(&self) -> String {
        self.selection()
            .map(|r| self.copy_range(r))
            .unwrap_or_default()
    }
    pub fn copy_range(&self, r: Range<usize>) -> String {
        let mut out = String::new();
        let mut pos = r.start;
        for s in self
            .spans
            .iter()
            .filter(|s| s.range.start >= r.start && s.range.end <= r.end)
        {
            out.push_str(&self.text[pos..s.range.start]);
            match &s.attachment.payload {
                AttachmentPayload::Text(t) => out.push_str(t),
                _ => out.push_str(&s.attachment.label),
            }
            pos = s.range.end;
        }
        out.push_str(&self.text[pos..r.end]);
        out
    }
    pub fn expanded_text(&self) -> String {
        self.copy_range(0..self.text.len())
    }
    pub fn replace(&mut self, mut range: Range<usize>, text: &str) -> Result<(), String> {
        let clean = clean_input(text);
        for s in &self.spans {
            if s.range.start < range.end && s.range.end > range.start {
                range.start = range.start.min(s.range.start);
                range.end = range.end.max(s.range.end);
            }
        }
        if self.text.len() - (range.end - range.start) + clean.len() > MAX_TEXT {
            return Err("Text exceeds the 1 MiB limit; nothing was inserted.".into());
        }
        if range.is_empty() && clean.is_empty() {
            return Ok(());
        }
        self.checkpoint();
        self.replace_raw(range, &clean);
        Ok(())
    }
    fn replace_raw(&mut self, range: Range<usize>, text: &str) {
        let delta = text.len() as isize - (range.end - range.start) as isize;
        self.spans
            .retain(|s| !(s.range.start < range.end && s.range.end > range.start));
        for s in &mut self.spans {
            if s.range.start >= range.end {
                s.range.start = (s.range.start as isize + delta) as usize;
                s.range.end = (s.range.end as isize + delta) as usize;
            }
        }
        for mark in self.bookmarks.values_mut() {
            if mark.start > range.end {
                mark.start = (mark.start as isize + delta) as usize;
                mark.end = (mark.end as isize + delta) as usize;
            } else if !(range.is_empty() && mark.start == range.start) && mark.end >= range.start {
                let p = mark.start.min(range.start);
                *mark = p..p;
            }
        }
        self.text.replace_range(range.clone(), text);
        self.cursor = range.start + text.len();
        self.anchor = None;
        self.preferred_column = None;
    }
    pub fn try_insert(&mut self, text: &str) -> Result<(), String> {
        self.replace(self.selection().unwrap_or(self.cursor..self.cursor), text)
    }
    pub fn insert(&mut self, text: &str) {
        let _ = self.try_insert(text);
    }
    pub fn insert_attachment(&mut self, attachment: Arc<Attachment>) -> Result<(), String> {
        let r = self.selection().unwrap_or(self.cursor..self.cursor);
        let start = r.start;
        self.replace(r, &attachment.label)?;
        self.spans.push(AttachmentSpan {
            range: start..start + attachment.label.len(),
            attachment,
        });
        self.spans.sort_by_key(|s| s.range.start);
        Ok(())
    }
    pub fn expand_attachment(&mut self, id: u64) -> Result<(), String> {
        let Some(s) = self.spans.iter().find(|s| s.attachment.id == id).cloned() else {
            return Ok(());
        };
        if let AttachmentPayload::Text(t) = &s.attachment.payload {
            self.replace(s.range, t)?;
        }
        Ok(())
    }
    pub fn snap(&self, pos: usize) -> usize {
        let mut p = pos.min(self.text.len());
        while !self.text.is_char_boundary(p) {
            p -= 1;
        }
        if let Some(s) = self
            .spans
            .iter()
            .find(|s| s.range.start < p && p < s.range.end)
        {
            if p - s.range.start < s.range.end - p {
                s.range.start
            } else {
                s.range.end
            }
        } else if p == self.text.len() {
            p
        } else {
            self.text
                .grapheme_indices(true)
                .map(|(i, _)| i)
                .take_while(|i| *i <= p)
                .last()
                .unwrap_or(0)
        }
    }
    fn left_pos(&self) -> usize {
        if let Some(s) = self.spans.iter().find(|s| s.range.end == self.cursor) {
            return s.range.start;
        }
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn right_pos(&self) -> usize {
        if let Some(s) = self.spans.iter().find(|s| s.range.start == self.cursor) {
            return s.range.end;
        }
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }
    pub fn left(&mut self) {
        self.cursor = self.left_pos();
        self.anchor = None;
    }
    pub fn right(&mut self) {
        self.cursor = self.right_pos();
        self.anchor = None;
    }
    pub fn backspace(&mut self) {
        let r = self.selection().unwrap_or(self.left_pos()..self.cursor);
        if !r.is_empty() {
            let _ = self.replace(r, "");
        }
    }
    pub fn delete(&mut self) {
        let r = self.selection().unwrap_or(self.cursor..self.right_pos());
        if !r.is_empty() {
            let _ = self.replace(r, "");
        }
    }
    fn word_left(&self) -> usize {
        let mut p = self.cursor;
        let mut seen = false;
        for (i, g) in self.text[..self.cursor].grapheme_indices(true).rev() {
            let space = g.chars().all(char::is_whitespace);
            if seen && space {
                break;
            }
            seen |= !space;
            p = i;
        }
        self.snap(p)
    }
    fn word_right(&self) -> usize {
        let mut p = self.cursor;
        let mut seen = false;
        for (i, g) in self.text[self.cursor..].grapheme_indices(true) {
            let space = g.chars().all(char::is_whitespace);
            if seen && space {
                break;
            }
            seen |= !space;
            p = self.cursor + i + g.len();
        }
        self.snap(p)
    }
    pub fn key(&mut self, key: KeyEvent) {
        let _ = self.key_at_width(key, 80);
    }
    pub fn key_at_width(&mut self, key: KeyEvent, width: usize) -> Result<(), String> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        if ctrl {
            match key.code {
                KeyCode::Char('a') => {
                    self.anchor = Some(0);
                    self.cursor = self.text.len();
                    return Ok(());
                }
                KeyCode::Char('z') if shift => {
                    self.redo();
                    return Ok(());
                }
                KeyCode::Char('z') => {
                    self.undo();
                    return Ok(());
                }
                KeyCode::Char('y') => {
                    self.redo();
                    return Ok(());
                }
                KeyCode::Backspace => {
                    let r = self.selection().unwrap_or(self.word_left()..self.cursor);
                    return self.replace(r, "");
                }
                KeyCode::Delete => {
                    let r = self.selection().unwrap_or(self.cursor..self.word_right());
                    return self.replace(r, "");
                }
                _ => {}
            }
        }
        let old = self.cursor;
        let target = match key.code {
            KeyCode::Left => Some(if ctrl {
                self.word_left()
            } else if !shift && self.selection().is_some() {
                self.selection().unwrap().start
            } else {
                self.left_pos()
            }),
            KeyCode::Right => Some(if ctrl {
                self.word_right()
            } else if !shift && self.selection().is_some() {
                self.selection().unwrap().end
            } else {
                self.right_pos()
            }),
            KeyCode::Home | KeyCode::End | KeyCode::Up | KeyCode::Down => {
                let rows = layout_rows(&self.text, width);
                let index = rows
                    .iter()
                    .rposition(|r| r.start <= self.cursor)
                    .unwrap_or(0);
                let row = &rows[index];
                let col = display_width(&self.text[row.start..self.cursor.min(row.end)]);
                let preferred = self.preferred_column.unwrap_or(col);
                match key.code {
                    KeyCode::Home => Some(if ctrl { 0 } else { row.start }),
                    KeyCode::End => Some(if ctrl { self.text.len() } else { row.end }),
                    _ => {
                        let next = if key.code == KeyCode::Up {
                            index.saturating_sub(1)
                        } else {
                            (index + 1).min(rows.len() - 1)
                        };
                        self.preferred_column = Some(preferred);
                        let r = &rows[next];
                        Some(if preferred >= display_width(&self.text[r.start..r.end]) {
                            r.end
                        } else {
                            r.cells
                                .iter()
                                .take_while(|(c, _)| *c <= preferred)
                                .last()
                                .map_or(r.start, |(_, p)| *p)
                        })
                    }
                }
            }
            _ => None,
        };
        if let Some(p) = target {
            if shift {
                self.anchor.get_or_insert(old);
            } else {
                self.anchor = None;
            }
            self.cursor = self.snap(p);
            if !matches!(key.code, KeyCode::Up | KeyCode::Down) {
                self.preferred_column = None;
            }
            return Ok(());
        }
        match key.code {
            KeyCode::Backspace => self.backspace(),
            KeyCode::Delete => self.delete(),
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.try_insert(&ch.to_string())?
            }
            _ => {}
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn graphemes_selection_undo() {
        let mut c = Composer::default();
        c.insert("a🤖e\u{301}");
        c.backspace();
        assert_eq!(c.text, "a🤖");
        c.backspace();
        assert_eq!(c.text, "a");
        c.undo();
        assert_eq!(c.text, "a🤖");
        c.redo();
        assert_eq!(c.text, "a");
        c.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
        c.insert("new");
        assert_eq!(c.text, "new");
    }
    #[test]
    fn atomic_attachment_copy_delete_undo() {
        let mut c = Composer::default();
        c.insert("before ");
        c.insert_attachment(Arc::new(Attachment {
            id: 1,
            label: "[Pasted Content 11 chars]".into(),
            payload: AttachmentPayload::Text("actual\ntext".into()),
        }))
        .unwrap();
        c.anchor = Some(0);
        assert_eq!(c.selected_text(), "before actual\ntext");
        c.anchor = None;
        c.backspace();
        assert_eq!(c.text, "before ");
        c.undo();
        assert_eq!(c.text, "before [Pasted Content 11 chars]");
        assert_eq!(c.spans.len(), 1);
        c.cursor = 0;
        c.insert("x");
        assert_eq!(c.spans[0].range.start, 8);
    }
    #[test]
    fn vertical_grapheme_mapping() {
        let mut c = Composer::default();
        c.set("ab界\nxyz");
        let rows = layout_rows(&c.text, 4);
        assert_eq!(rows[0].end, 5);
        c.key_at_width(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), 4)
            .unwrap();
        assert_eq!(c.cursor, 2);
    }
    #[test]
    fn limit_rejects_whole_insert() {
        let mut c = Composer::default();
        c.set("keep");
        assert!(c.try_insert(&"x".repeat(MAX_TEXT)).is_err());
        assert_eq!(c.text, "keep");
    }
}
