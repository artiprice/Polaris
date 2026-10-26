use polaris_ui_cli::presenter::{
    BufferedPresenter, CursorIntent, CursorState, MAX_FRAME_BYTES, PresenterError,
};
use ratatui::{
    Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    layout::{Position, Rect},
    widgets::Paragraph,
};
use std::{
    error::Error,
    io::{self, Write},
};
type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn changed_cells_restore_unchanged_editor_cursor_after_paint() -> TestResult {
    let mut presenter =
        BufferedPresenter::new_with_cursor_state(Sink::default(), CursorState::Hidden);
    let backend = CrosstermBackend::new(presenter.writer());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 20, 5)),
        },
    )?;
    for effort in ["medium", "high"] {
        let before = presenter.output().bytes.len();
        presenter.begin_frame()?;
        terminal.draw(|frame| {
            frame.render_widget(Paragraph::new(effort), Rect::new(10, 4, 10, 1));
            frame.set_cursor_position(Position::new(1, 1));
        })?;
        let _written = presenter.commit()?;
        let delta = presenter
            .output()
            .bytes
            .get(before..)
            .ok_or("output moved backwards")?;
        assert!(
            delta.ends_with(b"\x1b[2;2H\x1b[?25h"),
            "paint must end in editor position then reveal: {delta:?}"
        );
        if effort == "high" {
            assert!(
                delta.starts_with(b"\x1b[?25l"),
                "visible caret must be hidden while repainting"
            );
        }
    }
    presenter.begin_frame()?;
    terminal.draw(|frame| {
        frame.render_widget(Paragraph::new("high"), Rect::new(10, 4, 10, 1));
        frame.set_cursor_position(Position::new(1, 1));
    })?;
    assert_eq!(presenter.commit()?, 0);
    Ok(())
}

#[derive(Default)]
struct Sink {
    bytes: Vec<u8>,
    flushes: usize,
    fail_after: Option<usize>,
}
impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if let Some(left) = self.fail_after {
            if left == 0 {
                return Err(io::Error::other("injected partial failure"));
            }
            let written = left.min(bytes.len());
            let prefix = bytes
                .get(..written)
                .ok_or_else(|| io::Error::other("partial write exceeds input length"))?;
            self.bytes.extend_from_slice(prefix);
            self.fail_after = Some(left - written);
            return Ok(written);
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        Ok(())
    }
}

#[test]
fn commits_only_explicit_nonempty_frames() -> TestResult {
    let mut presenter = BufferedPresenter::new(Sink::default());
    presenter.begin_frame()?;
    let mut writer = presenter.writer();
    writer.write_all(b"frame")?;
    assert_eq!(presenter.output().bytes.len(), 0);
    assert_eq!(presenter.commit()?, 5);
    assert_eq!(presenter.output().bytes, b"frame");
    let flushes = presenter.output().flushes;
    presenter.begin_frame()?;
    assert_eq!(presenter.commit()?, 0);
    assert_eq!(presenter.output().flushes, flushes);
    Ok(())
}

#[test]
fn bounds_staged_frame_and_marks_resync_after_partial_write() -> TestResult {
    let mut presenter = BufferedPresenter::new(Sink {
        fail_after: Some(2),
        ..Sink::default()
    });
    presenter.begin_frame()?;
    let mut writer = presenter.writer();
    assert!(writer.write_all(&vec![b'x'; MAX_FRAME_BYTES + 1]).is_err());
    assert!(matches!(
        presenter.commit(),
        Err(PresenterError::FrameTooLarge)
    ));
    assert!(!presenter.take_resync_required());
    presenter.begin_frame()?;
    let mut writer = presenter.writer();
    writer.write_all(b"frame")?;
    assert!(presenter.commit().is_err());
    assert!(presenter.take_resync_required());
    assert_eq!(presenter.output().bytes, b"fr");
    Ok(())
}

#[test]
fn stages_and_deduplicates_cursor_intents() -> TestResult {
    let mut presenter =
        BufferedPresenter::new_with_cursor_state(Sink::default(), CursorState::Hidden);
    presenter.begin_frame()?;
    presenter.stage_cursor(CursorIntent::Hide)?;
    assert_eq!(presenter.commit()?, 0);
    presenter.begin_frame()?;
    presenter.stage_cursor(CursorIntent::Show)?;
    presenter.stage_cursor(CursorIntent::MoveTo(3, 4))?;
    assert!(presenter.commit()? > 0);
    let bytes = presenter.output().bytes.clone();
    presenter.begin_frame()?;
    presenter.stage_cursor(CursorIntent::Show)?;
    presenter.stage_cursor(CursorIntent::MoveTo(3, 4))?;
    assert_eq!(presenter.commit()?, 0);
    assert_eq!(presenter.output().bytes, bytes);
    presenter.begin_frame()?;
    presenter.stage_cursor(CursorIntent::Show)?;
    presenter.stage_cursor(CursorIntent::MoveTo(3, 5))?;
    assert_eq!(presenter.commit()?, 6);
    assert!(presenter.output().bytes.ends_with(b"\x1b[6;4H"));
    Ok(())
}

#[test]
fn extracted_presenter_closes_live_writer_handles() -> TestResult {
    let presenter = BufferedPresenter::new(Sink::default());
    let mut writer = presenter.writer();
    let _output = presenter.into_inner()?;
    assert!(writer.write_all(b"late").is_err());
    Ok(())
}

#[test]
fn ratatui_identical_grid_emits_zero_and_cursor_change_emits_only_position() -> TestResult {
    let mut presenter =
        BufferedPresenter::new_with_cursor_state(Sink::default(), CursorState::Hidden);
    let backend = CrosstermBackend::new(presenter.writer());
    let options = TerminalOptions {
        viewport: Viewport::Fixed(Rect::new(0, 0, 20, 5)),
    };
    let mut terminal = Terminal::with_options(backend, options)?;
    presenter.begin_frame()?;
    let first_buffer = terminal
        .draw(|frame| {
            frame.render_widget(Paragraph::new("same grid"), frame.area());
            frame.set_cursor_position(Position { x: 1, y: 1 });
        })?
        .buffer
        .clone();
    assert!(presenter.commit()? > 0);
    let before = presenter.output().bytes.len();
    presenter.begin_frame()?;
    let second_buffer = terminal
        .draw(|frame| {
            frame.render_widget(Paragraph::new("same grid"), frame.area());
            frame.set_cursor_position(Position { x: 1, y: 1 });
        })?
        .buffer
        .clone();
    assert_eq!(
        first_buffer, second_buffer,
        "Ratatui buffers should be identical"
    );
    let identical_bytes = presenter.commit()?;
    let emitted = presenter
        .output()
        .bytes
        .get(before..)
        .ok_or_else(|| io::Error::other("presenter output length moved backwards"))?;
    assert_eq!(
        identical_bytes, 0,
        "staged identical frame delta: {emitted:?}"
    );
    assert_eq!(presenter.output().bytes.len(), before);
    presenter.begin_frame()?;
    terminal.draw(|frame| {
        frame.render_widget(Paragraph::new("same grid"), frame.area());
        frame.set_cursor_position(Position { x: 3, y: 1 });
    })?;
    assert_eq!(presenter.commit()?, 6);
    assert!(presenter.output().bytes.ends_with(b"\x1b[2;4H"));
    Ok(())
}
