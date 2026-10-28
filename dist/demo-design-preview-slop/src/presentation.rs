//! Stage cursor intent and terminal bytes; commit only complete changed frames.
use ratatui::{
    backend::{Backend, ClearType, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use std::{
    cell::RefCell,
    io::{self, Write},
    rc::Rc,
};
pub struct StableCursorBackend<B: Backend> {
    inner: B,
    in_frame: bool,
    desired: Option<bool>,
    position: Option<Position>,
    visible: Option<bool>,
    last_position: Option<Position>,
    synchronized: bool,
    frame_size: Option<Size>,
}
impl<B: Backend> StableCursorBackend<B> {
    pub fn new(inner: B) -> Self {
        Self {
            inner,
            in_frame: false,
            desired: None,
            position: None,
            visible: None,
            last_position: None,
            synchronized: false,
            frame_size: None,
        }
    }
    pub fn set_frame_size(&mut self, size: Size) {
        self.frame_size = Some(size);
    }
    pub fn set_synchronized(&mut self, supported: bool) {
        self.synchronized = supported;
    }
}
impl<B: Backend> Backend for StableCursorBackend<B> {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        self.in_frame = true;
        self.desired = None;
        self.position = None;
        let mut cells = content.peekable();
        if cells.peek().is_some() {
            self.last_position = None;
            if !self.synchronized && self.visible != Some(false) {
                self.inner.hide_cursor()?;
                self.visible = Some(false);
            }
            self.inner.draw(cells)?;
        }
        Ok(())
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        if self.in_frame {
            self.desired = Some(false);
            Ok(())
        } else {
            self.visible = Some(false);
            self.inner.hide_cursor()
        }
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        if self.in_frame {
            self.desired = Some(true);
            Ok(())
        } else {
            self.visible = Some(true);
            self.inner.show_cursor()
        }
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, p: P) -> io::Result<()> {
        let p = p.into();
        if self.in_frame {
            self.position = Some(p);
            Ok(())
        } else {
            self.last_position = Some(p);
            self.inner.set_cursor_position(p)
        }
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        self.inner.get_cursor_position()
    }
    fn clear(&mut self) -> io::Result<()> {
        self.last_position = None;
        self.inner.clear()
    }
    fn clear_region(&mut self, t: ClearType) -> io::Result<()> {
        self.last_position = None;
        self.inner.clear_region(t)
    }
    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.last_position = None;
        self.inner.append_lines(n)
    }
    fn size(&self) -> io::Result<Size> {
        self.frame_size.map_or_else(|| self.inner.size(), Ok)
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(p) = self.position
            && self.last_position != Some(p)
        {
            self.inner.set_cursor_position(p)?;
            self.last_position = Some(p);
        }
        if let Some(v) = self.desired
            && self.visible != Some(v)
        {
            if v {
                self.inner.show_cursor()?;
            } else {
                self.inner.hide_cursor()?;
            }
            self.visible = Some(v);
        }
        self.inner.flush()?;
        self.in_frame = false;
        self.frame_size = None;
        Ok(())
    }
}
impl<B: Backend> Drop for StableCursorBackend<B> {
    fn drop(&mut self) {
        if self.in_frame {
            let _ = self.inner.show_cursor();
        }
    }
}

struct Output<W> {
    writer: W,
    pending: Vec<u8>,
}
pub struct FrameOutput<W: Write>(Rc<RefCell<Output<W>>>);
impl<W: Write> Clone for FrameOutput<W> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<W: Write> FrameOutput<W> {
    pub fn new(writer: W) -> Self {
        Self(Rc::new(RefCell::new(Output {
            writer,
            pending: vec![],
        })))
    }
    pub fn commit(&self, synchronized: bool) -> io::Result<()> {
        let mut out = self.0.borrow_mut();
        if out.pending.is_empty() {
            return Ok(());
        }
        let mut bytes = std::mem::take(&mut out.pending);
        if synchronized {
            let mut frame = Vec::with_capacity(bytes.len() + 16);
            frame.extend_from_slice(b"\x1b[?2026h");
            frame.append(&mut bytes);
            frame.extend_from_slice(b"\x1b[?2026l");
            bytes = frame;
        }
        let result = out
            .writer
            .write_all(&bytes)
            .and_then(|()| out.writer.flush());
        if result.is_err() {
            let _ = out.writer.write_all(b"\x1b[?2026l\x1b[?25h");
            let _ = out.writer.flush();
        }
        bytes.clear();
        out.pending = bytes;
        result
    }
}
impl<W: Write> Write for FrameOutput<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.borrow_mut().pending.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    // Crossterm execute! calls flush for individual commands. Only commit reaches the OS.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Clone, Default)]
    struct Sink(Rc<RefCell<(Vec<u8>, usize, usize)>>);
    impl Write for Sink {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            let mut s = self.0.borrow_mut();
            s.0.extend_from_slice(b);
            s.1 += 1;
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            self.0.borrow_mut().2 += 1;
            Ok(())
        }
    }
    #[test]
    fn commands_are_buffered_and_empty_commits_write_nothing() {
        let sink = Sink::default();
        let mut out = FrameOutput::new(sink.clone());
        out.write_all(b"paint").unwrap();
        out.flush().unwrap();
        assert!(sink.0.borrow().0.is_empty());
        out.commit(true).unwrap();
        assert_eq!(sink.0.borrow().0, b"\x1b[?2026hpaint\x1b[?2026l");
        assert_eq!((sink.0.borrow().1, sink.0.borrow().2), (1, 1));
        out.commit(true).unwrap();
        assert_eq!((sink.0.borrow().1, sink.0.borrow().2), (1, 1));
    }
}

/// Capability replies never delay startup or user input.
pub struct TerminalPolicy {
    pub synchronized: bool,
    original_blink: Option<bool>,
    fallback: bool,
    deadline: std::time::Instant,
}
impl TerminalPolicy {
    pub fn new(now: std::time::Instant) -> Self {
        Self {
            synchronized: false,
            original_blink: None,
            fallback: false,
            deadline: now + std::time::Duration::from_millis(100),
        }
    }
    pub const fn probe() -> &'static [u8] {
        b"\x1b[?2026$p\x1b[?12$p\x1b[?12l"
    }
    pub fn reply(&mut self, mode: u16, state: u16) {
        match mode {
            2026 => self.synchronized = matches!(state, 1 | 2),
            12 if self.original_blink.is_none() && matches!(state, 1 | 2) => {
                self.original_blink = Some(state == 1)
            }
            _ => {}
        }
    }
    pub fn fallback_due(&mut self, now: std::time::Instant) -> bool {
        if !self.fallback && self.original_blink.is_none() && now >= self.deadline {
            self.fallback = true;
            true
        } else {
            false
        }
    }
    pub fn restore(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        if self.synchronized {
            bytes.extend_from_slice(b"\x1b[?2026l");
        }
        if self.fallback {
            bytes.extend_from_slice(b"\x1b[0 q");
        }
        match self.original_blink {
            Some(true) => bytes.extend_from_slice(b"\x1b[?12h"),
            Some(false) => bytes.extend_from_slice(b"\x1b[?12l"),
            None => {}
        }
        bytes
    }
}
#[cfg(test)]
mod policy_tests {
    use super::*;
    use std::time::{Duration, Instant};
    #[test]
    fn supported_unsupported_and_late_replies_restore_cursor_without_forcing_blink() {
        let now = Instant::now();
        let mut p = TerminalPolicy::new(now);
        p.reply(2026, 2);
        p.reply(12, 2);
        assert!(p.synchronized);
        assert!(!p.fallback_due(now + Duration::from_secs(1)));
        assert_eq!(p.restore(), b"\x1b[?2026l\x1b[?12l");
        let mut p = TerminalPolicy::new(now);
        p.reply(2026, 0);
        assert!(!p.synchronized);
        assert!(!p.fallback_due(now));
        assert!(p.fallback_due(now + Duration::from_millis(100)));
        assert!(!p.fallback_due(now + Duration::from_secs(1)));
        assert_eq!(p.restore(), b"\x1b[0 q");
        p.reply(12, 1);
        assert_eq!(p.restore(), b"\x1b[0 q\x1b[?12h");
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    struct FailsOnce {
        bytes: Rc<RefCell<Vec<u8>>>,
        writes: usize,
        flush_failure: bool,
        short_write: bool,
    }
    impl Write for FailsOnce {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            if self.writes == 2 && self.short_write {
                return Err(io::Error::other("injected short-write failure"));
            }
            let n = if self.writes == 1 && self.short_write {
                5.min(b.len())
            } else {
                b.len()
            };
            self.bytes.borrow_mut().extend_from_slice(&b[..n]);
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            if self.flush_failure {
                self.flush_failure = false;
                Err(io::Error::other("injected flush failure"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn partial_write_and_flush_failures_end_sync_and_reveal_cursor() {
        for flush_failure in [false, true] {
            let bytes = Rc::new(RefCell::new(vec![]));
            let mut out = FrameOutput::new(FailsOnce {
                bytes: bytes.clone(),
                writes: 0,
                flush_failure,
                short_write: !flush_failure,
            });
            out.write_all(b"paint").unwrap();
            assert!(out.commit(true).is_err());
            assert!(bytes.borrow().ends_with(b"\x1b[?2026l\x1b[?25h"));
        }
    }
}
