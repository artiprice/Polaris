use polaris_ui_cli::{
    presenter::{BufferedPresenter, PresenterError},
    terminal,
};
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
use terminal::*;
#[derive(Clone, Default)]
struct Fake {
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail: Option<&'static str>,
    output_budget: Arc<Mutex<Option<usize>>>,
}
impl Fake {
    fn call(&mut self, n: &'static str) -> io::Result<()> {
        self.calls
            .lock()
            .map_err(|_| io::Error::other("calls lock poisoned"))?
            .push(n);
        if self.fail == Some(n) {
            Err(io::Error::other("injected"))
        } else {
            Ok(())
        }
    }
}
impl Write for Fake {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut budget = self
            .output_budget
            .lock()
            .map_err(|_| io::Error::other("budget lock poisoned"))?;
        if let Some(remaining) = *budget {
            if remaining == 0 {
                return Err(io::Error::other("injected output failure"));
            }
            let accepted = remaining.min(b.len());
            *budget = Some(remaining.saturating_sub(accepted));
            return Ok(accepted);
        }
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
struct Ops;
impl TerminalOps<Fake> for Ops {
    fn capture_output_mode(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("output-mode+")
    }
    fn restore_output_mode(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("output-mode-")
    }
    fn enable_raw(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("raw+")
    }
    fn disable_raw(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("raw-")
    }
    fn enter_alternate(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("alt+")
    }
    fn leave_alternate(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("alt-")
    }
    fn hide_cursor(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("hide")
    }
    fn show_cursor(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("show")
    }
    fn enable_paste(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("paste+")
    }
    fn disable_paste(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("paste-")
    }
    fn enable_mouse(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("mouse+")
    }
    fn disable_mouse(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("mouse-")
    }
    fn push_keyboard(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("keyboard+")
    }
    fn pop_keyboard(&mut self, w: &mut Fake) -> io::Result<()> {
        w.call("keyboard-")
    }
}
#[test]
fn rolls_back_acquired_modes_in_reverse_order() -> Result<(), Box<dyn std::error::Error>> {
    let f = Fake {
        fail: Some("paste+"),
        ..Fake::default()
    };
    let calls = Arc::clone(&f.calls);
    let e = TerminalGuard::acquire(Ops, f)
        .err()
        .ok_or("injected acquisition error was not reported")?;
    assert!(e.failures.iter().any(|s| s.contains("acquire")));
    let got = calls
        .lock()
        .map_err(|_| io::Error::other("calls lock poisoned"))?
        .clone();
    assert_eq!(
        got,
        vec![
            "output-mode+",
            "raw+",
            "alt+",
            "hide",
            "paste+",
            "paste-",
            "show",
            "alt-",
            "raw-",
            "output-mode-"
        ]
    );
    Ok(())
}
#[test]
fn close_attempts_every_restore_step_and_reports_errors() -> Result<(), Box<dyn std::error::Error>>
{
    let f = Fake::default();
    let calls = Arc::clone(&f.calls);
    let mut g = TerminalGuard::acquire(Ops, f)?;
    g.writer_mut().fail = Some("show");
    let e = g.close().err().ok_or("show failure not reported")?;
    assert!(e.failures.iter().any(|s| s.contains("show cursor")));
    let got = calls
        .lock()
        .map_err(|_| io::Error::other("calls lock poisoned"))?
        .clone();
    assert!(got.ends_with(&[
        "keyboard-",
        "mouse-",
        "paste-",
        "show",
        "alt-",
        "raw-",
        "output-mode-"
    ]));
    Ok(())
}
#[test]
fn drop_attempts_best_effort_cleanup() -> Result<(), Box<dyn std::error::Error>> {
    let f = Fake::default();
    let calls = Arc::clone(&f.calls);
    let g = TerminalGuard::acquire(Ops, f)?;
    drop(g);
    let got = calls
        .lock()
        .map_err(|_| io::Error::other("calls lock poisoned"))?
        .clone();
    assert!(got.ends_with(&[
        "keyboard-",
        "mouse-",
        "paste-",
        "show",
        "alt-",
        "raw-",
        "output-mode-"
    ]));
    Ok(())
}

#[test]
fn drop_attempts_remaining_modes_after_restore_error() -> Result<(), Box<dyn std::error::Error>> {
    let f = Fake::default();
    let calls = Arc::clone(&f.calls);
    let mut g = TerminalGuard::acquire(Ops, f)?;
    g.writer_mut().fail = Some("show");
    drop(g);
    let got = calls
        .lock()
        .map_err(|_| io::Error::other("calls lock poisoned"))?
        .clone();
    assert!(got.ends_with(&[
        "keyboard-",
        "mouse-",
        "paste-",
        "show",
        "alt-",
        "raw-",
        "output-mode-"
    ]));
    Ok(())
}

#[test]
fn presenter_partial_write_resync_then_guard_close_attempts_cursor_reveal()
-> Result<(), Box<dyn std::error::Error>> {
    let f = Fake::default();
    let calls = Arc::clone(&f.calls);
    let budget = Arc::clone(&f.output_budget);
    let mut guard = TerminalGuard::acquire(Ops, f)?;
    guard.writer_mut().fail = Some("show");
    *budget
        .lock()
        .map_err(|_| io::Error::other("budget lock poisoned"))? = Some(1);
    let mut presenter = BufferedPresenter::new(guard);
    let mut writer = presenter.writer();
    presenter.begin_frame()?;
    writer.write_all(b"frame")?;
    assert!(matches!(presenter.commit(), Err(PresenterError::Io(_))));
    assert!(presenter.take_resync_required());
    let mut guard = presenter.into_inner()?;
    assert!(
        guard.close().is_err(),
        "restore output also fails after injected partial write"
    );
    let got = calls
        .lock()
        .map_err(|_| io::Error::other("calls lock poisoned"))?
        .clone();
    assert!(got.ends_with(&[
        "keyboard-",
        "mouse-",
        "paste-",
        "show",
        "alt-",
        "raw-",
        "output-mode-"
    ]));
    Ok(())
}

#[test]
fn failed_mode_attempt_is_restored_even_when_enable_reports_error()
-> Result<(), Box<dyn std::error::Error>> {
    for (attempt, inverse) in [
        ("raw+", "raw-"),
        ("alt+", "alt-"),
        ("hide", "show"),
        ("paste+", "paste-"),
        ("mouse+", "mouse-"),
        ("keyboard+", "keyboard-"),
        ("output-mode+", "output-mode-"),
    ] {
        let writer = Fake {
            fail: Some(attempt),
            ..Fake::default()
        };
        let calls = Arc::clone(&writer.calls);
        assert!(
            TerminalGuard::acquire(Ops, writer).is_err(),
            "fault {attempt} must reject acquisition"
        );
        let calls = calls
            .lock()
            .map_err(|_| io::Error::other("calls lock poisoned"))?;
        assert!(
            calls.contains(&inverse),
            "partial mode attempt {attempt} requires inverse {inverse}; got {calls:?}"
        );
    }
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_vt_mouse_and_keyboard_protocols_are_balanced() -> Result<(), Box<dyn std::error::Error>>
{
    let mut bytes = Vec::new();
    let mut ops = CrosstermOps::default();
    ops.enable_mouse(&mut bytes)?;
    ops.push_keyboard(&mut bytes)?;
    ops.pop_keyboard(&mut bytes)?;
    ops.disable_mouse(&mut bytes)?;
    assert_eq!(
        bytes,
        b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1006h\x1b[>1u\x1b[<u\x1b[?1006l\x1b[?1003l\x1b[?1002l\x1b[?1000l"
    );
    Ok(())
}
