//! Spike S0-10: which key chords reach a crossterm 0.29 app on this terminal?
//!
//! Interactive: prompts for each chord, records the raw events for 400 ms after the first one,
//! judges PASS/FAIL, and appends a report to `s0-10-report.txt` in the current directory.
//! Esc skips a step. Run it once per terminal (Windows Terminal, conhost, VS Code, WSL/SSH).

use std::fs::OpenOptions;
use std::io::{Write, stdout};
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
    KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::{execute, terminal};

type AnyResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Step {
    prompt: &'static str,
    judge: fn(&[Event]) -> bool,
}

fn presses(events: &[Event]) -> impl Iterator<Item = &KeyEvent> {
    events.iter().filter_map(|e| match e {
        Event::Key(k) if k.kind != KeyEventKind::Release => Some(k),
        _ => None,
    })
}

fn first_press(events: &[Event]) -> Option<&KeyEvent> {
    presses(events).next()
}

const STEPS: &[Step] = &[
    Step {
        prompt: "Press Enter",
        judge: |e| first_press(e).is_some_and(|k| k.code == KeyCode::Enter && k.modifiers.is_empty()),
    },
    Step {
        prompt: "Press Shift+Enter",
        judge: |e| first_press(e).is_some_and(|k| k.code == KeyCode::Enter && k.modifiers.contains(KeyModifiers::SHIFT)),
    },
    Step {
        prompt: "Press Alt+Enter (Windows Terminal may toggle fullscreen instead)",
        judge: |e| first_press(e).is_some_and(|k| k.code == KeyCode::Enter && k.modifiers.contains(KeyModifiers::ALT)),
    },
    Step {
        prompt: "Press Ctrl+J",
        judge: |e| {
            first_press(e).is_some_and(|k| {
                (k.code == KeyCode::Char('j') && k.modifiers.contains(KeyModifiers::CONTROL))
                    || (k.code == KeyCode::Enter && k.modifiers.contains(KeyModifiers::CONTROL))
            })
        },
    },
    Step {
        prompt: "Press Shift+Tab",
        judge: |e| {
            first_press(e).is_some_and(|k| {
                k.code == KeyCode::BackTab || (k.code == KeyCode::Tab && k.modifiers.contains(KeyModifiers::SHIFT))
            })
        },
    },
    Step {
        prompt: "Paste two lines of text (copy 'a<newline>b' first), e.g. Ctrl+V or right-click",
        judge: |e| e.iter().any(|ev| matches!(ev, Event::Paste(s) if s.contains('\n') || s.contains('\r'))),
    },
];

fn collect(first_wait: Duration) -> AnyResult<Option<Vec<Event>>> {
    if !event::poll(first_wait)? {
        return Ok(None);
    }
    let mut events = vec![event::read()?];
    let t = Instant::now();
    while t.elapsed() < Duration::from_millis(400) {
        if event::poll(Duration::from_millis(20))? {
            events.push(event::read()?);
        }
    }
    Ok(Some(events))
}

fn main() -> AnyResult<()> {
    let term = [("WT_SESSION", "Windows Terminal"), ("TERM_PROGRAM", "TERM_PROGRAM"), ("TERM", "TERM")]
        .iter()
        .filter_map(|(var, label)| std::env::var(var).ok().map(|v| format!("{label}={v}")))
        .collect::<Vec<_>>()
        .join(" ");
    let mut out = stdout();
    terminal::enable_raw_mode()?;
    let paste = execute!(out, EnableBracketedPaste).map_err(|e| e.to_string());
    let kitty = execute!(
        out,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    )
    .map_err(|e| e.to_string());
    let mut report = vec![
        format!("terminal: {}", if term.is_empty() { "unknown (conhost?)".to_owned() } else { term }),
        format!("bracketed paste enable: {paste:?}"),
        format!("keyboard enhancement (CSI-u) push: {kitty:?}"),
    ];
    for step in STEPS {
        write!(out, "\r\n>>> {} (Esc = skip, 30 s timeout)\r\n", step.prompt)?;
        out.flush()?;
        let line = match collect(Duration::from_secs(30))? {
            None => format!("TIMEOUT  {}", step.prompt),
            Some(ev) if first_press(&ev).is_some_and(|k| k.code == KeyCode::Esc) => format!("SKIPPED  {}", step.prompt),
            Some(ev) => {
                let verdict = if (step.judge)(&ev) { "PASS    " } else { "FAIL    " };
                format!("{verdict} {} -> {ev:?}", step.prompt)
            }
        };
        write!(out, "{line}\r\n")?;
        report.push(line);
    }
    if kitty.is_ok() {
        let _ignored = execute!(out, PopKeyboardEnhancementFlags);
    }
    let _ignored = execute!(out, DisableBracketedPaste);
    terminal::disable_raw_mode()?;
    let mut file = OpenOptions::new().create(true).append(true).open("s0-10-report.txt")?;
    writeln!(file, "==== {:?}", std::time::SystemTime::now())?;
    for line in &report {
        writeln!(file, "{line}")?;
    }
    println!("\nreport appended to s0-10-report.txt");
    Ok(())
}
