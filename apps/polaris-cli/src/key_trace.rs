//! Bounded category-only diagnostics. Printable keys and paste content never reach the file.
use crossterm::event::{KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind};
use polaris_ui_cli::input::NormalizedInput;
use std::{
    fs::{File, OpenOptions},
    io::{self, Write},
    path::Path,
};
/// Newly created diagnostic file, with no overwrite and a finite byte budget.
#[derive(Debug)]
pub struct KeyTrace {
    file: File,
    bytes: usize,
}
impl KeyTrace {
    /// Atomically create a new trace; an existing file or symlink is rejected.
    /// # Errors
    /// Existing target, permission, or other filesystem failure.
    pub fn create(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        Ok(Self {
            file: options.open(path)?,
            bytes: 0,
        })
    }
    /// Record one category without text, printable key values, paths, or raw bytes.
    /// # Errors
    /// Trace byte cap or write failure; caller must report and stop using the trace.
    pub fn record_input(&mut self, event: &NormalizedInput) -> io::Result<()> {
        let category = match event {
            NormalizedInput::Key(key) => match key.code {
                KeyCode::Char(_) => "key text",
                KeyCode::Enter => "key enter",
                KeyCode::Esc => "key escape",
                KeyCode::Backspace => "key backspace",
                KeyCode::Delete => "key delete",
                KeyCode::Up
                | KeyCode::Down
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End
                | KeyCode::PageUp
                | KeyCode::PageDown => "key navigation",
                _ => "key control",
            },
            NormalizedInput::Paste(_) => "paste",
            NormalizedInput::Mouse(_) => "mouse",
            NormalizedInput::Resize { .. } => "resize",
            NormalizedInput::Focus { .. } => "focus",
        };
        let mut record = String::from(category);
        if let NormalizedInput::Mouse(mouse) = event {
            let (kind, button) = match mouse.kind {
                MouseEventKind::Down(button) => ("down", Some(button)),
                MouseEventKind::Up(button) => ("up", Some(button)),
                MouseEventKind::Drag(button) => ("drag", Some(button)),
                MouseEventKind::Moved => ("move", None),
                MouseEventKind::ScrollUp => ("scroll-up", None),
                MouseEventKind::ScrollDown => ("scroll-down", None),
                MouseEventKind::ScrollLeft => ("scroll-left", None),
                MouseEventKind::ScrollRight => ("scroll-right", None),
            };
            record.push(' ');
            record.push_str(kind);
            record.push(' ');
            record.push_str(match button {
                Some(MouseButton::Left) => "left",
                Some(MouseButton::Middle) => "middle",
                Some(MouseButton::Right) => "right",
                None => "none",
            });
            record.push(' ');
            record.push_str(&mouse.column.to_string());
            record.push(' ');
            record.push_str(&mouse.row.to_string());
        }
        let modifiers = match event {
            NormalizedInput::Key(key) => Some(key.modifiers),
            NormalizedInput::Mouse(mouse) => Some(mouse.modifiers),
            _ => None,
        };
        if let Some(modifiers) = modifiers {
            record.push(' ');
            let mut any = false;
            for (flag, label) in [
                (KeyModifiers::SHIFT, "shift"),
                (KeyModifiers::CONTROL, "control"),
                (KeyModifiers::ALT, "alt"),
                (KeyModifiers::SUPER, "super"),
                (KeyModifiers::HYPER, "hyper"),
                (KeyModifiers::META, "meta"),
            ] {
                if modifiers.contains(flag) {
                    if any {
                        record.push('+');
                    }
                    record.push_str(label);
                    any = true;
                }
            }
            if !any {
                record.push_str("none");
            }
        }
        if let NormalizedInput::Key(key) = event {
            record.push(' ');
            record.push_str(match key.kind {
                KeyEventKind::Press => "press",
                KeyEventKind::Repeat => "repeat",
                KeyEventKind::Release => "release",
            });
        }
        let attempted = self
            .bytes
            .checked_add(record.len())
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| io::Error::other("trace size overflow"))?;
        if attempted > 1_048_576 {
            return Err(io::Error::other("key trace reached its 1 MiB limit"));
        }
        self.file.write_all(record.as_bytes())?;
        self.file.write_all(b"\n")?;
        self.bytes = attempted;
        Ok(())
    }
    /// Flush and synchronize the trace before exit.
    /// # Errors
    /// Filesystem flush or synchronization failure.
    pub fn finish(mut self) -> io::Result<()> {
        self.file.flush()?;
        self.file.sync_data()
    }
}
