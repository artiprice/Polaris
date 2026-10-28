use crossterm::{
    cursor::Show,
    event::{DisableBracketedPaste, EnableBracketedPaste, Event, KeyEventKind},
    execute,
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
};
use polaris::{
    app::App,
    frame::{EventQueue, Scheduler, UiEvent},
    output::{ColorDepth, ConsoleOutputMode},
    presentation::{FrameOutput, StableCursorBackend, TerminalPolicy},
    theme::ThemeMode,
    ui,
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    env,
    io::{self, IsTerminal, Write},
    time::{Duration, Instant},
};

type Term = Terminal<StableCursorBackend<CrosstermBackend<FrameOutput<io::Stdout>>>>;
const HELP: &str = "polaris · coding-agent UI preview\n\nUsage: polaris [--preview] [OPTIONS]\n\n  --preview               Run the full-screen preview (interactive in a terminal)\n  --interactive           Require terminal input/output; fail instead of a static view\n  --key-trace PATH        Write key/state diagnostics to a new file (no draft text)\n  --account-email EMAIL   Optional local header label; does not sign in\n  --scenario NAME         Start at a named scenario; /preview lists all\n  --theme dark|terminal   Dark NVIDIA-inspired or terminal-native styling\n  --color auto|always|never  Color policy; NO_COLOR is respected in auto\n  --color-depth auto|truecolor|ansi  RGB selection; Windows VT uses RGB in auto\n  --ascii                 Use ASCII symbols and rules\n  --reduce-motion         Use static activity indicators\n  --dump WIDTHxHEIGHT     Print a deterministic plain-text screen; no TTY needed\n  -h, --help              Show this help\n  -V, --version           Show version\n\nExamples:\n  polaris --preview\n  polaris --preview --scenario conversation\n  polaris --scenario settings --dump 80x24\n\nUI PREVIEW: all inference, tools, history, and metrics are simulated.\nUI preferences save locally. Provider edits exist in memory only. No network or tool execution.\nPOLARIS_CONFIG_DIR overrides the UI settings directory (see /settings).\n";

struct Options {
    app: App,
    color_depth: ColorDepth,
    dump: Option<(u16, u16)>,
    interactive: bool,
    key_trace: Option<std::path::PathBuf>,
}
fn parse_args() -> Result<Option<Options>, String> {
    let mut args = env::args().skip(1);
    let workspace = env::current_dir()
        .map_err(|e| e.to_string())?
        .display()
        .to_string();
    let mut app = App::new(workspace);
    let mut dump = None;
    let mut interactive = false;
    let mut key_trace = None;
    let mut scenario = "welcome".to_string();
    let mut color = "auto".to_string();
    let mut color_depth = ColorDepth::Auto;
    let mut overrides = std::collections::BTreeSet::new();
    use polaris::preferences::{ColorPolicy, PreferenceField, PreferenceState, SaveStatus};
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--preview" => {}
            "--interactive" => interactive = true,
            "--key-trace" => {
                key_trace = Some(std::path::PathBuf::from(
                    args.next().ok_or("--key-trace requires a new file path")?,
                ));
                interactive = true;
            }
            "--ascii" => {
                app.ascii = true;
                overrides.insert(PreferenceField::Ascii);
            }
            "--reduce-motion" => {
                app.reduce_motion = true;
                overrides.insert(PreferenceField::Motion);
            }
            "--theme" => match args.next().as_deref() {
                Some("dark") => {
                    app.theme.mode = ThemeMode::Dark;
                    overrides.insert(PreferenceField::Theme);
                }
                Some("terminal") => {
                    app.theme.mode = ThemeMode::Terminal;
                    overrides.insert(PreferenceField::Theme);
                }
                _ => return Err("--theme requires dark or terminal".into()),
            },
            "--account-email" => {
                let email = args
                    .next()
                    .ok_or("--account-email requires a display label")?;
                app.account_email = Some(polaris::app::safe_text(&email).replace('\n', " "));
            }
            "--scenario" => scenario = args.next().ok_or("--scenario requires a name")?,
            "--color-depth" => {
                color_depth = args
                    .next()
                    .ok_or("--color-depth requires auto, truecolor, or ansi")?
                    .parse()?;
            }
            "--color" => {
                overrides.insert(PreferenceField::Color);
                color = args
                    .next()
                    .ok_or("--color requires auto, always, or never")?;
                if !["auto", "always", "never"].contains(&color.as_str()) {
                    return Err("--color requires auto, always, or never".into());
                }
            }
            "--dump" => {
                let value = args.next().ok_or("--dump requires WIDTHxHEIGHT")?;
                let (w, h) = value
                    .split_once('x')
                    .ok_or("--dump requires WIDTHxHEIGHT")?;
                let width = w.parse::<u16>().map_err(|_| "Invalid dump width")?;
                let height = h.parse::<u16>().map_err(|_| "Invalid dump height")?;
                if !(1..=500).contains(&width) || !(1..=200).contains(&height) {
                    return Err("Dump dimensions must be 1..500 columns and 1..200 rows".into());
                }
                dump = Some((width, height));
            }
            "--help" | "-h" => {
                print!("{HELP}");
                return Ok(None);
            }
            "--version" | "-V" => {
                println!(
                    "Polaris v{}",
                    env!("CARGO_PKG_VERSION").replace("-alpha", "alpha")
                );
                return Ok(None);
            }
            _ => return Err(format!("Unknown argument: {arg}. Use --help.")),
        }
    }
    if interactive && dump.is_some() {
        return Err("--interactive cannot be combined with --dump".into());
    }
    let auto_color = env::var_os("NO_COLOR").is_none() && env::var("TERM").as_deref() != Ok("dumb");
    app.preferences = polaris::preferences::configuration_path()
        .map(|path| PreferenceState::load(path, auto_color))
        .unwrap_or_else(|| PreferenceState {
            auto_color,
            status: SaveStatus::Warning(
                "Cannot locate UI configuration directory; changes are session-only.".into(),
            ),
            ..PreferenceState::default()
        });
    for field in &overrides {
        match field {
            PreferenceField::Theme => app.preferences.values.theme = app.theme.mode,
            PreferenceField::Ascii => app.preferences.values.ascii = app.ascii,
            PreferenceField::Motion => app.preferences.values.reduced_motion = app.reduce_motion,
            PreferenceField::Color => {
                app.preferences.values.color = match color.as_str() {
                    "always" => ColorPolicy::Always,
                    "never" => ColorPolicy::Never,
                    _ => ColorPolicy::Auto,
                }
            }
            PreferenceField::UsageBar | PreferenceField::StatsChart => {}
        }
    }
    app.preferences.overrides = overrides;
    polaris::settings::sync_effective(&mut app);
    let rgb_hint = env::var("COLORTERM")
        .map(|s| s == "truecolor" || s == "24bit")
        .unwrap_or(false)
        || env::var_os("WT_SESSION").is_some();
    app.theme.rgb = color_depth.resolve_rgb(rgb_hint, false, dump.is_some());
    app.load_scenario(&scenario)?;
    if let SaveStatus::Warning(message) = &app.preferences.status {
        app.notice = polaris::app::safe_text(message);
    }
    Ok(Some(Options {
        app,
        color_depth,
        dump,
        interactive,
        key_trace,
    }))
}

struct KeyTrace(Option<io::BufWriter<std::fs::File>>);
impl KeyTrace {
    fn new(path: Option<&std::path::Path>) -> io::Result<Self> {
        path.map(|path| {
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .map(io::BufWriter::new)
        })
        .transpose()
        .map(Self)
    }
    fn record(
        &mut self,
        event: &str,
        key: Option<crossterm::event::KeyEvent>,
        app: &App,
    ) -> io::Result<()> {
        let Some(file) = &mut self.0 else {
            return Ok(());
        };
        let code = key
            .map(|key| match key.code {
                crossterm::event::KeyCode::Char(ch) if !ch.is_control() => "Text".to_string(),
                code => format!("{code:?}"),
            })
            .unwrap_or_default();
        let modifiers = key
            .map(|key| format!("{:?}", key.modifiers))
            .unwrap_or_default();
        let theme = if app.theme.mode == ThemeMode::Dark {
            "dark"
        } else {
            "terminal"
        };
        let depth = if !app.theme.color {
            "none"
        } else if app.theme.mode == ThemeMode::Terminal {
            "terminal"
        } else if app.theme.rgb {
            "truecolor"
        } else {
            "ansi"
        };
        let quoted = |text: &str| text.replace('\\', "\\\\").replace('"', "\\\"");
        writeln!(
            file,
            "{{\"event\":\"{}\",\"code\":\"{}\",\"modifiers\":\"{}\",\"screen\":\"{:?}\",\"mode\":\"{}\",\"input_lines\":{},\"input_chars\":{},\"working\":{},\"quit\":{},\"active_model\":\"{}\",\"stdin_tty\":{},\"stdout_tty\":{},\"attachment_count\":{},\"preview\":{},\"selected\":{},\"scroll\":{},\"submission_count\":{},\"effort\":\"{}\",\"expanded_tool_count\":{},\"tool_details_expanded\":{},\"selection_bytes\":{},\"theme\":\"{}\",\"color_enabled\":{},\"color_depth\":\"{}\",\"editor_cursor_byte\":{},\"usage_style\":\"{}\",\"usage_details\":{},\"settings_category\":{},\"settings_categories_focused\":{},\"preferences_status\":\"{}\",\"viewed_agent\":{},\"selected_agent\":{},\"agent_focus\":{},\"agent_count\":{},\"agents_expanded\":{},\"stats_tab\":\"{:?}\",\"stats_range\":\"{:?}\",\"stats_chart\":\"{}\",\"stats_details\":{},\"stats_demo\":{},\"stats_default\":\"{}\"}}",
            event,
            quoted(&code),
            quoted(&modifiers),
            app.screen,
            app.mode_label(),
            app.input.text.split('\n').count(),
            app.input.text.chars().count(),
            app.working,
            app.quit,
            quoted(app.model_label()),
            io::stdin().is_terminal(),
            io::stdout().is_terminal(),
            app.input.spans.len(),
            app.preview.is_some(),
            app.selected,
            app.scroll,
            app.submissions.len(),
            app.effort_label(),
            app.visible_entries()
                .iter()
                .enumerate()
                .filter(|(index, entry)| entry.kind == polaris::app::EntryKind::Tool
                    && app.tool_expanded(*index))
                .count(),
            app.tool_details_expanded,
            app.view_selection.as_ref().map_or_else(
                || app.input.selection().map_or(0, |range| range.len()),
                |s| s.anchor.abs_diff(s.end)
            ),
            theme,
            app.theme.color,
            depth,
            app.input.cursor,
            app.preferences.values.usage_bar_style.label(),
            app.usage.details,
            app.tab,
            app.settings.categories_focused,
            app.preferences.status.label(),
            app.agents
                .viewed
                .map_or_else(|| "null".into(), |id| id.to_string()),
            app.agents
                .selected
                .map_or_else(|| "null".into(), |id| id.to_string()),
            app.agents.focused,
            app.agents.records.len(),
            app.agents.expanded,
            app.stats.tab,
            app.stats.range,
            polaris::stats::effective_style(app).label(),
            app.stats.details,
            app.stats.demo,
            app.preferences.values.stats_chart_style.label()
        )?;
        Ok(())
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &mut self.0 {
            file.flush()
        } else {
            Ok(())
        }
    }
}

// The guard covers ordinary errors and unwinding panics, including setup failure.
struct TerminalGuard {
    output_mode: ConsoleOutputMode,
    #[cfg(windows)]
    _input_mode: polaris::input::NativeConsoleInput,
    raw: bool,
    alternate: bool,
    keyboard_enhanced: bool,
    mouse_enabled: bool,
    policy: TerminalPolicy,
    policy_started: bool,
}
impl TerminalGuard {
    fn new() -> io::Result<Self> {
        let output_mode = ConsoleOutputMode::enter();
        let mut guard = Self {
            output_mode,
            #[cfg(windows)]
            _input_mode: polaris::input::NativeConsoleInput::enter()?,
            raw: false,
            alternate: false,
            keyboard_enhanced: false,
            mouse_enabled: false,
            policy: TerminalPolicy::new(Instant::now()),
            policy_started: false,
        };
        terminal::enable_raw_mode()?;
        guard.raw = true;
        guard.alternate = true;
        execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste)?;
        guard.keyboard_enhanced = enable_keyboard_protocol();
        guard.mouse_enabled = polaris::input::push_mouse_protocol(&mut io::stdout()).is_ok();
        Ok(guard)
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.policy_started {
            let _ = io::stdout().write_all(&self.policy.restore());
            let _ = io::stdout().flush();
        }
        if self.mouse_enabled {
            let _ = polaris::input::pop_mouse_protocol(&mut io::stdout());
        }
        if self.keyboard_enhanced {
            disable_keyboard_protocol();
        }
        if self.alternate {
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            Show,
            crossterm::style::ResetColor
        );
        if self.raw {
            let _ = terminal::disable_raw_mode();
        }
    }
}
fn enable_keyboard_protocol() -> bool {
    polaris::input::push_keyboard_protocol(&mut io::stdout()).is_ok()
}
fn disable_keyboard_protocol() {
    let _ = polaris::input::pop_keyboard_protocol(&mut io::stdout());
}

fn decode(
    event: Event,
    app: &App,
    keyboard: &mut polaris::input::KeyboardDecoder,
    trace: &mut KeyTrace,
    queue: &mut EventQueue,
) -> io::Result<()> {
    match event {
        Event::Key(key) => {
            if !keyboard.is_pasting() {
                trace.record("raw", Some(key), app)?;
            }
            for key in keyboard.feed(if keyboard.is_pasting() {
                key
            } else {
                polaris::input::recover_console_shift(key)
            }) {
                if key.kind != KeyEventKind::Release {
                    queue.push(UiEvent::Key(key));
                }
            }
            for error in keyboard.take_paste_errors() {
                queue.push(UiEvent::Notice(error));
            }
            for text in keyboard.take_pastes() {
                queue.push(UiEvent::Paste(text));
            }
            for event in keyboard.take_mouse() {
                queue.push(UiEvent::Mouse(event));
            }
        }
        Event::Paste(text) => queue.push(UiEvent::Paste(text)),
        Event::Mouse(event) => queue.push(UiEvent::Mouse(event)),
        Event::Resize(_, _) => queue.push(UiEvent::Resize),
        _ => {}
    }
    Ok(())
}
fn prepare(app: &mut App, size: ratatui::layout::Size) {
    app.editor_width = ui::editor_width(size.width);
    app.agents.offset = polaris::agents_ui::scroll_offset(app, size.height);
    if app.screen == polaris::app::Screen::Home {
        app.scroll = app
            .scroll
            .min(ui::conversation_max_scroll(app, size.width, size.height));
    }
    if app.screen == polaris::app::Screen::Stats {
        let sheet = ui::stats_sheet(app, size.width, size.height);
        app.stats.page_rows = polaris::stats::page_rows(app, sheet);
        app.scroll = app
            .scroll
            .min(polaris::stats::max_scroll(app, sheet, size.height));
    }
    if app.screen == polaris::app::Screen::Usage {
        app.usage.page_rows = ui::usage_page_rows(app, size.width, size.height);
        app.scroll = app
            .scroll
            .min(ui::usage_max_scroll(app, size.width, size.height));
    }
    if app.screen == polaris::app::Screen::Settings {
        app.settings.page_rows = ui::settings_page_rows(app, size.width, size.height);
        app.scroll = ui::settings_scroll_offset(app, size.width, size.height);
    }
}
fn redraw(
    terminal: &mut Term,
    app: &mut App,
    map: &mut polaris::interaction::UiMap,
    output: &FrameOutput<io::Stdout>,
    synchronized: bool,
) -> io::Result<()> {
    let _time = polaris::frame::FrameTime::capture();
    let size = terminal.size()?;
    prepare(app, size);
    terminal.backend_mut().set_frame_size(size);
    crossterm::style::force_color_output(app.theme.color);
    terminal.backend_mut().set_synchronized(synchronized);
    terminal.draw(|frame| *map = ui::render_interactive(frame, app))?;
    output.commit(synchronized)
}
fn refresh_map(app: &mut App, map: &mut polaris::interaction::UiMap) -> io::Result<()> {
    let (w, h) = terminal::size()?;
    prepare(app, ratatui::layout::Size::new(w, h));
    let mut scratch = Terminal::new(ratatui::backend::TestBackend::new(w, h))?;
    scratch.draw(|f| *map = ui::render_interactive(f, app))?;
    Ok(())
}
fn clock_stamp(app: &App) -> (u64, Option<u64>, Vec<Option<u64>>) {
    (
        app.working_elapsed().as_secs(),
        app.goal
            .as_ref()
            .filter(|g| g.is_active())
            .map(|g| g.elapsed().as_secs()),
        if app.agents.visible() {
            app.agents.records.iter().map(|a| a.elapsed()).collect()
        } else {
            vec![]
        },
    )
}
fn run(
    mut app: App,
    trace_path: Option<&std::path::Path>,
    color_depth: ColorDepth,
) -> io::Result<()> {
    let mut trace = KeyTrace::new(trace_path)?;
    trace.record("bootstrap", None, &app)?;
    trace.flush()?;
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "--interactive requires terminal input and output. Run polaris --preview in an interactive terminal; omit --interactive for a static preview, or use --dump 80x24.",
        ));
    }
    // Crossterm also reads NO_COLOR; use the already-resolved application policy
    // so --color always really overrides it without editing the environment.
    crossterm::style::force_color_output(app.theme.color);
    let mut guard = TerminalGuard::new()?;
    let windows_vt = guard.output_mode.vt_available();
    app.theme.rgb = color_depth.resolve_rgb(
        app.theme.rgb,
        windows_vt && app.theme.mode == ThemeMode::Dark,
        false,
    );
    if cfg!(windows) && !windows_vt {
        if color_depth == ColorDepth::TrueColor
            && app.theme.color
            && app.theme.mode == ThemeMode::Dark
        {
            return Err(io::Error::other(
                "--color-depth truecolor requires Windows VT output; use --color-depth ansi for this console",
            ));
        }
        app.theme.rgb = false;
    }
    trace.record("colors", None, &app)?;
    let output = FrameOutput::new(io::stdout());
    let mut terminal: Term = Terminal::new(StableCursorBackend::new(CrosstermBackend::new(
        output.clone(),
    )))?;
    let clipboard = polaris::clipboard::ClipboardService::new()?;
    app.start_index();
    trace.record("start", None, &app)?;
    trace.flush()?;
    let mut last_tick = Instant::now();
    let mut keyboard = polaris::input::KeyboardDecoder::default();
    let mut input = polaris::input::TerminalInput::new()?;
    let mut mouse = polaris::runtime::MouseController::default();
    let mut map = polaris::interaction::UiMap::default();
    let mut scheduler = Scheduler::new(Instant::now());
    let mut map_dirty = true;
    let mut toast_visible = false;
    let mut clock = clock_stamp(&app);
    let mut queue = EventQueue::default();
    guard.policy = TerminalPolicy::new(Instant::now());
    guard.policy_started = true;
    io::stdout().write_all(TerminalPolicy::probe())?;
    io::stdout().flush()?;
    while !app.quit {
        if app.poll_index() && app.completion_open() {
            scheduler.invalidate();
            map_dirty = true;
        }
        while let Some(result) = clipboard.try_recv() {
            app.clipboard_result(result);
            trace.record("clipboard", None, &app)?;
            scheduler.invalidate();
            map_dirty = true;
        }
        for action in std::mem::take(&mut app.clipboard_actions) {
            let (token, result) = match action {
                polaris::app::ClipboardAction::Paste(t) => (t, clipboard.send_paste(t)),
                polaris::app::ClipboardAction::Copy(t, text) => (t, clipboard.send_copy(t, text)),
            };
            if let Err(e) = result {
                app.clipboard_result(polaris::clipboard::ClipboardResult {
                    token,
                    result: Err(e.to_string()),
                });
                scheduler.invalidate();
                map_dirty = true;
            }
        }
        let visible = app.toast_text().is_some();
        if visible != toast_visible {
            scheduler.invalidate();
            toast_visible = visible;
        }
        if guard.policy.fallback_due(Instant::now()) {
            execute!(io::stdout(), crossterm::cursor::SetCursorStyle::SteadyBlock)?;
        }
        let new_clock = clock_stamp(&app);
        if new_clock != clock {
            clock = new_clock;
            scheduler.invalidate();
        }
        if scheduler.due(Instant::now()) {
            redraw(
                &mut terminal,
                &mut app,
                &mut map,
                &output,
                guard.policy.synchronized,
            )?;
            map_dirty = false;
            scheduler.presented(Instant::now());
            trace.record("frame", None, &app)?;
        }
        trace.flush()?;
        let mut wait = Duration::from_millis(200).saturating_sub(last_tick.elapsed());
        if keyboard.pending()
            || !app.clipboard_pending.is_empty()
            || app.file_index.as_ref().is_some_and(|i| !i.is_ready())
        {
            wait = wait.min(Duration::from_millis(16));
        }
        if let Some((_, until)) = &app.toast
            && *until > Instant::now()
        {
            wait = wait.min(until.saturating_duration_since(Instant::now()));
        }
        wait = scheduler.wait(Instant::now(), wait);
        if input.poll(wait)? {
            let deadline = Instant::now() + Duration::from_millis(4);
            loop {
                decode(input.read()?, &app, &mut keyboard, &mut trace, &mut queue)?;
                if queue.len() >= 256
                    || Instant::now() >= deadline
                    || !input.poll(Duration::ZERO)?
                {
                    break;
                }
            }
        }
        // A frame budget or slow trace write is not an input timeout. Never
        // expire a partial escape sequence while its remaining records are queued.
        if !input.poll(Duration::ZERO)? {
            for key in keyboard.expire() {
                queue.push(UiEvent::Key(key));
            }
        }
        for (mode, state) in keyboard.take_mode_replies() {
            guard.policy.reply(mode, state);
            trace.record("terminal_reply", None, &app)?;
        }
        while let Some(event) = queue.pop() {
            match event {
                UiEvent::Key(key) => {
                    if key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL)
                        && key.code == crossterm::event::KeyCode::Char('c')
                        && app.view_selection.is_some()
                    {
                        if map_dirty {
                            refresh_map(&mut app, &mut map)?;
                        }
                        app.request_copy(polaris::runtime::selection_text(&app, &map));
                    } else {
                        app.key(key);
                    }
                    trace.record("key", Some(key), &app)?;
                    scheduler.invalidate();
                    map_dirty = true;
                }
                UiEvent::Paste(text) => {
                    app.paste(&text);
                    trace.record("paste", None, &app)?;
                    scheduler.invalidate();
                    map_dirty = true;
                }
                UiEvent::Notice(text) => {
                    app.notice = text;
                    trace.record("paste_error", None, &app)?;
                    scheduler.invalidate();
                }
                UiEvent::Resize => {
                    if app.screen == polaris::app::Screen::Settings {
                        app.settings.reveal_selection = true;
                    }
                    scheduler.invalidate();
                    map_dirty = true;
                }
                UiEvent::Mouse(event) => {
                    if map_dirty {
                        refresh_map(&mut app, &mut map)?;
                        map_dirty = false;
                        trace.record("map", None, &app)?;
                    }
                    let old = app.mouse_position.and_then(|(x, y)| map.hit(x, y)).cloned();
                    let new = map.hit(event.column, event.row).cloned();
                    mouse.event(event, &map, &mut app);
                    if event.kind != crossterm::event::MouseEventKind::Moved {
                        scheduler.invalidate();
                        map_dirty = true;
                    } else if old != new {
                        scheduler.invalidate();
                    }
                    trace.record("mouse", None, &app)?;
                }
            }
            if app.quit {
                break;
            }
        }
        if last_tick.elapsed() >= Duration::from_millis(200) {
            let before = (
                app.working,
                app.visible_entries().len(),
                app.agents
                    .records
                    .iter()
                    .map(|a| (a.status, a.tokens, a.transcript.len()))
                    .collect::<Vec<_>>(),
            );
            app.tick();
            last_tick = Instant::now();
            let after = (
                app.working,
                app.visible_entries().len(),
                app.agents
                    .records
                    .iter()
                    .map(|a| (a.status, a.tokens, a.transcript.len()))
                    .collect::<Vec<_>>(),
            );
            if before != after {
                scheduler.invalidate();
                map_dirty = true;
            }
        }
    }
    trace.flush()?;
    drop(terminal);
    drop(guard);
    writeln!(
        io::stdout(),
        "polaris preview closed. No provider connections or tools were run."
    )?;
    Ok(())
}

fn main() {
    let result = match parse_args() {
        Ok(Some(options)) => {
            if let Some((w, h)) = options.dump {
                ui::dump(&options.app, w, h)
                    .and_then(|text| io::stdout().write_all(text.as_bytes()))
            } else if options.interactive
                || (io::stdin().is_terminal() && io::stdout().is_terminal())
            {
                run(
                    options.app,
                    options.key_trace.as_deref(),
                    options.color_depth,
                )
            } else if options.interactive {
                Err(io::Error::other(
                    "--interactive requires terminal input and output. Run polaris --preview in an interactive terminal, or omit --interactive for a static preview.",
                ))
            } else {
                eprintln!(
                    "polaris: No interactive terminal detected. Showing a static 80x24 preview; keyboard input is unavailable."
                );
                eprintln!(
                    "Run the same command in Windows Terminal or PowerShell for the interactive UI. Use --dump WIDTHxHEIGHT to choose static dimensions."
                );
                ui::dump(&options.app, 80, 24)
                    .and_then(|text| io::stdout().write_all(text.as_bytes()))
            }
        }
        Ok(None) => Ok(()),
        Err(message) => Err(io::Error::other(message)),
    };
    if let Err(error) = result {
        eprintln!("polaris: {error}");
        std::process::exit(1);
    }
}
