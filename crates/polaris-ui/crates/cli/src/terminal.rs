//! Fallible terminal mode acquisition with explicit, retryable restoration.
use crossterm::{
    cursor::{Hide, Show},
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};
/// Aggregated errors from terminal acquisition or restoration.
#[derive(Debug)]
pub struct TerminalError {
    /// Each mode operation or flush that failed.
    pub failures: Vec<String>,
}
impl std::fmt::Display for TerminalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "terminal lifecycle had {} failure(s): {}",
            self.failures.len(),
            self.failures.join("; ")
        )
    }
}
impl std::error::Error for TerminalError {}
/// Safe, injectable operations used to acquire and restore terminal modes.
pub trait TerminalOps<W: Write> {
    /// Enables Crossterm raw input mode.
    fn enable_raw(&mut self, w: &mut W) -> io::Result<()>;
    /// Restores cooked input mode.
    fn disable_raw(&mut self, w: &mut W) -> io::Result<()>;
    /// Enters the alternate screen.
    fn enter_alternate(&mut self, w: &mut W) -> io::Result<()>;
    /// Leaves the alternate screen.
    fn leave_alternate(&mut self, w: &mut W) -> io::Result<()>;
    /// Hides the terminal cursor.
    fn hide_cursor(&mut self, w: &mut W) -> io::Result<()>;
    /// Restores cursor visibility.
    fn show_cursor(&mut self, w: &mut W) -> io::Result<()>;
    /// Enables literal bracketed paste events.
    fn enable_paste(&mut self, w: &mut W) -> io::Result<()>;
    /// Disables bracketed paste reporting.
    fn disable_paste(&mut self, w: &mut W) -> io::Result<()>;
    /// Enables mouse capture.
    fn enable_mouse(&mut self, w: &mut W) -> io::Result<()>;
    /// Disables mouse capture.
    fn disable_mouse(&mut self, w: &mut W) -> io::Result<()>;
}
/// Crossterm implementation of [`TerminalOps`].
#[derive(Debug, Default)]
pub struct CrosstermOps;
impl<W: Write> TerminalOps<W> for CrosstermOps {
    fn enable_raw(&mut self, _: &mut W) -> io::Result<()> {
        terminal::enable_raw_mode()
    }
    fn disable_raw(&mut self, _: &mut W) -> io::Result<()> {
        terminal::disable_raw_mode()
    }
    fn enter_alternate(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, EnterAlternateScreen)
    }
    fn leave_alternate(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, LeaveAlternateScreen)
    }
    fn hide_cursor(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, Hide)
    }
    fn show_cursor(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, Show)
    }
    fn enable_paste(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, crossterm::event::EnableBracketedPaste)
    }
    fn disable_paste(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, crossterm::event::DisableBracketedPaste)
    }
    fn enable_mouse(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, EnableMouseCapture)
    }
    fn disable_mouse(&mut self, w: &mut W) -> io::Result<()> {
        execute!(w, DisableMouseCapture)
    }
}
/// Owns the terminal writer and restoration of every possibly changed terminal mode.
pub struct TerminalGuard<O: TerminalOps<W>, W: Write> {
    ops: O,
    writer: W,
    raw: bool,
    alternate: bool,
    hidden: bool,
    paste: bool,
    mouse: bool,
}
impl<O, W> TerminalGuard<O, W>
where
    O: TerminalOps<W>,
    W: Write,
{
    /// Acquires modes in order and restores every attempted mode on failure.
    /// An operation can fail after a partial write or flush, so its inverse is armed first.
    /// # Errors
    /// Acquisition and every failed restoration are retained in the error report.
    pub fn acquire(ops: O, writer: W) -> Result<Self, TerminalError> {
        let mut g = Self {
            ops,
            writer,
            raw: false,
            alternate: false,
            hidden: false,
            paste: false,
            mouse: false,
        };
        let r = (|| -> io::Result<()> {
            g.raw = true;
            g.ops.enable_raw(&mut g.writer)?;
            g.alternate = true;
            g.ops.enter_alternate(&mut g.writer)?;
            g.hidden = true;
            g.ops.hide_cursor(&mut g.writer)?;
            g.paste = true;
            g.ops.enable_paste(&mut g.writer)?;
            g.mouse = true;
            g.ops.enable_mouse(&mut g.writer)?;
            g.writer.flush()
        })();
        if let Err(e) = r {
            let mut failures = vec![format!("acquire: {e}")];
            failures.extend(g.restore());
            return Err(TerminalError { failures });
        }
        Ok(g)
    }
    fn restore(&mut self) -> Vec<String> {
        let mut e = Vec::new();
        macro_rules! attempt {
            ($field:ident,$call:expr,$label:literal) => {
                if self.$field {
                    match $call {
                        Ok(()) => self.$field = false,
                        Err(x) => e.push(format!(concat!($label, ": {}"), x)),
                    }
                }
            };
        }
        attempt!(
            mouse,
            self.ops.disable_mouse(&mut self.writer),
            "disable mouse"
        );
        attempt!(
            paste,
            self.ops.disable_paste(&mut self.writer),
            "disable paste"
        );
        attempt!(
            hidden,
            self.ops.show_cursor(&mut self.writer),
            "show cursor"
        );
        attempt!(
            alternate,
            self.ops.leave_alternate(&mut self.writer),
            "leave alternate screen"
        );
        attempt!(
            raw,
            self.ops.disable_raw(&mut self.writer),
            "disable raw mode"
        );
        if let Err(x) = self.writer.flush() {
            e.push(format!("flush restore: {x}"));
        }
        e
    }
    /// Attempts every remaining restoration step; failed steps remain retryable.
    pub fn close(&mut self) -> Result<(), TerminalError> {
        let failures = self.restore();
        if failures.is_empty() {
            Ok(())
        } else {
            Err(TerminalError { failures })
        }
    }
    /// Borrows the terminal writer for a presenter or diagnostics.
    pub fn writer_mut(&mut self) -> &mut W {
        &mut self.writer
    }
}
impl<O, W> Drop for TerminalGuard<O, W>
where
    O: TerminalOps<W>,
    W: Write,
{
    fn drop(&mut self) {
        let failures = self.restore();
        if !failures.is_empty() {
            tracing::error!(failures = ?failures, "best-effort terminal restoration failed during drop");
        }
    }
}

impl<O, W> Write for TerminalGuard<O, W>
where
    O: TerminalOps<W>,
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writer.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

impl<O, W> std::fmt::Debug for TerminalGuard<O, W>
where
    O: TerminalOps<W>,
    W: Write,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalGuard")
            .field("raw", &self.raw)
            .field("alternate", &self.alternate)
            .field("hidden", &self.hidden)
            .field("paste", &self.paste)
            .field("mouse", &self.mouse)
            .finish_non_exhaustive()
    }
}
