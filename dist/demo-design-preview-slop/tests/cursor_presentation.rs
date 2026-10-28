use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris::{app::App, presentation::StableCursorBackend, ui};
use ratatui::{
    Terminal,
    backend::{Backend, ClearType, TestBackend, WindowSize},
    buffer::Cell,
    layout::{Position, Size},
};
use std::{cell::RefCell, io, rc::Rc};
#[derive(Clone, Debug)]
enum Event {
    Hide,
    Show(Position),
    Paint(bool),
    Move,
    Flush,
}
struct Recorder {
    inner: TestBackend,
    events: Rc<RefCell<Vec<Event>>>,
    visible: bool,
    fault: Option<&'static str>,
    position: Position,
}
impl Recorder {
    fn new(w: u16, h: u16, events: Rc<RefCell<Vec<Event>>>) -> Self {
        Self {
            inner: TestBackend::new(w, h),
            events,
            visible: true,
            fault: None,
            position: Position::ORIGIN,
        }
    }
}
impl Backend for Recorder {
    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let cells = content.collect::<Vec<_>>();
        for &(x, y, c) in &cells {
            self.events.borrow_mut().push(Event::Paint(self.visible));
            self.position = Position::new(x + c.symbol().chars().count() as u16, y);
        }
        if self.fault == Some("draw") {
            return Err(io::Error::other("injected paint failure"));
        }
        self.inner.draw(cells.into_iter())
    }
    fn hide_cursor(&mut self) -> io::Result<()> {
        self.visible = false;
        self.events.borrow_mut().push(Event::Hide);
        self.inner.hide_cursor()
    }
    fn show_cursor(&mut self) -> io::Result<()> {
        self.visible = true;
        self.events.borrow_mut().push(Event::Show(self.position));
        self.inner.show_cursor()
    }
    fn set_cursor_position<P: Into<Position>>(&mut self, p: P) -> io::Result<()> {
        if self.fault == Some("position") {
            return Err(io::Error::other("injected position failure"));
        }
        self.position = p.into();
        self.events.borrow_mut().push(Event::Move);
        self.inner.set_cursor_position(self.position)
    }
    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(self.position)
    }
    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }
    fn clear_region(&mut self, t: ClearType) -> io::Result<()> {
        self.inner.clear_region(t)
    }
    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }
    fn size(&self) -> io::Result<Size> {
        self.inner.size()
    }
    fn window_size(&mut self) -> io::Result<WindowSize> {
        self.inner.window_size()
    }
    fn flush(&mut self) -> io::Result<()> {
        self.events.borrow_mut().push(Event::Flush);
        if self.fault == Some("flush") {
            return Err(io::Error::other("injected flush failure"));
        }
        Ok(())
    }
}
fn assert_reveal(events: &[Event], position: Position, require_paint: bool) {
    assert!(
        !require_paint || events.iter().any(|e| matches!(e, Event::Paint(_))),
        "regression must exercise a real repaint"
    );
    assert!(
        !events.iter().any(|e| matches!(e, Event::Paint(true))),
        "cursor was visible during painting: {events:?}"
    );
    let reveals = events
        .iter()
        .filter_map(|e| {
            if let Event::Show(p) = e {
                Some(*p)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    if !events
        .iter()
        .any(|e| matches!(e, Event::Paint(_) | Event::Hide))
    {
        assert!(
            reveals.is_empty(),
            "cursor-only/no-op frames must not toggle visibility"
        );
        return;
    }
    assert_eq!(
        reveals,
        [position],
        "cursor must be revealed only at its final position"
    );
    let show = events
        .iter()
        .rposition(|e| matches!(e, Event::Show(_)))
        .unwrap();
    assert!(events[..show].iter().any(|e| matches!(e, Event::Move)));
}
#[test]
fn inserting_inside_something_never_reveals_cursor_at_repaint_end() {
    let events = Rc::new(RefCell::new(vec![]));
    let mut terminal = Terminal::new(StableCursorBackend::new(Recorder::new(
        80,
        24,
        events.clone(),
    )))
    .unwrap();
    let mut app = App::default();
    app.input.set("something");
    for _ in 0..5 {
        app.input.left();
    }
    terminal.draw(|f| ui::render(f, &app)).unwrap();
    events.borrow_mut().clear();
    for ch in "text".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        terminal.draw(|f| ui::render(f, &app)).unwrap();
        let expected = Position::new(6 + app.input.cursor as u16, 20);
        assert_eq!(terminal.get_cursor_position().unwrap(), expected);
        assert_reveal(&events.borrow(), expected, true);
        events.borrow_mut().clear();
    }
    assert_eq!(app.input.text, "sometextthing");
    assert_eq!(app.input.cursor, 8);
}

fn reference_cursor(app: &App, w: u16, h: u16) -> Position {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(|f| ui::render(f, app)).unwrap();
    t.get_cursor_position().unwrap()
}
#[test]
fn unicode_tabs_wrapping_paste_and_deletion_keep_cursor_placement_stable() {
    for (w, h) in [(60, 20), (80, 24), (120, 40)] {
        for text in [
            "界é\tabcDEF\nsecond line\nthird line".to_string(),
            format!("{}abcDEF", "x".repeat(w as usize - 7)),
        ] {
            let events = Rc::new(RefCell::new(vec![]));
            let mut t = Terminal::new(StableCursorBackend::new(Recorder::new(
                w,
                h,
                events.clone(),
            )))
            .unwrap();
            let mut app = App::default();
            app.input.set(&text);
            app.input.cursor = text.find("abc").unwrap() + 1;
            t.draw(|f| ui::render(f, &app)).unwrap();
            events.borrow_mut().clear();
            for key in [
                KeyEvent::new(KeyCode::Char('λ'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
                KeyEvent::new(KeyCode::Char('字'), KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
                KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            ] {
                app.key(key);
                t.draw(|f| ui::render(f, &app)).unwrap();
                let expected = reference_cursor(&app, w, h);
                assert_eq!(t.get_cursor_position().unwrap(), expected);
                assert_reveal(&events.borrow(), expected, false);
                events.borrow_mut().clear();
            }
            let at = app.input.cursor;
            let mut expected_text = app.input.text.clone();
            expected_text.insert_str(at, "PASTE");
            app.paste("PASTE");
            t.draw(|f| ui::render(f, &app)).unwrap();
            assert_eq!(app.input.text, expected_text);
            assert_eq!(app.input.cursor, at + 5);
            assert_reveal(&events.borrow(), reference_cursor(&app, w, h), true);
        }
    }
}
#[test]
fn popup_cursor_stays_hidden_and_restores_the_midline_position_on_return() {
    let events = Rc::new(RefCell::new(vec![]));
    let mut t = Terminal::new(StableCursorBackend::new(Recorder::new(
        80,
        24,
        events.clone(),
    )))
    .unwrap();
    let mut app = App::default();
    app.input.set("something");
    app.input.cursor = 4;
    t.draw(|f| ui::render(f, &app)).unwrap();
    events.borrow_mut().clear();
    for command in ["/model", "/status", "/help"] {
        app.command(command);
        t.draw(|f| ui::render(f, &app)).unwrap();
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, Event::Show(_) | Event::Paint(true)))
        );
        events.borrow_mut().clear();
        app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        t.draw(|f| ui::render(f, &app)).unwrap();
        assert_eq!(app.input.cursor, 4);
        assert_reveal(&events.borrow(), Position::new(10, 20), true);
        events.borrow_mut().clear();
    }
}
#[test]
fn failed_paint_position_and_flush_restore_cursor_during_cleanup() {
    for fault in ["draw", "position", "flush"] {
        let events = Rc::new(RefCell::new(vec![]));
        let mut backend = Recorder::new(80, 24, events.clone());
        backend.fault = Some(fault);
        let mut t = Terminal::new(StableCursorBackend::new(backend)).unwrap();
        let app = App::default();
        assert!(t.draw(|f| ui::render(f, &app)).is_err());
        drop(t);
        assert!(
            matches!(events.borrow().last(), Some(Event::Show(_))),
            "hidden cursor after {fault}: {:?}",
            events.borrow()
        );
        assert!(
            !events
                .borrow()
                .iter()
                .any(|e| matches!(e, Event::Paint(true)))
        );
    }
}

#[test]
fn unchanged_frames_and_cursor_only_moves_do_not_hide_or_reveal() {
    let events = Rc::new(RefCell::new(vec![]));
    let mut t = Terminal::new(StableCursorBackend::new(Recorder::new(
        120,
        40,
        events.clone(),
    )))
    .unwrap();
    let mut app = App::default();
    app.input.set("something");
    t.draw(|f| ui::render(f, &app)).unwrap();
    events.borrow_mut().clear();
    for _ in 0..20 {
        t.draw(|f| ui::render(f, &app)).unwrap();
    }
    assert!(events.borrow().iter().all(|e| matches!(e, Event::Flush)));
    events.borrow_mut().clear();
    app.input.left();
    t.draw(|f| ui::render(f, &app)).unwrap();
    assert_eq!(
        events
            .borrow()
            .iter()
            .filter(|e| matches!(e, Event::Move))
            .count(),
        1
    );
    assert!(
        !events
            .borrow()
            .iter()
            .any(|e| matches!(e, Event::Hide | Event::Show(_) | Event::Paint(_)))
    );
}
