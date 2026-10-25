//! Bounded category-only diagnostics. Printable keys and paste content never reach the file.
use crossterm::event::KeyCode;
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
                KeyCode::Backspace | KeyCode::Delete => "key edit",
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
        let attempted = self
            .bytes
            .checked_add(category.len())
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| io::Error::other("trace size overflow"))?;
        if attempted > 1_048_576 {
            return Err(io::Error::other("key trace reached its 1 MiB limit"));
        }
        self.file.write_all(category.as_bytes())?;
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
