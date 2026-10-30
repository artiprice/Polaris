//! Application-owned terminal loop, deterministic dumps and real offline backend coordination.
use crate::{
    Args, CliError,
    cli_settings::{CliSettings, Navigation, PersistencePolicy, SettingsService},
    demo::DemoRuntime,
    demo_gallery::{DemoGallery, GalleryEffect, GalleryPage},
    key_trace::KeyTrace,
    session::{ChatProgress, OfflineSession, ResolvedRecord},
    ui_services::{
        ClipboardResult, ClipboardService, IndexLimits, PreferenceOverrides, WorkspaceFileIndex,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, SessionId},
};
use polaris_ui_cli::{
    input::{InputReader, NormalizedInput, TerminalInput},
    presenter::{BufferedPresenter, CursorState},
    scheduler::InputScheduler,
    screen::{
        self, ColorMode, CompletionOverlay, InfoSheet, PanelVisibility, RenderOptions, ScreenState,
    },
    terminal::{CrosstermOps, TerminalGuard},
};
use polaris_ui_model::{
    completion::{self, StaticPathIndex},
    composer::{Composer, PreparedSubmission},
    keymap,
    session::SessionViews,
    view::ViewState,
};
use polaris_ui_session::{ConversationHost, HostEvent, TurnReport};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect};
use std::{
    collections::{BTreeMap, VecDeque},
    io::{self, IsTerminal, Write},
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, watch};

/// Validated finite dump size, checked before any backend allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DumpSize {
    /// Columns, 1 through 400.
    pub width: u16,
    /// Rows, 1 through 200.
    pub height: u16,
}
impl FromStr for DumpSize {
    type Err = String;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let (w, h) = text
            .split_once(['x', 'X'])
            .ok_or_else(|| "expected WxH, for example 80x24".to_owned())?;
        let width = w
            .parse::<u16>()
            .map_err(|_| "invalid dump width".to_owned())?;
        let height = h
            .parse::<u16>()
            .map_err(|_| "invalid dump height".to_owned())?;
        if width == 0 || height == 0 || width > 400 || height > 200 {
            return Err("dump must be within 1x1 through 400x200 (80000 cells)".to_owned());
        }
        Ok(Self { width, height })
    }
}
/// Explicit application color policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorPolicy {
    /// Detect terminal and NO_COLOR.
    Auto,
    /// Force the chosen palette.
    Always,
    /// Emit no application colors.
    Never,
}
/// Requested palette depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ColorDepth {
    /// Conservative ANSI palette unless truecolor is advertised.
    Auto,
    /// Explicit RGB palette.
    Truecolor,
    /// Portable indexed colors.
    Ansi,
}
/// Terminal background policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum ThemeMode {
    /// Black application canvas.
    Dark,
    /// Inherit the terminal canvas/colors.
    Terminal,
}

/// Validate branch conflicts and TTY availability before state/log/terminal initialization.
/// # Errors
/// Invalid option combinations or a default interactive invocation without a TTY.
pub fn validate_invocation(args: &Args) -> Result<(), CliError> {
    if args.scenario.is_some() && args.dump.is_none() && !args.demo {
        return Err(CliError::Unavailable(
            "--scenario requires --dump or --demo",
        ));
    }
    if args.command.is_some()
        && (args.dump.is_some()
            || args.prompt.is_some()
            || args.scenario.is_some()
            || args.key_trace.is_some()
            || args.demo)
    {
        return Err(CliError::Unavailable(
            "UI options cannot be combined with a headless subcommand",
        ));
    }
    if args.dump.is_some() && args.key_trace.is_some() {
        return Err(CliError::Unavailable(
            "--key-trace is for interactive input, not --dump",
        ));
    }
    if args.command.is_none()
        && args.dump.is_none()
        && (!io::stdin().is_terminal() || !io::stdout().is_terminal())
    {
        return Err(CliError::Unavailable(
            "an interactive terminal is required; use --dump WxH or a headless subcommand",
        ));
    }
    if let Some(scenario) = args.scenario.as_deref() {
        let known = if args.demo {
            crate::demo_gallery::SCENARIOS
                .iter()
                .any(|(name, _)| *name == scenario)
        } else {
            matches!(
                scenario,
                "welcome" | "conversation" | "error" | "completion" | "panel" | "help"
            )
        };
        if !known {
            return Err(CliError::Unavailable(if args.demo {
                "unknown demo --scenario; open /preview in Demo to see the scenario catalog"
            } else {
                "unknown offline --scenario; use welcome, conversation, error, completion, panel or help"
            }));
        }
    }
    Ok(())
}
fn render_options(args: &Args) -> RenderOptions {
    options_from_preferences(
        preference_overrides(args).apply(polaris_config::CliPreferences::default()),
        args.dump.is_some(),
    )
}
fn preference_overrides(args: &Args) -> PreferenceOverrides {
    use polaris_config::{CliColorDepth, CliColorPolicy, CliTheme};
    PreferenceOverrides {
        theme: args.theme.map(|value| match value {
            ThemeMode::Dark => CliTheme::Dark,
            ThemeMode::Terminal => CliTheme::Terminal,
        }),
        color: args.color.map(|value| match value {
            ColorPolicy::Auto => CliColorPolicy::Auto,
            ColorPolicy::Always => CliColorPolicy::Always,
            ColorPolicy::Never => CliColorPolicy::Never,
        }),
        color_depth: args.color_depth.map(|value| match value {
            ColorDepth::Auto => CliColorDepth::Auto,
            ColorDepth::Truecolor => CliColorDepth::Truecolor,
            ColorDepth::Ansi => CliColorDepth::Ansi,
        }),
        ascii: if args.ascii {
            Some(true)
        } else if args.no_ascii {
            Some(false)
        } else {
            None
        },
        reduce_motion: if args.reduce_motion {
            Some(true)
        } else if args.no_reduce_motion {
            Some(false)
        } else {
            None
        },
        panel: None,
    }
}
fn options_from_preferences(
    preferences: polaris_config::CliPreferences,
    // Only the Windows live-TTY colour policy reads it.
    #[cfg_attr(not(windows), allow(unused_variables))] dump: bool,
) -> RenderOptions {
    use polaris_config::{CliColorDepth, CliColorPolicy, CliTheme};
    let no_color = std::env::var_os("NO_COLOR").is_some();
    let term_dumb = std::env::var("TERM").ok().as_deref() == Some("dumb");
    let color_term_truecolor = std::env::var("COLORTERM")
        .ok()
        .is_some_and(|s| s == "truecolor" || s == "24bit");
    #[cfg(windows)]
    // Windows live TTY policy (verified through native ConPTY): select RGB
    // without an environment hint. Do not call supports_ansi() here: that
    // function changes console output mode before terminal acquisition.
    // Crossterm's terminal transport owns activation; explicit ANSI remains
    // available for a host that cannot display RGB.
    let windows_vt_supported = !dump && io::stdout().is_terminal();
    #[cfg(not(windows))]
    let windows_vt_supported = false;
    let colors = resolve_color_mode(
        match preferences.color {
            CliColorPolicy::Auto => ColorPolicy::Auto,
            CliColorPolicy::Always => ColorPolicy::Always,
            CliColorPolicy::Never => ColorPolicy::Never,
        },
        match preferences.color_depth {
            CliColorDepth::Auto => ColorDepth::Auto,
            CliColorDepth::Truecolor => ColorDepth::Truecolor,
            CliColorDepth::Ansi => ColorDepth::Ansi,
        },
        match preferences.theme {
            CliTheme::Dark => ThemeMode::Dark,
            CliTheme::Terminal => ThemeMode::Terminal,
        },
        no_color,
        term_dumb,
        color_term_truecolor,
        windows_vt_supported,
    );
    RenderOptions {
        colors,
        ascii: preferences.ascii,
    }
}
fn resolve_color_mode(
    policy: ColorPolicy,
    depth: ColorDepth,
    theme: ThemeMode,
    no_color: bool,
    term_dumb: bool,
    color_term_truecolor: bool,
    windows_vt_supported: bool,
) -> ColorMode {
    if matches!(policy, ColorPolicy::Never)
        || (matches!(policy, ColorPolicy::Auto) && (no_color || term_dumb))
    {
        return ColorMode::NoColor;
    }
    if theme == ThemeMode::Terminal {
        return ColorMode::Terminal;
    }
    match depth {
        ColorDepth::Truecolor => ColorMode::TrueColor,
        ColorDepth::Ansi => ColorMode::Ansi,
        ColorDepth::Auto => {
            if color_term_truecolor || windows_vt_supported {
                ColorMode::TrueColor
            } else {
                ColorMode::Ansi
            }
        }
    }
}
fn offline_view() -> Result<ViewState, CliError> {
    let model = ModelId::new("offline")
        .map_err(|_| CliError::Unavailable("invalid offline model identity"))?;
    Ok(ViewState::new(model, Effort::Medium))
}
/// Render only, through the same pure screen function as interactive mode.
/// # Errors
/// Invalid fixture, draft, dimensions, renderer or stdout failure. Creates no state directory.
pub fn dump(args: &Args) -> Result<(), CliError> {
    let size = args
        .dump
        .ok_or(CliError::Unavailable("missing dump dimensions"))?;
    if args.demo {
        let mut state = UiState::new(
            SessionId::new_v7(),
            args.workspace.display().to_string(),
            args.prompt.clone(),
            Vec::new(),
        )?;
        state.demo_active = true;
        state.offline_ready = false;
        state.viewport = Rect::new(0, 0, size.width, size.height);
        if let Some(scenario) = args.scenario.as_deref() {
            load_demo_scenario(&mut state, scenario)?;
        }
        if args.scenario.as_deref() == Some("working") {
            let _changed = state
                .demo
                .tick(Duration::from_millis(640))
                .map_err(ui_error)?;
        }
        let options = render_options(args);
        let mut terminal =
            Terminal::new(ratatui::backend::TestBackend::new(size.width, size.height))
                .map_err(ui_error)?;
        terminal
            .draw(|frame| {
                let _metadata = paint_ui_frame(frame, &mut state, &options);
            })
            .map_err(ui_error)?;
        let mut output = String::new();
        for y in 0..size.height {
            for x in 0..size.width {
                if let Some(cell) = terminal.backend().buffer().cell((x, y)) {
                    output.push_str(cell.symbol());
                }
            }
            output.push('\n');
        }
        let mut stdout = io::stdout().lock();
        stdout.write_all(output.as_bytes())?;
        stdout.flush()?;
        return Ok(());
    }
    let mut view = offline_view()?;
    let mut composer = Composer::new();
    if let Some(prompt) = &args.prompt {
        composer
            .insert(prompt)
            .map_err(|e| CliError::Ui(e.to_string()))?
    }
    let mut panel = PanelVisibility::Closed;
    let mut lines = Vec::new();
    if let Some(scenario) = args.scenario.as_deref() {
        match scenario {
            "conversation" => {
                view.push_user("SYNTHETIC render fixture: review this Rust function")
                    .map_err(ui_error)?;
                view.push_assistant("fn greeting() -> &'static str {\n    \"hello\"\n}\n\nThis is display test data, not an executed response.").map_err(ui_error)?;
            }
            "error" => {
                view.push_error("SYNTHETIC fixture: operation unavailable; no backend was started")
                    .map_err(ui_error)?;
            }
            "panel" => panel = PanelVisibility::Open,
            "help" => lines = help_lines(),
            "welcome" | "completion" => {}
            _ => return Err(CliError::Unavailable("unknown render scenario")),
        }
    }
    let paths = StaticPathIndex::new(Vec::<String>::new());
    let completion = if args.scenario.as_deref() == Some("completion") {
        composer
            .insert("/")
            .map_err(|e| CliError::Ui(e.to_string()))?;
        completion::complete(composer.text(), composer.cursor(), &paths)
    } else {
        Vec::new()
    };
    let notice = if args.demo {
        "DEMO · scripted UI only; no model calls or session writes"
    } else if args.scenario.is_some() {
        "SYNTHETIC render fixture - no backend"
    } else {
        "Render only - no session or backend started"
    };
    let screen = ScreenState::new(&view, &composer)
        .with_notice(Some(notice))
        .with_panel(panel)
        .with_completion(if completion.is_empty() {
            None
        } else {
            Some(CompletionOverlay {
                title: "Commands",
                items: &completion,
                selected: None,
            })
        })
        .with_sheet(if lines.is_empty() {
            None
        } else {
            Some(InfoSheet {
                title: "Help",
                lines: &lines,
                footer: Some("Render fixture"),
                scroll: 0,
            })
        });
    let text = screen::dump_text(&screen, &render_options(args), size.width, size.height)
        .map_err(|e| CliError::Ui(e.to_string()))?;
    let mut out = io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.write_all(b"\n")?;
    out.flush()?;
    Ok(())
}
fn effective_bindings() -> Vec<keymap::KeyBinding> {
    keymap::keymap()
        .iter()
        .filter_map(|binding| {
            if !matches!(
                binding.context,
                keymap::Context::Composer
                    | keymap::Context::Global
                    | keymap::Context::Popup
                    | keymap::Context::InfoPopup
                    | keymap::Context::SelectionMenu
            ) {
                return None;
            }
            if !matches!(binding.trigger, keymap::Trigger::Key(_)) {
                return None;
            }
            if !matches!(
                binding.action,
                keymap::Action::Submit
                    | keymap::Action::Newline
                    | keymap::Action::HistoryPrev
                    | keymap::Action::HistoryNext
                    | keymap::Action::ToggleMode
                    | keymap::Action::EffortUp
                    | keymap::Action::EffortDown
                    | keymap::Action::OpenHelp
                    | keymap::Action::OpenStatus
                    | keymap::Action::ToggleTools
                    | keymap::Action::Undo
                    | keymap::Action::Redo
                    | keymap::Action::InterruptOrExit
                    | keymap::Action::PopupBack
                    | keymap::Action::ClosePopup
                    | keymap::Action::ConfirmSelection
                    | keymap::Action::ScrollPageUp
                    | keymap::Action::ScrollPageDown
            ) {
                return None;
            }
            let mut effective = *binding;
            if binding.action == keymap::Action::Newline {
                let keymap::Trigger::Key(chord) = binding.trigger else {
                    return None;
                };
                if chord.key == keymap::Key::Enter && chord.mods.ctrl {
                    return None;
                }
            }
            if binding.action == keymap::Action::ToggleMode {
                effective.description = "Toggle plan mode"
            } else if binding.action == keymap::Action::EffortUp {
                effective.description = "Raise effort (switches to the provider's model for it)"
            } else if binding.action == keymap::Action::EffortDown {
                effective.description = "Lower effort (switches to the provider's model for it)"
            } else if binding.action == keymap::Action::InterruptOrExit {
                effective.description = "Close overlays, clear draft; twice on empty draft exits"
            }
            if !matches!(
                binding.action,
                keymap::Action::Submit
                    | keymap::Action::Newline
                    | keymap::Action::OpenHelp
                    | keymap::Action::ClosePopup
                    | keymap::Action::PopupBack
                    | keymap::Action::ConfirmSelection
            ) {
                effective.footer = None
            }
            Some(effective)
        })
        .collect()
}
fn footer_text(state: &UiState) -> String {
    let context = if state.sheet.is_some() {
        keymap::Context::InfoPopup
    } else if !state.completion.is_empty() {
        keymap::Context::CompletionMenu
    } else {
        keymap::Context::Composer
    };
    let (first, last) = screen::composer_visual_guards(&state.composer, composer_area(state));
    keymap::footer_text(
        &effective_bindings(),
        context,
        keymap::GuardState {
            draft_empty: state.composer.is_empty(),
            first_visual_row: first,
            last_visual_row: last,
        },
    )
}
fn help_lines() -> Vec<String> {
    let mut lines = vec![
        "Commands: /help /status /model /providers /usage /sessions /session resume <id> /clear /quit"
            .to_owned(),
        "Ask anything about the workspace; the model reads files with its tools. Esc interrupts a reply."
            .to_owned(),
        "Mention files with @path; switch models with /model or Shift+Up/Shift+Down effort. /onboarding explains setup."
            .to_owned(),
        "Cockpit: /panel [tasks|agents|questions|review|proof|context|system|off], F4 toggle, F6 focus"
            .to_owned(),
        "Enter sends; Shift+Enter inserts a newline. Ctrl+J or Alt+Enter are compatibility fallbacks."
            .to_owned(),
        "Auto mode may change files: each change shows its diff; y applies, n declines, Enter with a note declines with it. Plan mode only reads."
            .to_owned(),
        "NVIDIA: /nv-profile shows the detected stack and GPUs, /nv-verify runs the CUDA advisor; $nvidia(off|list|auto) controls what replies get."
            .to_owned(),
        "/hack audits the workspace for races, blocking calls in async code, unsafe code, secrets and CUDA bugs."
            .to_owned(),
        "/ci shows the workspace's CI pipeline; /ci-run runs it in Docker with cached cargo home and target."
            .to_owned(),
        "/search-local <words> ranks workspace passages (lexical); $rag adds the best ones to each question."
            .to_owned(),
    ];
    lines.push(
        "Ctrl+V/Shift+Insert paste text; Ctrl+C copies selection; Ctrl+X cuts after clipboard acknowledgement. Images are unavailable."
            .to_owned(),
    );
    lines.push("F5 or click offline/demo chooses the profile. DEMO: /demo long, /demo replay, /demo reset. /settings edits CLI preferences.".to_owned());
    lines.push("Mouse: hover controls, click completions, drag to select, double-click a word. Click a paste chip to preview/copy/expand.".to_owned());
    lines.extend(keymap::help_lines(&effective_bindings()));
    lines
}

#[derive(Debug)]
enum BackendRequest {
    StartOffline,
    Directive {
        session: SessionId,
        text: String,
    },
    /// Record typed input, then answer it with a chat turn on the session kernel.
    Prompt {
        session: SessionId,
        text: String,
        /// Plan mode: ask for a plan rather than a full answer.
        plan: bool,
    },
    /// Interrupt the running turn.
    CancelTurn,
    /// Forget the conversation carried into the next turn.
    ClearChat,
    /// Continue on the provider's model for this effort.
    SetEffort(Effort),
    /// Activity across every local session.
    Stats,
    /// `/nv-profile`, `/nv-verify` or `/hack` on the session's workspace.
    Local(polaris_ui_session::commands::LocalCommand),
    Sessions,
    Resume(SessionId),
    /// Apply or decline the change the running turn waits on.
    Review {
        request_id: polaris_sdk::human::HumanRequestId,
        apply: bool,
        note: Option<String>,
    },
}
#[derive(Debug)]
enum BackendMessage {
    OfflineReady,
    Record(ResolvedRecord),
    Committed,
    Rejected(String),
    QueryFailed(String),
    ResumeRejected(String),
    Sheet {
        title: String,
        lines: Vec<String>,
    },
    Resumed {
        id: SessionId,
        workspace: String,
    },
    /// The provider chain selected a model for this run.
    ProviderReady {
        provider: &'static str,
        model: String,
        effort: Effort,
        /// The provider's model for every effort, for `/model`.
        models: Vec<(Effort, String)>,
        skipped: Vec<String>,
    },
    /// The conversation continues on another model or effort.
    ModelSwitched {
        model: String,
        effort: Effort,
    },
    /// No provider is usable; typed input is still recorded.
    ProviderUnavailable(String),
    /// Typed input was recorded and its turn is starting.
    PromptAccepted,
    TurnProgress(ChatProgress),
    TurnFinished(TurnReport),
    TurnFailed(String),
}

fn send_message(
    tx: &mpsc::Sender<BackendMessage>,
    message: BackendMessage,
) -> Result<(), CliError> {
    tx.try_send(message).map_err(|_| {
        CliError::Unavailable(
            "UI event intake overloaded or closed; durable records remain replayable",
        )
    })
}
/// Start the real local profile and keep kernel/control work off the terminal owner thread.
/// # Errors
/// Kernel, terminal, input, projection, persistence, query, or cleanup failure.
pub async fn run(
    args: &Args,
    loaded: &polaris_config::Loaded,
    dirs: &polaris_config::Dirs,
) -> Result<(), CliError> {
    let id = match args.session {
        Some(id) => id,
        None => SessionId::new_v7(),
    };
    let mut session = if args.demo {
        None
    } else {
        Some(OfflineSession::open(&args.state_dir, &args.workspace, id).await?)
    };
    let workspace = match session.as_ref() {
        Some(active) => active.workspace().display().to_string(),
        None => std::fs::canonicalize(&args.workspace)?
            .display()
            .to_string(),
    };
    let (commands_tx, mut commands_rx) = mpsc::channel(4);
    // At most 32 resolved 64 KiB payloads, plus fixed envelopes: bounded independent intake.
    let (events_tx, events_rx) = mpsc::channel(32);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let settings = CliSettings::from_snapshot(
        dirs.clone(),
        loaded.config.ui.cli,
        preference_overrides(args),
        if args.demo {
            PersistencePolicy::MemoryOnly
        } else {
            PersistencePolicy::Global
        },
    );
    let options = options_from_preferences(settings.effective(), false);
    let demo = args.demo;
    let initial_scenario = args.scenario.clone();
    let initial = args.prompt.clone();
    let trace = args.key_trace.clone();
    let root = args.state_dir.clone();
    let config_workspace = args.workspace.clone();
    let provenance: Vec<String> = loaded
        .provenance
        .iter()
        .map(|(key, layer, _)| format!("{key}: {layer:?}"))
        .collect();
    let signal_stop = stop_tx.clone();
    let mut ui = tokio::task::spawn_blocking(move || {
        terminal_loop(
            id,
            workspace,
            initial,
            trace,
            options,
            provenance,
            demo,
            initial_scenario,
            settings,
            commands_tx,
            events_rx,
            signal_stop,
        )
    });
    let mut ui_observed = false;
    let mut chat = ConversationHost::new(
        loaded.config.clone(),
        polaris_ui_session::default_env_file(args.env_file.as_deref(), &dirs.global),
    );
    if !demo {
        chat.start_pick();
    }
    // Changes the running turn waits on, by request id, for answers from the terminal.
    let mut reviews = BTreeMap::new();
    let backend_result = async {
        loop {
            tokio::select! {
                result = &mut ui => {
                    ui_observed = true;
                    break result.map_err(|_| CliError::Unavailable("terminal owner failed"))?;
                }
                signal = crate::app::shutdown_signal() => {
                    signal?;
                    stop_tx.send_replace(true);
                    break Ok(());
                }
                changed = stop_rx.changed() => {
                    if changed.is_err() || *stop_rx.borrow() {
                        break Ok(());
                    }
                }
                item = next_offline_record(&mut session) => match item {
                    Ok(Some(item)) => {
                        if let Err(error) = send_message(&events_tx, BackendMessage::Record(item)) {
                            break Err(error);
                        }
                    }
                    Ok(None) => break Err(CliError::Unavailable("kernel event stream closed")),
                    Err(error) => break Err(error),
                },
                event = chat.next_event() => {
                    match &event {
                        HostEvent::Progress(ChatProgress::Review(review)) => {
                            reviews.insert(review.request_id, review.clone());
                        }
                        HostEvent::Progress(ChatProgress::ReviewClosed(id)) => {
                            reviews.remove(id);
                        }
                        HostEvent::Finished(_) | HostEvent::Failed(_) => reviews.clear(),
                        _ => {}
                    }
                    if let Err(error) = handle_chat_event(event, &events_tx) {
                        break Err(error);
                    }
                }
                request = commands_rx.recv() => {
                    let Some(request) = request else { break Ok(()) };
                    if matches!(request, BackendRequest::StartOffline) {
                        if session.is_none() {
                            session = Some(OfflineSession::open(&root, &config_workspace, id).await?);
                        }
                        chat.start_pick();
                        send_message(&events_tx, BackendMessage::OfflineReady)?;
                        continue;
                    }
                    let Some(active) = session.as_mut() else {
                        send_message(
                            &events_tx,
                            BackendMessage::Rejected("Offline profile is not active".to_owned()),
                        )?;
                        continue;
                    };
                    let reply = match request {
                        BackendRequest::StartOffline => None,
                        BackendRequest::Directive { session: expected, text } => {
                            if expected == active.id {
                                Some(match active.apply_directive(&text).await {
                                    Ok(_) => BackendMessage::Committed,
                                    Err(error) => BackendMessage::Rejected(error.to_string()),
                                })
                            } else {
                                Some(BackendMessage::Rejected(
                                    "Session changed; directive was not executed".to_owned(),
                                ))
                            }
                        }
                        BackendRequest::Prompt {
                            session: expected,
                            text,
                            plan,
                        } => {
                            if expected != active.id {
                                Some(BackendMessage::Rejected(
                                    "Session changed; prompt was not sent".to_owned(),
                                ))
                            } else if chat.is_running() {
                                Some(BackendMessage::Rejected(
                                    "A reply is still running; press Esc to interrupt it".to_owned(),
                                ))
                            } else if !chat.is_ready() {
                                Some(BackendMessage::Rejected(chat.unavailable_reason()))
                            } else if let Err(error) = active.submit_user_input(&text).await {
                                Some(BackendMessage::Rejected(error.to_string()))
                            } else {
                                for record in polaris_ui_session::records_until_own_input(active).await? {
                                    send_message(&events_tx, BackendMessage::Record(record))?;
                                }
                                send_message(&events_tx, BackendMessage::PromptAccepted)?;
                                chat.start_turn(active, text, plan)
                                    .await
                                    .err()
                                    .map(|error| BackendMessage::TurnFailed(error.to_string()))
                            }
                        }
                        BackendRequest::CancelTurn => {
                            chat.cancel_turn();
                            None
                        }
                        BackendRequest::Review { request_id, apply, note } => {
                            match reviews.get(&request_id) {
                                Some(review) => polaris_ui_session::answer_review(
                                    active.kernel(),
                                    review,
                                    apply,
                                    note,
                                )
                                .await
                                .err()
                                .map(|error| BackendMessage::QueryFailed(error.to_string())),
                                None => Some(BackendMessage::QueryFailed(
                                    "That change is no longer waiting for a decision".to_owned(),
                                )),
                            }
                        }
                        BackendRequest::ClearChat => {
                            chat.clear();
                            None
                        }
                        BackendRequest::SetEffort(effort) => Some(match chat.set_effort(effort) {
                            Ok(model) => BackendMessage::ModelSwitched { model, effort },
                            Err(reason) => BackendMessage::Rejected(reason),
                        }),
                        BackendRequest::Stats => {
                            Some(match polaris_ui_session::list_sessions(&root, 10_000).await {
                                Ok(rows) => {
                                    let now = jiff::Zoned::now();
                                    BackendMessage::Sheet {
                                        title: "Stats".to_owned(),
                                        lines: polaris_ui_session::stats::stats_lines(
                                            &rows,
                                            now.date(),
                                            now.time_zone(),
                                        ),
                                    }
                                }
                                Err(error) => BackendMessage::QueryFailed(error.to_string()),
                            })
                        }
                        BackendRequest::Local(command) => {
                            // Answered off the backend loop: a CI run can take minutes.
                            let kernel = active.kernel().clone();
                            let workspace = active.workspace().to_path_buf();
                            let events = events_tx.clone();
                            tokio::spawn(async move {
                                let message = match command.run(&kernel, &workspace).await {
                                    Ok(lines) => BackendMessage::Sheet {
                                        title: command.title().to_owned(),
                                        lines,
                                    },
                                    Err(error) => BackendMessage::QueryFailed(error.to_string()),
                                };
                                let _delivered = send_message(&events, message);
                            });
                            None
                        }
                        BackendRequest::Sessions => Some(match active.list_sessions().await {
                            Ok(rows) => BackendMessage::Sheet {
                                title: "Sessions".to_owned(),
                                lines: rows
                                    .into_iter()
                                    .map(|s| {
                                        format!(
                                            "{}  {:?}  {}  {}",
                                            s.id,
                                            s.status,
                                            s.name,
                                            s.workspace.display()
                                        )
                                    })
                                    .collect(),
                            },
                            Err(error) => BackendMessage::QueryFailed(error.to_string()),
                        }),
                        BackendRequest::Resume(target) => {
                            if chat.is_running() {
                                Some(BackendMessage::ResumeRejected(
                                    "A reply is still running; interrupt it first".to_owned(),
                                ))
                            } else if !active.session_exists(target).await? {
                                Some(BackendMessage::ResumeRejected(
                                    "Unknown session; no new session was created".to_owned(),
                                ))
                            } else {
                                active.close().await?;
                                drop(session.take());
                                chat.reset_conversation();
                                match OfflineSession::open(&root, &config_workspace, target).await {
                                    Ok(next) => {
                                        let workspace = next.workspace().display().to_string();
                                        session = Some(next);
                                        Some(BackendMessage::Resumed { id: target, workspace })
                                    }
                                    Err(error) => break Err(CliError::from(error)),
                                }
                            }
                        }
                    };
                    if let Some(message) = reply
                        && let Err(error) = send_message(&events_tx, message) {
                            break Err(error);
                        }
                }
            }
        }
    }
    .await;
    chat.shutdown().await;
    stop_tx.send_replace(true);
    let cleanup = match session.as_mut() {
        Some(session) => session.close().await.map_err(CliError::from),
        None => Ok(()),
    };
    let ui_cleanup = if !ui_observed {
        // Native terminal I/O may be physically uninterruptible; do not block kernel shutdown on it.
        match tokio::time::timeout(Duration::from_secs(5), &mut ui).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CliError::Unavailable(
                "terminal owner failed during shutdown",
            )),
            Err(_) => Err(CliError::Unavailable(
                "terminal owner did not stop within the cooperative grace period",
            )),
        }
    } else {
        Ok(())
    };
    let cleanup = combine_results(cleanup, ui_cleanup);
    match (backend_result, cleanup) {
        (Err(primary), Err(cleanup)) => Err(CliError::Ui(format!(
            "UI failed: {primary}; cleanup failed: {cleanup}"
        ))),
        (Err(error), _) | (_, Err(error)) => Err(error),
        _ => Ok(()),
    }
}

fn handle_chat_event(event: HostEvent, tx: &mpsc::Sender<BackendMessage>) -> Result<(), CliError> {
    let message = match event {
        HostEvent::ProviderReady {
            provider,
            model,
            effort,
            models,
            skipped,
        } => BackendMessage::ProviderReady {
            provider,
            model,
            effort,
            models,
            skipped,
        },
        HostEvent::ProviderUnavailable(reason) => BackendMessage::ProviderUnavailable(reason),
        HostEvent::Progress(item) => BackendMessage::TurnProgress(item),
        HostEvent::Finished(report) => BackendMessage::TurnFinished(report),
        HostEvent::Failed(reason) => BackendMessage::TurnFailed(reason),
    };
    send_message(tx, message)
}

async fn next_offline_record(
    session: &mut Option<OfflineSession>,
) -> Result<Option<ResolvedRecord>, CliError> {
    match session.as_mut() {
        Some(active) => active.next_record().await.map_err(CliError::from),
        None => std::future::pending().await,
    }
}

/// Model provider state as the terminal sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderState {
    Connecting,
    Ready { provider: &'static str },
    Unavailable(String),
}

/// What the running turn is doing, for the status row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TurnPhase {
    Waiting,
    Thinking,
    Writing,
}

/// Terminal-side state of the running chat turn.
#[derive(Debug)]
struct TurnUi {
    started: Instant,
    /// Conversation entry receiving streamed text.
    entry: Option<usize>,
    /// A new model request began after text was shown; separate the next text.
    break_pending: bool,
    phase: TurnPhase,
    /// A change waits for the person's decision.
    waiting: bool,
    /// A review card ended the streamed text; a later answer goes below it.
    after_review: bool,
}
impl TurnUi {
    fn new() -> Self {
        Self {
            started: Instant::now(),
            entry: None,
            break_pending: false,
            phase: TurnPhase::Waiting,
            waiting: false,
            after_review: false,
        }
    }
    fn label(&self) -> String {
        if self.waiting {
            return "◆ Waiting for your decision · y apply · n don't apply · or type a note and Enter"
                .to_owned();
        }
        const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
        let elapsed = self.started.elapsed();
        let frame = usize::try_from(elapsed.as_millis() / 100).unwrap_or(0) % SPINNER.len();
        let phase = match self.phase {
            TurnPhase::Waiting => "Working",
            TurnPhase::Thinking => "Thinking",
            TurnPhase::Writing => "Writing",
        };
        format!(
            "{} {phase} {} · Esc to interrupt",
            SPINNER[frame],
            polaris_ui_model::view::format_duration_ms(
                u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
            )
        )
    }
}

#[derive(Debug)]
struct OwnedSheet {
    title: String,
    lines: Vec<String>,
    scroll: usize,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanelKind {
    Tasks,
    Agents,
    Questions,
    Review,
    Proof,
    Context,
    System,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PasteTarget {
    session: SessionId,
    demo: bool,
    revision: u64,
}
impl PanelKind {
    fn title(self) -> &'static str {
        match self {
            Self::Tasks => "Tasks",
            Self::Agents => "Agents",
            Self::Questions => "Questions",
            Self::Review => "Review",
            Self::Proof => "Proof",
            Self::Context => "Context",
            Self::System => "System",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionSurface {
    Composer,
    Transcript,
    Question { index: usize, notes: bool },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionGranularity {
    Grapheme,
    Word,
    Line,
}
#[derive(Debug, Clone, Copy)]
struct SelectionGesture {
    surface: SelectionSurface,
    granularity: SelectionGranularity,
    anchor_start: screen::TranscriptPoint,
    anchor_end: screen::TranscriptPoint,
}
#[derive(Debug, Clone, Copy)]
struct ClickSequence {
    surface: SelectionSurface,
    position: (u16, u16),
    time: Instant,
    count: u8,
}

struct UiState {
    id: SessionId,
    workspace: String,
    views: SessionViews,
    empty: ViewState,
    composer: Composer,
    pending: Option<PreparedSubmission>,
    switch_pending: bool,
    notice: String,
    sheet: Option<OwnedSheet>,
    panel: bool,
    panel_kind: PanelKind,
    panel_focus: bool,
    panel_scroll: usize,
    history_scroll: usize,
    completion: Vec<completion::Suggestion>,
    completion_selected: Option<usize>,
    completion_dismissed: bool,
    paths: StaticPathIndex,
    provenance: Vec<String>,
    exit_armed: Option<Instant>,
    last_metadata: Option<screen::RenderMetadata>,
    render_cache: Option<screen::RenderCache>,
    layout_dirty: bool,
    vertical_goal: Option<usize>,
    viewport: Rect,
    clipboard: Option<ClipboardService>,
    clipboard_generation: u64,
    pending_cut: Option<(u64, String, std::ops::Range<usize>)>,
    paste_target: Option<PasteTarget>,
    index: Option<WorkspaceFileIndex>,
    index_generation: u64,
    demo: DemoRuntime,
    demo_active: bool,
    gallery: DemoGallery,
    demo_model_label: Option<&'static str>,
    question_review: bool,
    question_scroll: usize,
    offline_ready: bool,
    inactive_composer: Composer,
    inactive_scroll: usize,
    profile_picker: Option<bool>,
    effort_picker: Option<Effort>,
    settings: Option<CliSettings>,
    inactive_settings: Option<CliSettings>,
    settings_open: bool,
    settings_worker: Option<SettingsService>,
    settings_generation: u64,
    settings_pending: bool,
    hover: Option<screen::HitTarget>,
    chip_preview: Option<u64>,
    selection_gesture: Option<SelectionGesture>,
    click_sequence: Option<ClickSequence>,
    selection_owner: Option<SelectionSurface>,
    transcript_selection: Option<screen::TranscriptSelection>,
    transcript_span: Option<screen::TranscriptSpanSelection>,
    provider: ProviderState,
    /// Words in the text being copied, for the confirmation.
    copy_words: usize,
    /// A confirmation notice and when it disappears, unless replaced meanwhile.
    transient_notice: Option<(Instant, String)>,
    /// The provider's model per effort, when connected.
    provider_models: Vec<(Effort, String)>,
    /// Providers skipped before the selection, with reasons.
    provider_skipped: Vec<String>,
    /// Short footer label of the live model (`vendor/name:tag` -> `name`).
    model_label: Option<String>,
    turn: Option<TurnUi>,
    /// Changes the running turn waits on, oldest first.
    reviews: VecDeque<polaris_ui_session::ChangeReview>,
    activity: String,
}
impl UiState {
    fn new(
        id: SessionId,
        workspace: String,
        initial: Option<String>,
        provenance: Vec<String>,
    ) -> Result<Self, CliError> {
        let empty = offline_view()?;
        let views = SessionViews::new(empty.model.clone(), empty.effort);
        let mut composer = Composer::new();
        if let Some(text) = initial {
            composer
                .insert(&text)
                .map_err(|e| CliError::Ui(e.to_string()))?
        }
        let gallery = DemoGallery::new();
        let demo_model_label = gallery.selected_model().map(|model| model.name);
        Ok(Self {
            id,
            workspace,
            views,
            empty,
            composer,
            pending: None,
            switch_pending: false,
            notice: String::new(),
            sheet: None,
            panel: false,
            panel_kind: PanelKind::System,
            panel_focus: false,
            panel_scroll: 0,
            history_scroll: 0,
            completion: Vec::new(),
            completion_selected: None,
            completion_dismissed: false,
            paths: StaticPathIndex::new(Vec::<String>::new()),
            provenance,
            exit_armed: None,
            last_metadata: None,
            render_cache: Some(screen::RenderCache::new()),
            layout_dirty: true,
            vertical_goal: None,
            viewport: Rect::new(0, 0, 80, 24),
            clipboard: None,
            clipboard_generation: 0,
            pending_cut: None,
            paste_target: None,
            index: None,
            index_generation: 0,
            demo: DemoRuntime::new().map_err(ui_error)?,
            demo_active: false,
            gallery,
            demo_model_label,
            question_review: false,
            question_scroll: 0,
            offline_ready: true,
            inactive_composer: Composer::new(),
            inactive_scroll: 0,
            profile_picker: None,
            effort_picker: None,
            settings: None,
            inactive_settings: None,
            settings_open: false,
            settings_worker: None,
            settings_generation: 0,
            settings_pending: false,
            hover: None,
            chip_preview: None,
            selection_gesture: None,
            click_sequence: None,
            selection_owner: None,
            transcript_selection: None,
            transcript_span: None,
            provider: ProviderState::Connecting,
            copy_words: 0,
            transient_notice: None,
            provider_models: Vec::new(),
            provider_skipped: Vec::new(),
            model_label: None,
            turn: None,
            reviews: VecDeque::new(),
            activity: String::new(),
        })
    }
    /// Profile label beside the model control.
    fn profile_label(&self) -> &'static str {
        if self.demo_active {
            return "demo";
        }
        match &self.provider {
            ProviderState::Ready { provider } => provider,
            ProviderState::Connecting => "connecting",
            ProviderState::Unavailable(_) => "offline",
        }
    }
    /// Show `model` and `effort` in every live view.
    fn show_model(&mut self, model: &str, effort: Effort) {
        let last = model.rsplit('/').next().unwrap_or(model);
        self.model_label = Some(last.split(':').next().unwrap_or(last).to_owned());
        if let Ok(model) = ModelId::new(model) {
            self.empty.set_model(model.clone());
            self.empty.set_effort(effort);
            if let Some(view) = self.views.get_mut(self.id) {
                view.set_model(model);
                view.set_effort(effort);
            }
        }
    }
    /// Whether model and effort changes go to a connected provider.
    fn live_model(&self) -> bool {
        !self.demo_active && matches!(self.provider, ProviderState::Ready { .. })
    }
    /// One line describing the model connection, for status sheets and panels.
    fn provider_line(&self) -> String {
        match &self.provider {
            ProviderState::Ready { provider } => {
                format!("Model {} via {provider}", self.view().model)
            }
            ProviderState::Connecting => "Connecting to a model provider…".to_owned(),
            ProviderState::Unavailable(reason) => format!("No model provider: {reason}"),
        }
    }
    /// Append streamed text to the running turn's entry; capacity limits only drop text.
    fn stream_text(&mut self, delta: &str) {
        let Some(turn) = self.turn.as_mut() else {
            return;
        };
        turn.phase = TurnPhase::Writing;
        let separator = if turn.break_pending && turn.entry.is_some() {
            "\n\n"
        } else {
            ""
        };
        turn.break_pending = false;
        let entry = turn.entry;
        let view = self.offline_view_mut();
        let result = match entry {
            Some(index) => view
                .append_to_entry(index, separator)
                .and_then(|()| view.append_to_entry(index, delta)),
            None => {
                let index = view.conversation.len();
                view.push_assistant(delta.to_owned()).map(|()| {
                    if let Some(turn) = self.turn.as_mut() {
                        turn.entry = Some(index);
                    }
                })
            }
        };
        if let Err(error) = result {
            self.notice = format!("Reply display limit reached: {error}");
        }
    }
    /// Close the running turn with its final answer and a short work summary.
    /// Show a change the turn waits on: its prompt (with the diff) and how to decide.
    fn show_review(&mut self, review: polaris_ui_session::ChangeReview) {
        if self
            .reviews
            .iter()
            .any(|known| known.request_id == review.request_id)
        {
            return;
        }
        if let Some(turn) = self.turn.as_mut() {
            turn.entry = None;
            turn.break_pending = false;
            turn.waiting = true;
            turn.after_review = true;
        }
        let view = self.offline_view_mut();
        let shown = view.push_assistant(review.prompt.clone()).and_then(|()| {
            view.push_system(
                "y apply · n don't apply · or type a note and press Enter to decline with it",
            )
        });
        if let Err(error) = shown {
            self.notice = format!("Reply display limit reached: {error}");
        }
        self.reviews.push_back(review);
        self.history_scroll = 0;
    }
    fn close_review(&mut self, id: polaris_sdk::human::HumanRequestId) {
        self.reviews.retain(|review| review.request_id != id);
        let waiting = !self.reviews.is_empty();
        if let Some(turn) = self.turn.as_mut() {
            turn.waiting = waiting;
        }
    }
    fn finish_turn(&mut self, report: TurnReport) {
        let (streamed, after_review) = self.turn.take().map_or((false, false), |turn| {
            (turn.entry.is_some(), turn.after_review)
        });
        self.reviews.clear();
        let view = self.offline_view_mut();
        let mut result = Ok(());
        let answer = report
            .answer
            .as_deref()
            .map(str::trim)
            .filter(|answer| !answer.is_empty());
        if !streamed {
            result = match answer {
                Some(answer) => view.push_assistant(answer.to_owned()),
                None if after_review => Ok(()),
                None => view.push_warning(format!(
                    "The model stopped without an answer ({})",
                    report.stop
                )),
            };
        }
        let summary = report.summary();
        result = result.and_then(|()| view.push_system(summary));
        if let Err(error) = result {
            self.notice = format!("Reply display limit reached: {error}");
        }
        self.activity.clear();
        self.history_scroll = 0;
    }
    fn view(&self) -> &ViewState {
        if self.demo_active {
            return self.demo.view();
        }
        match self.views.get(self.id) {
            Some(v) => v,
            None => &self.empty,
        }
    }
    fn view_mut(&mut self) -> &mut ViewState {
        if self.demo_active {
            return self.demo.view_mut();
        }
        self.offline_view_mut()
    }
    fn offline_view_mut(&mut self) -> &mut ViewState {
        match self.views.get_mut(self.id) {
            Some(v) => v,
            None => &mut self.empty,
        }
    }
    fn switch_profile(&mut self, demo: bool) -> Result<(), CliError> {
        if self.demo_active == demo {
            self.profile_picker = None;
            self.sheet = None;
            return Ok(());
        }
        if self.pending.is_some() || self.settings_pending {
            self.notice = "Wait for the pending operation before switching profiles".to_owned();
            return Ok(());
        }
        if self.demo_active {
            self.demo.cancel().map_err(ui_error)?;
        }
        std::mem::swap(&mut self.composer, &mut self.inactive_composer);
        std::mem::swap(&mut self.history_scroll, &mut self.inactive_scroll);
        if self.inactive_settings.is_none() {
            self.inactive_settings = self.settings.clone();
        }
        std::mem::swap(&mut self.settings, &mut self.inactive_settings);
        if let Some(settings) = self.settings.as_mut() {
            settings.set_persistence(if demo {
                PersistencePolicy::MemoryOnly
            } else {
                PersistencePolicy::Global
            });
        }
        self.demo_active = demo;
        self.gallery.open(GalleryPage::Welcome);
        self.question_review = false;
        self.question_scroll = 0;
        self.profile_picker = None;
        self.settings_open = false;
        self.sheet = None;
        self.chip_preview = None;
        self.hover = None;
        self.selection_gesture = None;
        self.click_sequence = None;
        self.selection_owner = None;
        self.transcript_selection = None;
        self.transcript_span = None;
        self.vertical_goal = None;
        self.last_metadata = None;
        self.clipboard_generation = self
            .clipboard_generation
            .checked_add(1)
            .ok_or(CliError::Unavailable("clipboard generation exhausted"))?;
        if let Some(clipboard) = self.clipboard.as_mut() {
            clipboard.cancel(self.clipboard_generation);
        }
        self.pending_cut = None;
        self.paste_target = None;
        self.notice.clear();
        self.completion_dismissed = false;
        self.refresh_completion();
        Ok(())
    }
    fn refresh_completion(&mut self) {
        self.completion = if self.sheet.is_some()
            || self.gallery.page() != GalleryPage::Welcome
            || question_focused(self)
            || self.completion_dismissed
        {
            Vec::new()
        } else {
            completion::complete(self.composer.text(), self.composer.cursor(), &self.paths)
        };
        if self.sheet.is_none()
            && self.gallery.page() == GalleryPage::Welcome
            && !question_focused(self)
            && !self.completion_dismissed
            && let Some(token) =
                completion::detect_token(self.composer.text(), self.composer.cursor())
            && token.kind == completion::CompletionKind::Command
        {
            for (name, _page, description) in LOCAL_VIEW_COMMANDS {
                if name.trim_start_matches('/').starts_with(token.query) {
                    if let Some(existing) =
                        self.completion.iter_mut().find(|item| item.insert == *name)
                    {
                        if self.demo_active {
                            existing.description = description;
                        }
                    } else if *name == "/preview" {
                        self.completion.push(completion::Suggestion {
                            label: (*name).to_owned(),
                            insert: (*name).to_owned(),
                            description,
                            kind: completion::CompletionKind::Command,
                            range: token.start..token.end,
                        });
                    }
                }
            }
            if self.demo_active && "demo".starts_with(token.query) {
                self.completion.push(completion::Suggestion {
                    label: "/demo".to_owned(),
                    insert: "/demo".to_owned(),
                    description: "Demo gallery; /demo reset, replay, cancel, long or questions",
                    kind: completion::CompletionKind::Command,
                    range: token.start..token.end,
                });
            }
        }
        if self
            .completion_selected
            .is_some_and(|i| i >= self.completion.len())
        {
            self.completion_selected = None
        }
    }
    fn receive(&mut self, message: BackendMessage) -> Result<(), CliError> {
        self.layout_dirty = true;
        self.hover = None;
        match message {
            BackendMessage::OfflineReady => {
                self.offline_ready = true;
                self.switch_pending = false;
                self.switch_profile(false)?;
            }
            BackendMessage::Record(item) => {
                if let Some(text) = item.text.as_ref()
                    && item.record.session() == self.id
                {
                    match self.views.get(self.id) {
                        Some(view) => view,
                        None => &self.empty,
                    }
                    .check_user_capacity(text)
                    .map_err(ui_error)?;
                }
                self.views
                    .apply_selected(&item.record, self.id)
                    .map_err(|e| CliError::Ui(format!("projection requires recovery: {e}")))?;
                if item.record.session() == self.id
                    && let Some(text) = item.text
                {
                    self.offline_view_mut().push_user(text).map_err(ui_error)?;
                }
            }
            BackendMessage::Committed => {
                if let Some(prepared) = self.pending.take() {
                    self.composer
                        .commit_submit(prepared)
                        .map_err(|e| CliError::Ui(e.to_string()))?;
                }
                self.notice = "Directive committed by the kernel".to_owned();
                self.completion_dismissed = false;
                self.history_scroll = 0;
            }
            BackendMessage::QueryFailed(reason) => {
                self.notice = format!("Query failed: {reason}");
            }
            BackendMessage::ResumeRejected(reason) => {
                self.switch_pending = false;
                self.notice = format!("Resume rejected: {reason}");
            }
            BackendMessage::Rejected(reason) => {
                if let Some(prepared) = self.pending.take() {
                    self.composer.cancel_submit(prepared)
                }
                self.notice = format!("Rejected: {reason}");
            }
            BackendMessage::Sheet { title, lines } => {
                self.sheet = Some(OwnedSheet {
                    title,
                    lines,
                    scroll: 0,
                })
            }
            BackendMessage::ProviderReady {
                provider,
                model,
                effort,
                models,
                skipped,
            } => {
                self.provider = ProviderState::Ready { provider };
                self.provider_models = models;
                self.provider_skipped = skipped.clone();
                self.show_model(&model, effort);
                if !skipped.is_empty() {
                    self.notice = format!("Using {provider}; skipped {}", skipped.join(", "));
                }
            }
            BackendMessage::ModelSwitched { model, effort } => {
                self.show_model(&model, effort);
                self.notice = format!("Model {model} · {} effort", effort.name());
            }
            BackendMessage::ProviderUnavailable(reason) => {
                self.provider_skipped = reason.split("; ").map(str::to_owned).collect();
                self.notice = format!("No model provider ({reason}); prompts are not sent");
                self.provider = ProviderState::Unavailable(reason);
            }
            BackendMessage::PromptAccepted => {
                if let Some(prepared) = self.pending.take() {
                    self.composer
                        .commit_submit(prepared)
                        .map_err(|e| CliError::Ui(e.to_string()))?;
                }
                self.notice.clear();
                self.completion_dismissed = false;
                self.history_scroll = 0;
                self.turn = Some(TurnUi::new());
            }
            BackendMessage::TurnProgress(item) => match item {
                ChatProgress::Text(delta) => self.stream_text(&delta),
                ChatProgress::Reasoning(_) => {
                    if let Some(turn) = self.turn.as_mut()
                        && turn.phase != TurnPhase::Writing
                    {
                        turn.phase = TurnPhase::Thinking;
                    }
                }
                ChatProgress::Request => {
                    if let Some(turn) = self.turn.as_mut() {
                        turn.break_pending = turn.entry.is_some();
                        turn.phase = TurnPhase::Waiting;
                    }
                }
                ChatProgress::Review(review) => self.show_review(review),
                ChatProgress::ReviewClosed(id) => self.close_review(id),
                ChatProgress::Applied(path) => {
                    if let Err(error) = self
                        .offline_view_mut()
                        .push_system(format!("✓ Edited {path}"))
                    {
                        self.notice = format!("Reply display limit reached: {error}");
                    }
                }
                ChatProgress::Deleted(path) => {
                    if let Err(error) = self
                        .offline_view_mut()
                        .push_system(format!("✓ Deleted {path}"))
                    {
                        self.notice = format!("Reply display limit reached: {error}");
                    }
                }
            },
            BackendMessage::TurnFinished(report) => self.finish_turn(report),
            BackendMessage::TurnFailed(reason) => {
                self.turn = None;
                self.reviews.clear();
                self.activity.clear();
                let view = self.offline_view_mut();
                let shown = if reason == polaris_ui_session::host::INTERRUPTED {
                    view.push_warning("Interrupted")
                } else {
                    view.push_error(reason)
                };
                if let Err(error) = shown {
                    self.notice = format!("Reply display limit reached: {error}");
                }
            }
            BackendMessage::Resumed { id, workspace } => {
                self.clipboard_generation = self
                    .clipboard_generation
                    .checked_add(1)
                    .ok_or(CliError::Unavailable("clipboard generation exhausted"))?;
                if let Some(worker) = self.clipboard.as_mut() {
                    worker.cancel(self.clipboard_generation);
                }
                self.paste_target = None;
                self.pending_cut = None;
                self.transcript_selection = None;
                self.switch_pending = false;
                self.id = id;
                self.workspace = workspace;
                self.views = SessionViews::new(self.empty.model.clone(), self.empty.effort);
                self.history_scroll = 0;
                self.sheet = None;
                self.notice = "Resumed durable session; replaying committed records".to_owned();
            }
        }
        self.refresh_completion();
        Ok(())
    }
    fn panel_lines(&self) -> Vec<String> {
        let view = self.view();
        match self.panel_kind {
            PanelKind::Tasks => vec![
                match &self.turn {
                    Some(turn) => format!("Running: {}", turn.label()),
                    None => "Idle - type a request to start a turn.".to_owned(),
                },
                format!("Model requests this session: {}", view.hud.requests),
                format!("Tool calls this session: {}", view.tools.len()),
            ],
            PanelKind::Agents => vec![
                format!(
                    "Main · {} · {}",
                    view.model,
                    if self.turn.is_some() {
                        "working"
                    } else {
                        "idle"
                    }
                ),
                format!("Session {}", self.id),
                "Delegated child agents are not started by read-only conversations.".to_owned(),
            ],
            PanelKind::Questions => vec![
                "No pending questions.".to_owned(),
                "Read-only conversations answer directly instead of asking.".to_owned(),
            ],
            PanelKind::Review => vec![
                format!("Approved diffs observed: {}", view.status.diffs_approved),
                format!("Rejected diffs observed: {}", view.status.diffs_rejected),
                "Read-only conversations propose patches in the reply; nothing is applied."
                    .to_owned(),
            ],
            PanelKind::Proof => vec![
                match view.status.proof_of_done {
                    Some(result) => format!("Observed proof result: {result:?}"),
                    None => "No proof result has been observed.".to_owned(),
                },
                "Model prose cannot mark this session completed.".to_owned(),
            ],
            PanelKind::Context => vec![
                "Durable source is retained in local CAS/eventlog.".to_owned(),
                format!("Loaded conversation entries: {}", view.conversation.len()),
                format!("Last prompt size: {} tokens", view.hud.last_prompt_tokens),
            ],
            PanelKind::System => vec![
                format!("Session {}", self.id),
                format!("Workspace {}", self.workspace),
                format!(
                    "Applied directives observed: {}",
                    view.status.directives_applied
                ),
                format!(
                    "Worker restarts observed: {:?}",
                    view.status.worker_restarts
                ),
                self.provider_line(),
            ],
        }
    }
}

fn ui_error(error: impl std::fmt::Display) -> CliError {
    CliError::Ui(error.to_string())
}
fn request_paste(state: &mut UiState) -> Result<(), CliError> {
    let Some(worker) = state.clipboard.as_mut() else {
        state.notice =
            "Native clipboard adapter is unavailable; use the terminal's paste action".to_owned();
        return Ok(());
    };
    state.clipboard_generation = state
        .clipboard_generation
        .checked_add(1)
        .ok_or(CliError::Unavailable("clipboard generation exhausted"))?;
    state.pending_cut = None;
    state.paste_target = Some(PasteTarget {
        session: state.id,
        demo: state.demo_active,
        revision: state.composer.revision(),
    });
    match worker.request_read(state.clipboard_generation) {
        Ok(()) => state.notice = "Reading text clipboard…".to_owned(),
        Err(error) => {
            state.paste_target = None;
            state.notice = format!("Clipboard: {error}");
        }
    }
    Ok(())
}
fn request_copy(state: &mut UiState, cut: bool) -> Result<(), CliError> {
    let Some(text) = state.composer.selected_text() else {
        return Ok(());
    };
    let pending = if cut {
        state
            .composer
            .selection()
            .map(|range| (state.composer.text().to_owned(), range))
    } else {
        None
    };
    request_copy_text(state, text)?;
    if let Some((draft, range)) = pending
        && state.notice == "Copying text…"
    {
        state.pending_cut = Some((state.clipboard_generation, draft, range));
    }
    Ok(())
}
fn request_copy_text(state: &mut UiState, text: String) -> Result<(), CliError> {
    let Some(worker) = state.clipboard.as_mut() else {
        state.notice =
            "Native clipboard adapter is unavailable; use the terminal's copy action".to_owned();
        return Ok(());
    };
    state.clipboard_generation = state
        .clipboard_generation
        .checked_add(1)
        .ok_or(CliError::Unavailable("clipboard generation exhausted"))?;
    state.pending_cut = None;
    state.paste_target = None;
    state.copy_words = text.split_whitespace().count();
    state.notice = match worker.request_copy(state.clipboard_generation, text) {
        Ok(()) => "Copying text…".to_owned(),
        Err(error) => format!("Clipboard: {error}"),
    };
    Ok(())
}
fn receive_clipboard_result(state: &mut UiState, result: ClipboardResult) -> bool {
    state.layout_dirty = true;
    match result {
        ClipboardResult::TextRead { generation, result }
            if generation == state.clipboard_generation =>
        {
            let target = state.paste_target.take();
            let current = PasteTarget {
                session: state.id,
                demo: state.demo_active,
                revision: state.composer.revision(),
            };
            if target != Some(current) {
                state.notice = "Clipboard paste cancelled: draft or cursor changed".to_owned();

                return true;
            }
            match result {
                Ok(text)
                    if state.sheet.is_none()
                        && !gallery_open(state)
                        && !question_focused(state)
                        && !state.panel_focus
                        && state.pending.is_none()
                        && !state.switch_pending =>
                {
                    state.notice = match state.composer.insert(&text) {
                        Ok(()) => String::new(),
                        Err(error) => format!("Paste rejected: {error}"),
                    };
                    state.vertical_goal = None;
                    state.completion_dismissed = false;
                    state.refresh_completion();
                }
                Ok(_) => {
                    state.notice = "Clipboard paste cancelled: editing target changed".to_owned()
                }
                Err(error) => state.notice = format!("Clipboard: {error}"),
            }
        }
        ClipboardResult::TextCopied { generation, result }
            if generation == state.clipboard_generation =>
        {
            match result {
                Ok(()) => {
                    state.notice = match state.copy_words {
                        1 => "Copied 1 word".to_owned(),
                        words => format!("Copied {words} words"),
                    };
                    state.transient_notice = Some((
                        Instant::now() + Duration::from_secs(2),
                        state.notice.clone(),
                    ));
                    if let Some((pending_generation, draft, range)) = state.pending_cut.take() {
                        if pending_generation == generation
                            && state.composer.text() == draft
                            && state.composer.selection() == Some(range)
                        {
                            let _cut = state.composer.cut();
                            state.refresh_completion();
                        } else {
                            state.notice =
                                "Text copied; draft changed, so nothing was cut".to_owned();
                        }
                    }
                }
                Err(error) => {
                    state.pending_cut = None;
                    state.notice = format!("Clipboard: {error}; draft retained");
                }
            }
        }
        _ => return false,
    }
    state.hover = None;
    true
}
fn poll_services(state: &mut UiState) -> Result<bool, CliError> {
    let mut changed = false;
    for _ in 0..8 {
        let result = state
            .clipboard
            .as_ref()
            .and_then(ClipboardService::try_recv);
        let Some(result) = result else { break };
        changed |= receive_clipboard_result(state, result);
    }
    if let Some(result) = state.index.as_ref().and_then(WorkspaceFileIndex::try_recv)
        && result.generation == state.index_generation
    {
        match result.result {
            Ok(snapshot) => {
                state.paths = StaticPathIndex::new(snapshot.paths);
                state.refresh_completion();
                if snapshot.truncated {
                    state.notice = "File suggestions use a bounded workspace snapshot".to_owned();
                }
            }
            Err(error) => state.notice = format!("File index: {error}"),
        }
        changed = true;
    }
    if let Some(result) = state
        .settings_worker
        .as_ref()
        .and_then(SettingsService::try_recv)
        && result.generation == state.settings_generation
    {
        state.settings_pending = false;
        match result.result {
            Ok(settings) => {
                state.settings = Some(settings);
                state.notice = "Selected CLI preference saved to global config".to_owned();
            }
            Err(reason) => state.notice = reason,
        }
        if state.settings_open {
            refresh_settings_sheet(state);
        }
        changed = true;
    }
    if changed {
        state.layout_dirty = true;
        state.hover = None;
    }
    Ok(changed)
}
fn open_profile_picker(state: &mut UiState) {
    state.profile_picker = Some(state.demo_active);
    refresh_profile_picker(state);
}
fn open_effort_picker(state: &mut UiState) {
    state.effort_picker = Some(state.view().effort);
    refresh_effort_picker(state);
}
fn refresh_effort_picker(state: &mut UiState) {
    let live = state.live_model();
    let mut lines: Vec<String> = Effort::ALL
        .iter()
        .map(|effort| {
            let marker = if state.effort_picker == Some(*effort) {
                ">"
            } else {
                " "
            };
            let model = state
                .provider_models
                .iter()
                .find(|(level, _)| level == effort)
                .map(|(_, model)| model.as_str());
            match (live, model) {
                (true, Some(model)) => format!("{marker} {:<7} {model}", effort.name()),
                _ => format!("{marker} {}", effort.name()),
            }
        })
        .collect();
    lines.push(if live {
        "The conversation continues on the chosen model.".to_owned()
    } else {
        "This is a local preference; no provider capability is implied.".to_owned()
    });
    open_sheet(state, if live { "Model" } else { "Effort" }, lines);
}
/// Apply an effort choice: through the provider when connected, locally otherwise.
fn choose_effort(state: &mut UiState, effort: Effort, commands: &mpsc::Sender<BackendRequest>) {
    if state.live_model() {
        match queue_request(commands, BackendRequest::SetEffort(effort)) {
            Ok(()) => state.notice = "Switching model…".to_owned(),
            Err(error) => state.notice = error.to_string(),
        }
    } else {
        state.view_mut().set_effort(effort);
        state.notice = "Local effort preference changed".to_owned();
    }
}

fn apply_profile_choice(
    state: &mut UiState,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<(), CliError> {
    let demo = state.profile_picker == Some(true);
    if state.pending.is_some() || state.settings_pending {
        state.notice = "Wait for pending operations before switching".to_owned();
    } else if !demo && !state.offline_ready {
        match queue_request(commands, BackendRequest::StartOffline) {
            Ok(()) => {
                state.switch_pending = true;
                state.notice = "Starting offline local controls…".to_owned();
            }
            Err(error) => state.notice = error.to_string(),
        }
    } else {
        state.switch_profile(demo)?;
    }
    Ok(())
}

fn refresh_profile_picker(state: &mut UiState) {
    let selected = state.profile_picker == Some(true);
    open_sheet(
        state,
        "Profile",
        vec![
            format!(
                "{} Live · local session, replies from the provider chain",
                if !selected { ">" } else { " " }
            ),
            format!(
                "{} Demo · scripted UI, isolated drafts; no session writes",
                if selected { ">" } else { " " }
            ),
            String::new(),
            "Up/Down choose · Enter switch · Esc close".to_owned(),
        ],
    );
}
fn refresh_settings_sheet(state: &mut UiState) {
    let Some(settings) = state.settings.as_ref() else {
        return;
    };
    let mut lines: Vec<String> = settings
        .rows()
        .iter()
        .map(|row| {
            format!(
                "{} {}: {}",
                if row.selected { ">" } else { " " },
                row.label,
                row.value
            )
        })
        .collect();
    lines.push(String::new());
    lines.push(
        if state.demo_active {
            "DEMO: changes stay in memory"
        } else {
            "Enter saves only the selected field to global config"
        }
        .to_owned(),
    );
    lines.push("Up/Down choose · Left/Right/Space change · Esc close".to_owned());
    open_sheet(state, "CLI settings", lines);
}
fn open_settings(state: &mut UiState) {
    if state.settings.is_none() {
        state.notice = "CLI settings are not attached to this controller".to_owned();
        return;
    }
    state.settings_open = true;
    refresh_settings_sheet(state);
}
fn refresh_demo_question(state: &mut UiState) {
    if state.demo.question().is_none() {
        state.question_review = false;
    }
    if question_focused(state) {
        state.completion.clear();
        state.completion_selected = None;
        state.completion_dismissed = true;
    }
}
fn question_focused(state: &UiState) -> bool {
    state.demo_active
        && state.demo.question().is_some()
        && state.sheet.is_none()
        && state.gallery.page() == GalleryPage::Welcome
}
fn navigate_question(state: &mut UiState, index: usize) {
    if let Some(question) = state.demo.question_mut() {
        if index >= question.spec().questions.len() {
            state.question_review = true;
        } else {
            let _selected = question.select_question(index);
            state.question_review = false;
        }
    }
}
fn handle_question_key(state: &mut UiState, key: KeyEvent) -> Result<bool, CliError> {
    use polaris_ui_model::questionnaire::{
        AnswerSubmitError, QuestionKind, QuestionnaireAction as A, QuestionnaireFocus,
    };
    if !question_focused(state) {
        return Ok(false);
    }
    if key.code == KeyCode::Enter && key.modifiers.is_empty() {
        if !state.question_review {
            state.question_review = true;
        } else {
            match state.demo.accept_answer() {
                Ok(batch) => {
                    state.question_review = false;
                    state.notice = format!(
                        "DEMO: {} answers accepted locally; nothing sent",
                        batch.answers.len()
                    );
                    state.completion_dismissed = false;
                }
                Err(crate::demo::DemoError::Answer(AnswerSubmitError::Incomplete(error))) => {
                    navigate_question(state, error.question_index);
                    state.notice = "Choose an answer for this required question".to_owned();
                }
                Err(error) => state.notice = format!("DEMO answer rejected: {error}"),
            }
        }
        refresh_demo_question(state);
        return Ok(true);
    }
    if state.question_review {
        match key.code {
            KeyCode::Up => state.question_scroll = state.question_scroll.saturating_sub(1),
            KeyCode::Down => {
                state.question_scroll = state.question_scroll.saturating_add(1).min(32768)
            }
            KeyCode::PageUp => state.question_scroll = state.question_scroll.saturating_sub(6),
            KeyCode::PageDown => {
                state.question_scroll = state.question_scroll.saturating_add(6).min(32768)
            }
            _ => {}
        }
        if matches!(key.code, KeyCode::Left | KeyCode::BackTab) {
            let last = state
                .demo
                .question()
                .map_or(0, |q| q.spec().questions.len().saturating_sub(1));
            navigate_question(state, last);
        }
        return Ok(true);
    }
    let viewport = state.viewport;
    let editor_geometry = if matches!(key.code, KeyCode::Up | KeyCode::Down) {
        current_metadata(state, &RenderOptions::default(), viewport).questionnaire_editor
    } else {
        None
    };
    let Some(question) = state.demo.question_mut() else {
        return Ok(false);
    };
    let editing = question.focus() == QuestionnaireFocus::Notes
        || question
            .current_question()
            .is_some_and(|q| q.kind == QuestionKind::FreeText);
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let action = match key.code {
        KeyCode::Tab | KeyCode::BackTab => Some(if question.focus() == QuestionnaireFocus::Notes {
            A::FocusChoices
        } else {
            A::FocusNotes
        }),
        KeyCode::Left if !editing => {
            question.navigate(-1);
            None
        }
        KeyCode::Right if !editing => {
            if question.question_index().saturating_add(1) == question.spec().questions.len() {
                state.question_review = true;
            } else {
                question.navigate(1);
            }
            None
        }
        KeyCode::Up if !editing => Some(A::ChoiceUp),
        KeyCode::Down if !editing => Some(A::ChoiceDown),
        KeyCode::Char(' ') if !editing => Some(A::ToggleChoice),
        KeyCode::Up | KeyCode::Down if editing => {
            if let (Some(field), Some(editor)) = (editor_geometry, question.active_editor()) {
                let delta = if key.code == KeyCode::Up { -1 } else { 1 };
                let (at, _) = field.geometry.vertical_target(editor.cursor(), delta, None);
                Some(A::SetCursor(at))
            } else {
                None
            }
        }
        KeyCode::Backspace if editing => Some(A::Backspace),
        KeyCode::Delete if editing => Some(A::Delete),
        KeyCode::Left if editing => Some(A::MoveLeft {
            extend: shift,
            word: control,
        }),
        KeyCode::Right if editing => Some(A::MoveRight {
            extend: shift,
            word: control,
        }),
        KeyCode::Home if editing => Some(A::Home { extend: shift }),
        KeyCode::End if editing => Some(A::End { extend: shift }),
        KeyCode::Char('a' | 'A') if control && editing => Some(A::SelectAll),
        KeyCode::Char('z' | 'Z') if control && editing => {
            Some(if shift { A::Redo } else { A::Undo })
        }
        KeyCode::Char('y' | 'Y') if control && editing => Some(A::Redo),
        KeyCode::Enter if editing && (shift || key.modifiers.contains(KeyModifiers::ALT)) => {
            Some(A::InsertText("\n".to_owned()))
        }
        KeyCode::Char(ch)
            if editing
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            Some(A::InsertText(ch.to_string()))
        }
        _ => None,
    };
    if let Some(action) = action {
        if let Err(error) = question.apply(action) {
            state.notice = format!("Question edit rejected: {error}");
        } else {
            state.notice.clear();
        }
    }
    refresh_demo_question(state);
    Ok(true)
}

/// Observed usage of this session, from committed model request records.
fn usage_lines(state: &UiState) -> Vec<String> {
    let view = state.view();
    let hud = &view.hud;
    let mut lines = vec![
        state.provider_line(),
        format!("Model requests      {}", hud.requests),
        format!("Prompt tokens       {}", hud.prompt_tokens),
        format!("Completion tokens   {}", hud.completion_tokens),
        format!(
            "Cached prompt       {}{}",
            hud.cached_tokens,
            hud.cache_hit_permille()
                .map(|permille| format!(" ({}.{}%)", permille / 10, permille % 10))
                .unwrap_or_default()
        ),
        format!("Last prompt         {} tokens", hud.last_prompt_tokens),
    ];
    if let Some(latency) = hud.last_s2_latency_us {
        lines.push(format!(
            "Last reply          {}",
            polaris_ui_model::view::format_duration_ms(latency / 1000)
        ));
    }
    if hud.cost_micro_usd > 0 {
        lines.push(format!(
            "Reported cost       ${}.{:06}",
            hud.cost_micro_usd / 1_000_000,
            hud.cost_micro_usd % 1_000_000
        ));
    }
    lines.push(format!(
        "Directives applied  {}",
        view.status.directives_applied
    ));
    lines.push(
        "Counts come from committed records of this session; provider quotas are not queried."
            .to_owned(),
    );
    lines
}
/// Provider chain state: the selection, its models per effort and every skip reason.
fn provider_lines(state: &UiState) -> Vec<String> {
    let mut lines = vec![state.provider_line()];
    if !state.provider_models.is_empty() {
        lines.push(String::new());
        lines.push("Models by effort:".to_owned());
        for (effort, model) in &state.provider_models {
            let marker = if *effort == state.view().effort {
                ">"
            } else {
                " "
            };
            lines.push(format!("{marker} {:<7} {model}", effort.name()));
        }
    }
    if !state.provider_skipped.is_empty() {
        lines.push(String::new());
        lines.push("Skipped:".to_owned());
        lines.extend(
            state
                .provider_skipped
                .iter()
                .map(|skip| format!("  {skip}")),
        );
    }
    lines.push(String::new());
    lines.push("Order and models come from provider.order and the [nebius], [openrouter] and [local] tables in ~/.polaris/config.toml.".to_owned());
    lines
}
fn onboarding_lines() -> Vec<String> {
    [
        "1. Pick a provider. Polaris tries provider.order: token_factory, openrouter, local.",
        "2. Hosted keys: put NEBIUS_API_KEY or OPENROUTER_API_KEY in ~/.polaris/.env",
        "   (or pass --env-file). A repository .env is never read implicitly.",
        "3. Local models: run Ollama or llama.cpp on 127.0.0.1 and set local.model",
        "   in ~/.polaris/config.toml, for example local.model = \"qwen3:8b\".",
        "4. Optional System One: set system_one.decide_model_dir and system_one.ort_dylib",
        "   for semantic privacy and tool screening; without it masking stays deterministic.",
        "5. Ask about the workspace. Use @path to point at files and /model to switch.",
        "6. /status shows the session, /providers the chain, /usage the token counts.",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn queue_request(
    tx: &mpsc::Sender<BackendRequest>,
    request: BackendRequest,
) -> Result<(), CliError> {
    tx.try_send(request)
        .map_err(|_| CliError::Unavailable("local control queue is full or closed; draft retained"))
}
fn open_sheet(state: &mut UiState, title: &str, lines: Vec<String>) {
    state.sheet = Some(OwnedSheet {
        title: title.to_owned(),
        lines,
        scroll: 0,
    });
    state.completion_dismissed = true;
    state.refresh_completion()
}
/// Local navigation metadata reused by demo dispatch and completion.
const LOCAL_VIEW_COMMANDS: &[(&str, GalleryPage, &str)] = &[
    (
        "/preview",
        GalleryPage::Preview,
        "Browse all demo screens and states",
    ),
    (
        "/model",
        GalleryPage::Model,
        "Choose a demo model and effort",
    ),
    (
        "/usage",
        GalleryPage::Usage,
        "Inspect mock usage, context and limits",
    ),
    (
        "/stats",
        GalleryPage::Stats,
        "Inspect mock activity and model charts",
    ),
    (
        "/providers",
        GalleryPage::Providers,
        "Configure reference providers in demo memory",
    ),
    (
        "/sessions",
        GalleryPage::Sessions,
        "Browse demo session history",
    ),
    (
        "/diff",
        GalleryPage::Diff,
        "Review a demo patch without file changes",
    ),
    (
        "/onboarding",
        GalleryPage::Onboarding,
        "Explore the first-run walkthrough",
    ),
    (
        "/settings",
        GalleryPage::Settings,
        "Preview CLI appearance and interaction settings",
    ),
    (
        "/agents",
        GalleryPage::Agents,
        "Inspect demo agent conversations",
    ),
    (
        "/status",
        GalleryPage::Status,
        "Inspect the local demo state",
    ),
    (
        "/help",
        GalleryPage::Help,
        "Show demo commands and shortcuts",
    ),
];
fn local_view_page(command: &str) -> Option<GalleryPage> {
    LOCAL_VIEW_COMMANDS
        .iter()
        .find_map(|(name, page, _)| if *name == command { Some(*page) } else { None })
}
fn gallery_open(state: &UiState) -> bool {
    state.demo_active && state.gallery.page() != GalleryPage::Welcome
}
fn gallery_rect(state: &UiState, area: Rect) -> Rect {
    let preferred = match state.gallery.page() {
        GalleryPage::Model => 14,
        GalleryPage::ProviderEdit => 21,
        GalleryPage::Settings => area.height.saturating_sub(9).max(14),
        GalleryPage::Status => 17,
        GalleryPage::Usage
            if state.gallery.usage_details_visible() && area.width.saturating_sub(8) < 104 =>
        {
            40
        }
        GalleryPage::Usage => 30,
        GalleryPage::Stats => 32,
        GalleryPage::Transcript | GalleryPage::Diff => 24,
        GalleryPage::Help | GalleryPage::Preview => 23,
        _ => 18,
    };
    let height = preferred.min(area.height.saturating_sub(6));
    Rect::new(
        area.x.saturating_add(2.min(area.width)),
        area.bottom().saturating_sub(height.saturating_add(2)),
        area.width.saturating_sub(4),
        height,
    )
}
fn load_demo_scenario(state: &mut UiState, name: &str) -> Result<(), CliError> {
    state.selection_gesture = None;
    state.click_sequence = None;
    state.gallery.open(GalleryPage::Welcome);
    state.question_review = false;
    state.sheet = None;
    if let Some(page) = match name {
        "models" => Some(GalleryPage::Model),
        "providers" => Some(GalleryPage::Providers),
        "settings" => Some(GalleryPage::Settings),
        "sessions" => Some(GalleryPage::Sessions),
        "status" => Some(GalleryPage::Status),
        "usage" | "usage-demo" => Some(GalleryPage::Usage),
        "stats" | "stats-demo" | "stats-stacked" | "stats-lines" | "stats-trends"
        | "stats-details" => Some(GalleryPage::Stats),
        "diff" => Some(GalleryPage::Diff),
        "transcript" => Some(GalleryPage::Transcript),
        "approval" => Some(GalleryPage::Approval),
        "onboarding" => Some(GalleryPage::Onboarding),
        "help" => Some(GalleryPage::Help),
        "agents-working" | "agents-mixed" | "agents-completed" | "agents-many" => {
            Some(GalleryPage::Agents)
        }
        _ => None,
    } {
        state.gallery.open(page);
    } else {
        match name {
            "welcome" => {
                state.demo.reset().map_err(ui_error)?;
            }
            "questions" => {
                state.demo.start_questionnaire().map_err(ui_error)?;
            }
            "working" => {
                state.demo.reset().map_err(ui_error)?;
                state
                    .demo
                    .submit("Show the working demo")
                    .map_err(ui_error)?;
            }
            "conversation" | "messages" | "approval" | "error" | "editor" => {
                state.demo.load_fixture(name).map_err(ui_error)?;
                if name == "error" && state.composer.is_empty() {
                    state
                        .composer
                        .insert("Continue reviewing the validation helper")
                        .map_err(ui_error)?;
                }
            }
            _ => return Err(CliError::Ui(format!("Unknown demo scenario: {name}"))),
        }
    }
    let _recognized = state.gallery.load_scenario(name);
    state.notice.clear();
    state.history_scroll = 0;
    state.completion_dismissed = true;
    state.refresh_completion();
    Ok(())
}
fn apply_gallery_effect(state: &mut UiState, effect: GalleryEffect) -> Result<(), CliError> {
    match effect {
        GalleryEffect::Close => {
            state.gallery.open(GalleryPage::Welcome);
            state.completion_dismissed = false;
        }
        GalleryEffect::Consumed => {}
        GalleryEffect::ApprovalDecision(approved) => {
            state
                .view_mut()
                .push_system(if approved {
                    "[DEMO] Proposal accepted locally; no file changes or execution occurred"
                } else {
                    "[DEMO] Proposal rejected locally; no file changes or execution occurred"
                })
                .map_err(ui_error)?;
            state.gallery.open(GalleryPage::Welcome);
        }
        GalleryEffect::LoadScenario(name) => load_demo_scenario(state, name)?,
        GalleryEffect::ModelSelected(model) => {
            state.view_mut().model = ModelId::new(model.id).map_err(ui_error)?;
            state.demo_model_label = Some(model.name);
            state.view_mut().effort = model.effort;
            state.gallery.open(GalleryPage::Welcome);
            state.notice = format!("DEMO model: {} · {}", model.name, model.effort.name());
        }
        GalleryEffect::ProviderSaved(provider) => {
            state.notice = format!(
                "DEMO provider {} saved in memory; no connection made",
                provider.label
            );
            state.gallery.open(GalleryPage::Providers);
        }
        GalleryEffect::SettingSelected { key, value } => {
            if let Some(settings) = state.settings.as_mut() {
                let _changed = settings.apply_demo_choice(key, value);
            }
            state.notice = "DEMO preference applied in memory".to_owned();
        }
        GalleryEffect::ViewedConversation {
            scenario: _,
            title,
            owner,
        } => {
            state.demo.load_fixture("conversation").map_err(ui_error)?;
            state
                .view_mut()
                .push_system(format!(
                    "[DEMO] Viewing {owner}: {title}; prompts still target Main"
                ))
                .map_err(ui_error)?;
            state.gallery.open(GalleryPage::Welcome);
        }
    }
    state.refresh_completion();
    Ok(())
}
fn submit_draft(
    state: &mut UiState,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<bool, CliError> {
    if state.pending.is_some() || state.switch_pending {
        state.notice =
            "Waiting for the previous directive acknowledgement; draft retained".to_owned();
        return Ok(false);
    }
    if state.composer.is_empty() {
        return Ok(false);
    }
    let prepared = state.composer.prepare_submit().map_err(ui_error)?;
    if !prepared.submission().images.is_empty() {
        state.notice =
            "Attachment processing requires the model/ingest profile; draft retained".to_owned();
        state.composer.cancel_submit(prepared);
        return Ok(false);
    }
    let text = prepared.submission().text.trim();
    if state.demo_active {
        let demo_command = match text {
            "/demo reset" => Some(state.demo.reset()),
            "/demo replay" => Some(state.demo.replay()),
            "/demo cancel" => Some(state.demo.cancel()),
            _ => None,
        };

        if let Some(result) = demo_command {
            match result {
                Ok(()) => {
                    state.composer.commit_submit(prepared).map_err(ui_error)?;
                    state.sheet = None;
                    state.notice = "DEMO state updated locally".to_owned();
                }
                Err(error) => {
                    state.composer.cancel_submit(prepared);
                    state.notice = format!("DEMO: {error}");
                }
            }
            state.refresh_completion();
            return Ok(false);
        }
        if let Some(page) = local_view_page(text) {
            state.gallery.open(page);
            state.composer.commit_submit(prepared).map_err(ui_error)?;
            state.notice.clear();
            state.completion_dismissed = true;
            state.refresh_completion();
            return Ok(false);
        }
        if text.starts_with("/preview ") {
            let name = text.strip_prefix("/preview ").map_or("", str::trim);
            match load_demo_scenario(state, name) {
                Ok(()) => {
                    state.composer.commit_submit(prepared).map_err(ui_error)?;
                }
                Err(error) => {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                }
            }
            return Ok(false);
        }
        if text == "/questions" {
            state.demo.start_questionnaire().map_err(ui_error)?;
            state.composer.commit_submit(prepared).map_err(ui_error)?;
            state.question_review = false;
            refresh_demo_question(state);
            return Ok(false);
        }
        if text == "/demo" {
            state.gallery.open(GalleryPage::Preview);
            state.composer.commit_submit(prepared).map_err(ui_error)?;
            return Ok(false);
        }
        if matches!(text, "/demo questions" | "/demo long") {
            if text == "/demo questions" {
                state.demo.start_questionnaire().map_err(ui_error)?;
            } else {
                state.demo.submit("long demo").map_err(ui_error)?;
            }
            state.composer.commit_submit(prepared).map_err(ui_error)?;
            refresh_demo_question(state);
            return Ok(false);
        }
        if !text.starts_with('/') {
            match state.demo.submit(&prepared.submission().text) {
                Ok(()) => {
                    state.composer.commit_submit(prepared).map_err(ui_error)?;
                    state.notice =
                        "DEMO running · Esc cancels · /demo replay or /demo reset".to_owned();
                    state.history_scroll = 0;
                }
                Err(error) => {
                    state.composer.cancel_submit(prepared);
                    state.notice = format!("DEMO: {error}; draft retained");
                }
            }
            state.completion_dismissed = false;
            state.refresh_completion();
            return Ok(false);
        }
    }
    let parsed = match polaris_kernel::directives::parse(
        text,
        &polaris_kernel::directives::ParseLimits::default(),
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            state.notice = if text.starts_with('/') {
                format!("Unknown or invalid command: {error}")
            } else {
                format!("Invalid input: {error}")
            };
            state.composer.cancel_submit(prepared);
            return Ok(false);
        }
    };
    if let Some(command) = parsed.command {
        use polaris_kernel::directives::slash::{PanelTarget, SlashCommand};
        match command {
            SlashCommand::Quit => {
                state.composer.cancel_submit(prepared);
                return Ok(true);
            }
            SlashCommand::Help(_) => open_sheet(state, "Help", help_lines()),
            SlashCommand::Settings => open_settings(state),
            SlashCommand::Status => {
                let mut lines = vec![
                    format!("Session {}", state.id),
                    format!("Workspace {}", state.workspace),
                    state.provider_line(),
                    "Actual configuration source layers:".to_owned(),
                ];
                lines.extend(state.provenance.clone());
                lines.push("Use /settings to edit CLI preferences.".to_owned());
                open_sheet(state, "Status", lines)
            }
            SlashCommand::Stats => {
                if let Err(error) = queue_request(commands, BackendRequest::Stats) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
            }
            SlashCommand::Usage | SlashCommand::Quota => {
                let lines = usage_lines(state);
                open_sheet(state, "Usage", lines)
            }
            SlashCommand::Providers => {
                let lines = provider_lines(state);
                open_sheet(state, "Providers", lines)
            }
            SlashCommand::Onboarding => open_sheet(state, "Getting started", onboarding_lines()),
            SlashCommand::Diff | SlashCommand::Undo => {
                use polaris_ui_session::commands::LocalCommand;
                let local = if matches!(command, SlashCommand::Undo) {
                    LocalCommand::Undo
                } else {
                    LocalCommand::Diff
                };
                if let Err(error) = queue_request(commands, BackendRequest::Local(local)) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
            }
            SlashCommand::Rollback(_) => {
                state.notice =
                    "Rollback to a step is not available; /undo restores the newest applied change"
                        .to_owned();
            }
            SlashCommand::Sessions => {
                if let Err(error) = queue_request(commands, BackendRequest::Sessions) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
            }
            SlashCommand::SessionResume(id) => {
                if let Err(error) = queue_request(commands, BackendRequest::Resume(id)) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
                state.switch_pending = true;
                state.notice = "Waiting for session switch acknowledgement".to_owned();
            }
            SlashCommand::Clear => {
                if state.demo_active {
                    state.demo.reset().map_err(ui_error)?;
                    state.question_review = false;
                    state.gallery.open(GalleryPage::Welcome);
                }
                state.view_mut().conversation.clear();
                if !state.demo_active && state.turn.is_none() {
                    let _queued = queue_request(commands, BackendRequest::ClearChat);
                }
                state.transcript_selection = None;
                state.transcript_span = None;
                state.history_scroll = 0;
                state.notice.clear()
            }
            SlashCommand::Panel(target) => match target {
                Some(PanelTarget::Off) => {
                    state.panel = false;
                    state.panel_focus = false
                }
                Some(target) => {
                    state.panel_kind = match target {
                        PanelTarget::Tasks => PanelKind::Tasks,
                        PanelTarget::Agents => PanelKind::Agents,
                        PanelTarget::Questions => PanelKind::Questions,
                        PanelTarget::Review => PanelKind::Review,
                        PanelTarget::Proof => PanelKind::Proof,
                        PanelTarget::Context => PanelKind::Context,
                        PanelTarget::System => PanelKind::System,
                        PanelTarget::Off => PanelKind::System,
                    };
                    state.panel = true;
                    state.panel_scroll = 0;
                }
                None => state.panel = !state.panel,
            },
            SlashCommand::Agents => {
                state.panel_kind = PanelKind::Agents;
                state.panel = true;
                state.panel_scroll = 0
            }
            SlashCommand::Questions => {
                state.panel_kind = PanelKind::Questions;
                state.panel = true;
                state.panel_scroll = 0
            }
            SlashCommand::Model(argument) => match argument.as_deref().map(str::trim) {
                None | Some("") => open_effort_picker(state),
                Some(name) => {
                    let wanted = Effort::ALL
                        .iter()
                        .copied()
                        .find(|effort| effort.name().eq_ignore_ascii_case(name))
                        .or_else(|| {
                            state
                                .provider_models
                                .iter()
                                .find(|(_, model)| model == name)
                                .map(|(effort, _)| *effort)
                        });
                    match wanted {
                        Some(effort) => choose_effort(state, effort, commands),
                        None => {
                            state.notice =
                                format!("Unknown model or effort {name}; /model lists the choices");
                            state.composer.cancel_submit(prepared);
                            return Ok(false);
                        }
                    }
                }
            },
            SlashCommand::NvProfile
            | SlashCommand::NvVerify
            | SlashCommand::Hack
            | SlashCommand::Ci
            | SlashCommand::CiRun
            | SlashCommand::SearchLocal(_)
            | SlashCommand::SearchWeb(_)
            | SlashCommand::NvDocs(_)
            | SlashCommand::NvDocsSync
            | SlashCommand::Research(_) => {
                use polaris_ui_session::commands::LocalCommand;
                use polaris_ui_session::research::ResearchCommand;
                let local = match command {
                    SlashCommand::Research(research) => {
                        use polaris_kernel::directives::slash::ResearchCmd;
                        LocalCommand::Research(match research {
                            ResearchCmd::Start(goal) => {
                                state.notice =
                                    "Research run started; the report opens when it ends"
                                        .to_owned();
                                ResearchCommand::Start(goal)
                            }
                            ResearchCmd::Status => ResearchCommand::Status,
                            ResearchCmd::Report => ResearchCommand::Report,
                            ResearchCmd::Resume => {
                                state.notice =
                                    "Research run resumed; the report opens when it ends"
                                        .to_owned();
                                ResearchCommand::Resume
                            }
                        })
                    }
                    SlashCommand::NvProfile => LocalCommand::NvProfile,
                    SlashCommand::NvVerify => LocalCommand::NvVerify,
                    SlashCommand::Ci => LocalCommand::Ci,
                    SlashCommand::CiRun => {
                        state.notice = "Running the CI pipeline in Docker…".to_owned();
                        LocalCommand::CiRun
                    }
                    SlashCommand::SearchLocal(query) => LocalCommand::SearchLocal(query),
                    SlashCommand::SearchWeb(query) => LocalCommand::SearchWeb(query),
                    SlashCommand::NvDocs(query) => LocalCommand::NvDocs(query),
                    SlashCommand::NvDocsSync => {
                        state.notice = "Downloading the NVIDIA documentation seed…".to_owned();
                        LocalCommand::NvDocsSync
                    }
                    _ => LocalCommand::Hack,
                };
                if let Err(error) = queue_request(commands, BackendRequest::Local(local)) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
            }
            SlashCommand::Plan
            | SlashCommand::Goal(_)
            | SlashCommand::SessionReplay { .. }
            | SlashCommand::NebiusStatus
            | SlashCommand::ContreeBranches
            | SlashCommand::DatalabSync
            | SlashCommand::DatalabFinetune
            | SlashCommand::ConcurrencyStats
            | SlashCommand::CacheStats
            | SlashCommand::CachePurge
            | SlashCommand::ContextMap
            | SlashCommand::ContextExpand(_)
            | SlashCommand::ParallelDag
            | SlashCommand::Screenshot
            | SlashCommand::Browser(_)
            | SlashCommand::UiDiff
            | SlashCommand::Redteam
            | SlashCommand::NixFlake
            | SlashCommand::ProofOfDone => {
                state.notice = "This command requires its real backend profile and is unavailable here; draft retained".to_owned();
                state.composer.cancel_submit(prepared);
                return Ok(false);
            }
        }
        state.composer.commit_submit(prepared).map_err(ui_error)?;
        state.completion_dismissed = false;
        state.refresh_completion();
        return Ok(false);
    }
    if parsed.directives.len() == 1 && parsed.prose.trim().is_empty() && parsed.mentions.is_empty()
    {
        if let Err(error) = queue_request(
            commands,
            BackendRequest::Directive {
                session: state.id,
                text: prepared.submission().text.clone(),
            },
        ) {
            state.notice = error.to_string();
            state.composer.cancel_submit(prepared);
            return Ok(false);
        }
        state.pending = Some(prepared);
        state.notice = "Pending signed directive admission".to_owned();
        state.completion.clear();
    } else if state.turn.is_some() {
        state.composer.cancel_submit(prepared);
        state.notice = "A reply is still running; Esc interrupts it. Draft retained".to_owned();
    } else {
        match &state.provider {
            ProviderState::Ready { .. } => {
                if let Err(error) = queue_request(
                    commands,
                    BackendRequest::Prompt {
                        session: state.id,
                        text: prepared.submission().text.clone(),
                        plan: state.view().mode == polaris_ui_model::view::Mode::Plan,
                    },
                ) {
                    state.notice = error.to_string();
                    state.composer.cancel_submit(prepared);
                    return Ok(false);
                }
                state.pending = Some(prepared);
                state.notice = "Sending…".to_owned();
                state.completion.clear();
            }
            ProviderState::Connecting => {
                state.composer.cancel_submit(prepared);
                state.notice = "Still connecting to a model provider; draft retained".to_owned();
            }
            ProviderState::Unavailable(reason) => {
                state.notice = format!("No model provider ({reason}); draft retained");
                state.composer.cancel_submit(prepared);
            }
        }
    }
    Ok(false)
}
fn model_chord(key: KeyEvent) -> Option<keymap::KeyChord> {
    let code = match key.code {
        KeyCode::Enter => keymap::Key::Enter,
        KeyCode::Esc => keymap::Key::Esc,
        KeyCode::Tab | KeyCode::BackTab => keymap::Key::Tab,
        KeyCode::Up => keymap::Key::Up,
        KeyCode::Down => keymap::Key::Down,
        KeyCode::Left => keymap::Key::Left,
        KeyCode::Right => keymap::Key::Right,
        KeyCode::PageUp => keymap::Key::PageUp,
        KeyCode::PageDown => keymap::Key::PageDown,
        KeyCode::Insert => keymap::Key::Insert,
        KeyCode::F(n) => keymap::Key::F(n),
        KeyCode::Char(c) => keymap::Key::Char(c),
        _ => return None,
    };
    Some(keymap::KeyChord::new(
        code,
        keymap::Modifiers {
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
            alt: key.modifiers.contains(KeyModifiers::ALT),
            shift: key.modifiers.contains(KeyModifiers::SHIFT) || key.code == KeyCode::BackTab,
        },
    ))
}
fn composer_area(state: &UiState) -> Rect {
    let lines = state.panel_lines();
    screen::composer_rect(&projection(state, &lines, ""), state.viewport)
}
fn accept_completion(state: &mut UiState) -> Result<(), CliError> {
    if let Some(index) = state.completion_selected
        && let Some(s) = state.completion.get(index)
    {
        let range = s.range.clone();
        let insert = s.insert.clone();
        state
            .composer
            .apply_completion(range, &insert)
            .map_err(ui_error)?;
    }
    state.completion_dismissed = true;
    state.notice.clear();
    state.refresh_completion();
    Ok(())
}
/// While a change waits: `y` applies and `n` declines it when the composer is empty, and Enter
/// declines it with the composer text as the note for the model.
fn handle_review_key(
    state: &mut UiState,
    key: KeyEvent,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<bool, CliError> {
    if state.reviews.is_empty() || state.sheet.is_some() || state.panel_focus {
        return Ok(false);
    }
    let plain = key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT;
    let draft = state.composer.text().trim().to_owned();
    let decision = match key.code {
        KeyCode::Char('y' | 'Y') if plain && draft.is_empty() => (true, None),
        KeyCode::Char('n' | 'N') if plain && draft.is_empty() => (false, None),
        KeyCode::Enter
            if key.modifiers.is_empty() && !draft.is_empty() && !draft.starts_with('/') =>
        {
            (false, Some(draft))
        }
        _ => return Ok(false),
    };
    let Some(review) = state.reviews.pop_front() else {
        return Ok(false);
    };
    let (apply, note) = decision;
    queue_request(
        commands,
        BackendRequest::Review {
            request_id: review.request_id,
            apply,
            note: note.clone(),
        },
    )?;
    if note.is_some() {
        let len = state.composer.text().len();
        state
            .composer
            .apply_completion(0..len, "")
            .map_err(ui_error)?;
        state.refresh_completion();
    }
    let waiting = !state.reviews.is_empty();
    if let Some(turn) = state.turn.as_mut() {
        turn.waiting = waiting;
    }
    if !apply {
        let line = match note {
            Some(note) => format!("Not applied · {note}"),
            None => "Not applied".to_owned(),
        };
        if let Err(error) = state.offline_view_mut().push_system(line) {
            state.notice = format!("Reply display limit reached: {error}");
        }
    }
    state.history_scroll = 0;
    Ok(true)
}

fn handle_key(
    state: &mut UiState,
    key: KeyEvent,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<(bool, bool), CliError> {
    if key.kind == KeyEventKind::Release {
        return Ok((false, false));
    }
    state.selection_gesture = None;
    state.click_sequence = None;
    if gallery_open(state) {
        let effect = state.gallery.handle_key(key);
        apply_gallery_effect(state, effect)?;
        return Ok((false, true));
    }
    state.hover = None;
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    if control
        && state.sheet.is_none()
        && !question_focused(state)
        && !state.panel_focus
        && matches!(key.code, KeyCode::Char('c' | 'C'))
        && let Some(text) = selected_transcript_text(state)
    {
        request_copy_text(state, text)?;
        return Ok((false, true));
    }
    if let Some(id) = state.chip_preview {
        match key.code {
            KeyCode::Char('c' | 'C') if !key.modifiers.contains(KeyModifiers::ALT) => {
                let payload = state
                    .composer
                    .chips()
                    .into_iter()
                    .find(|chip| chip.id == id)
                    .and_then(|chip| match chip.attachment {
                        polaris_ui_model::composer::Attachment::Text { payload, .. } => {
                            Some(payload.to_string())
                        }
                        polaris_ui_model::composer::Attachment::Image { .. } => None,
                    });
                if let Some(payload) = payload {
                    request_copy_text(state, payload)?;
                }
                return Ok((false, true));
            }
            KeyCode::Char('e' | 'E') if key.modifiers.is_empty() => {
                match state.composer.expand_chip(id) {
                    Ok(()) => {
                        state.sheet = None;
                        state.chip_preview = None;
                        state.notice.clear();
                        state.refresh_completion();
                    }
                    Err(error) => state.notice = format!("Expand rejected: {error}"),
                }
                return Ok((false, true));
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                state.sheet = None;
                state.chip_preview = None;
                return Ok((false, true));
            }
            _ => {}
        }
    }
    if control
        && matches!(key.code, KeyCode::Char('c' | 'C' | 'x' | 'X'))
        && state.sheet.is_none()
        && !state.panel_focus
        && !question_focused(state)
        && state.composer.selection().is_some()
    {
        request_copy(state, matches!(key.code, KeyCode::Char('x' | 'X')))?;
        return Ok((false, true));
    }
    if key.code == KeyCode::Esc
        || (control && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')))
    {
        if state.sheet.is_some() && !question_focused(state) {
            state.sheet = None;
            state.chip_preview = None;
            state.profile_picker = None;
            state.effort_picker = None;
            state.settings_open = false;
            state.refresh_completion();
            return Ok((false, true));
        }
        if state.demo_active && (state.demo.busy() || state.demo.question().is_some()) {
            state.demo.cancel().map_err(ui_error)?;
            state.sheet = None;
            state.notice = "DEMO interrupted locally".to_owned();
            return Ok((false, true));
        }
        if state.sheet.take().is_some() {
            state.chip_preview = None;
            state.profile_picker = None;
            state.effort_picker = None;
            state.settings_open = false;
            state.refresh_completion();
            return Ok((false, true));
        }
        if !state.completion.is_empty() {
            state.completion_dismissed = true;
            state.refresh_completion();
            return Ok((false, true));
        }
        if state.panel_focus {
            state.panel_focus = false;
            return Ok((false, true));
        }
        let selected_entry = state.transcript_selection.take().is_some();
        let selected_span = state.transcript_span.take().is_some();
        if selected_entry || selected_span {
            return Ok((false, true));
        }
        if state.turn.is_some() && !control {
            match queue_request(commands, BackendRequest::CancelTurn) {
                Ok(()) => state.notice = "Interrupting…".to_owned(),
                Err(error) => state.notice = error.to_string(),
            }
            return Ok((false, true));
        }
        if state.pending.is_some() {
            state.notice =
                "Directive admission is pending; its durable result must be reconciled before exit"
                    .to_owned();
            return Ok((false, true));
        }
        if control {
            if !state.composer.is_empty() {
                let len = state.composer.text().len();
                state
                    .composer
                    .apply_completion(0..len, "")
                    .map_err(ui_error)?;
                state.exit_armed = None;
                state.refresh_completion();
                return Ok((false, true));
            }
            let now = Instant::now();
            if state
                .exit_armed
                .is_some_and(|at| now.saturating_duration_since(at) <= Duration::from_secs(1))
            {
                return Ok((true, false));
            }
            state.exit_armed = Some(now);
            state.notice = "Press Ctrl+C again within one second to exit".to_owned();
            return Ok((false, true));
        }
        state.notice = "Nothing is running; draft retained".to_owned();
        return Ok((false, true));
    }
    state.exit_armed = None;
    if let Some(selected) = state.effort_picker {
        match key.code {
            KeyCode::Up | KeyCode::Left | KeyCode::Down | KeyCode::Right => {
                let index = Effort::ALL
                    .iter()
                    .position(|effort| *effort == selected)
                    .unwrap_or(0);
                let next = if matches!(key.code, KeyCode::Up | KeyCode::Left) {
                    index.saturating_sub(1)
                } else {
                    index
                        .saturating_add(1)
                        .min(Effort::ALL.len().saturating_sub(1))
                };
                state.effort_picker = Effort::ALL.get(next).copied();
                refresh_effort_picker(state);
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                state.effort_picker = None;
                state.sheet = None;
                choose_effort(state, selected, commands);
            }
            _ => {}
        }
        return Ok((false, true));
    }
    if state.profile_picker.is_some() {
        match key.code {
            KeyCode::Up | KeyCode::Down | KeyCode::Tab | KeyCode::BackTab => {
                state.profile_picker = Some(!matches!(state.profile_picker, Some(true)));
                refresh_profile_picker(state);
            }
            KeyCode::Enter if key.modifiers.is_empty() => apply_profile_choice(state, commands)?,
            _ => {}
        }
        return Ok((false, true));
    }
    if state.settings_open {
        if state.settings_pending {
            state.notice = "Saving selected preference…".to_owned();
            return Ok((false, true));
        }
        if let Some(settings) = state.settings.as_mut() {
            match key.code {
                KeyCode::Up => settings.navigate(Navigation::Previous),
                KeyCode::Down => settings.navigate(Navigation::Next),
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') => settings.cycle(),
                KeyCode::Enter if key.modifiers.is_empty() => {
                    if state.demo_active {
                        state.notice =
                            "DEMO preference applied in memory; no config written".to_owned();
                    } else if !settings.has_pending_edit() {
                        state.notice = "No preference change to save".to_owned();
                    } else if let Some(worker) = state.settings_worker.as_mut() {
                        state.settings_generation = state
                            .settings_generation
                            .checked_add(1)
                            .ok_or(CliError::Unavailable("settings generation exhausted"))?;
                        match worker.request_save(state.settings_generation, settings.clone()) {
                            Ok(()) => {
                                state.settings_pending = true;
                                state.notice = "Saving selected preference…".to_owned();
                            }
                            Err(error) => state.notice = format!("Settings: {error}"),
                        }
                    }
                }
                _ => {}
            }
        }
        refresh_settings_sheet(state);
        return Ok((false, true));
    }
    if handle_question_key(state, key)? {
        return Ok((false, true));
    }
    if handle_review_key(state, key, commands)? {
        return Ok((false, true));
    }
    if key.code == KeyCode::F(5) {
        open_profile_picker(state);
        return Ok((false, true));
    }
    if !matches!(key.code, KeyCode::Up | KeyCode::Down) || !key.modifiers.is_empty() {
        state.vertical_goal = None;
    }
    if key.code == KeyCode::F(2) {
        let lines = vec![
            format!("Session {}", state.id),
            format!("Workspace {}", state.workspace),
            state.provider_line(),
        ];
        open_sheet(state, "Status", lines);
        return Ok((false, true));
    }
    if key.code == KeyCode::F(3) {
        state.panel_kind = PanelKind::Agents;
        state.panel = true;
        return Ok((false, true));
    }
    if key.code == KeyCode::F(4) {
        state.panel = !state.panel;
        state.panel_focus = false;
        return Ok((false, true));
    }
    if key.code == KeyCode::F(6) && state.panel {
        state.panel_focus = !state.panel_focus;
        return Ok((false, true));
    }
    if let Some(sheet) = state.sheet.as_mut() {
        match key.code {
            KeyCode::Enter => state.sheet = None,
            KeyCode::Up => sheet.scroll = sheet.scroll.saturating_sub(1),
            KeyCode::Down => {
                sheet.scroll = sheet
                    .scroll
                    .saturating_add(1)
                    .min(sheet.lines.len().saturating_sub(1))
            }
            KeyCode::PageUp => sheet.scroll = sheet.scroll.saturating_sub(10),
            KeyCode::PageDown => {
                sheet.scroll = sheet
                    .scroll
                    .saturating_add(10)
                    .min(sheet.lines.len().saturating_sub(1))
            }
            _ => return Ok((false, false)),
        };
        return Ok((false, true));
    }
    if state.panel_focus {
        match key.code {
            KeyCode::Up | KeyCode::PageUp => {
                state.panel_scroll = state.panel_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::PageDown => {
                state.panel_scroll = state
                    .panel_scroll
                    .saturating_add(1)
                    .min(state.panel_lines().len().saturating_sub(1))
            }
            _ => return Ok((false, false)),
        };
        return Ok((false, true));
    }
    if state.pending.is_some() || state.switch_pending {
        return Ok((false, false));
    }
    if !state.completion.is_empty() {
        match key.code {
            KeyCode::Up => {
                state.completion_selected = Some(match state.completion_selected {
                    Some(i) => i.saturating_sub(1),
                    None => 0,
                });
                return Ok((false, true));
            }
            KeyCode::Down => {
                state.completion_selected = Some(match state.completion_selected {
                    Some(i) => i
                        .saturating_add(1)
                        .min(state.completion.len().saturating_sub(1)),
                    None => 0,
                });
                return Ok((false, true));
            }
            KeyCode::Tab | KeyCode::Enter if key.modifiers.is_empty() => {
                if state.completion_selected.is_none() {
                    state.completion_selected = Some(0)
                }
                accept_completion(state)?;
                return Ok((false, true));
            }
            _ => {}
        }
    }
    let area = composer_area(state);
    let (first, last) = screen::composer_visual_guards(&state.composer, area);
    let guards = keymap::GuardState {
        draft_empty: state.composer.is_empty(),
        first_visual_row: first,
        last_visual_row: last,
    };
    let action = model_chord(key)
        .and_then(|chord| keymap::lookup(keymap::Context::Composer, chord, guards))
        .map(|b| b.action);
    if action == Some(keymap::Action::Submit) {
        return Ok((submit_draft(state, commands)?, true));
    }
    let mut edited = false;
    if action == Some(keymap::Action::Newline) {
        if let Err(error) = state.composer.newline() {
            state.notice = format!("Edit rejected: {error}");
            return Ok((false, true));
        }
        edited = true;
    } else if action == Some(keymap::Action::OpenHelp) {
        open_sheet(state, "Help", help_lines())
    } else if action == Some(keymap::Action::Undo) {
        edited = state.composer.undo();
    } else if action == Some(keymap::Action::Redo) {
        edited = state.composer.redo();
    } else if action == Some(keymap::Action::ToggleTools) {
        if state.demo_active {
            state.demo.toggle_all_tools();
        }
        state.view_mut().toggle_all_tools();
    } else if action == Some(keymap::Action::ToggleMode) {
        state.view_mut().toggle_mode();
        state.notice = match state.view().mode {
            polaris_ui_model::view::Mode::Plan => {
                "Plan mode: the model plans before answering".to_owned()
            }
            polaris_ui_model::view::Mode::Auto => "Auto mode".to_owned(),
        };
    } else if action == Some(keymap::Action::EffortUp) || action == Some(keymap::Action::EffortDown)
    {
        let current = state.view().effort;
        let index = Effort::ALL
            .iter()
            .position(|effort| *effort == current)
            .unwrap_or(0);
        let next = if action == Some(keymap::Action::EffortUp) {
            index
                .saturating_add(1)
                .min(Effort::ALL.len().saturating_sub(1))
        } else {
            index.saturating_sub(1)
        };
        match Effort::ALL.get(next).copied() {
            Some(effort) if effort != current => choose_effort(state, effort, commands),
            _ => state.notice = format!("Effort is already {}", current.name()),
        }
    } else if action == Some(keymap::Action::Paste) {
        request_paste(state)?;
    } else {
        match key.code {
            KeyCode::Backspace => {
                edited = if control {
                    state.composer.delete_word_backward()
                } else {
                    state.composer.backspace()
                };
            }
            KeyCode::Delete => {
                edited = if control {
                    state.composer.delete_word_forward()
                } else {
                    state.composer.delete()
                };
            }
            KeyCode::Left if control => state.composer.move_word_left_with_selection(shift),
            KeyCode::Right if control => state.composer.move_word_right_with_selection(shift),
            KeyCode::Left if shift => state.composer.move_left_with_selection(true),
            KeyCode::Right if shift => state.composer.move_right_with_selection(true),
            KeyCode::Left => state.composer.move_left(),
            KeyCode::Right => state.composer.move_right(),
            KeyCode::Home if control && shift => state.composer.move_document_home_with_selection(),
            KeyCode::End if control && shift => state.composer.move_document_end_with_selection(),
            KeyCode::Home if control => state.composer.move_document_home(),
            KeyCode::End if control => state.composer.move_document_end(),
            KeyCode::Home => state.composer.move_line_home_with_selection(shift),
            KeyCode::End => state.composer.move_line_end_with_selection(shift),
            KeyCode::Up => {
                if first {
                    edited = state.composer.history_prev(true);
                } else if let Some((at, goal)) = screen::composer_vertical_cursor_with_goal(
                    &state.composer,
                    area,
                    false,
                    state.vertical_goal,
                ) {
                    state.composer.set_cursor(at);
                    state.vertical_goal = Some(goal);
                }
            }
            KeyCode::Down => {
                if last {
                    edited = state.composer.history_next(true);
                } else if let Some((at, goal)) = screen::composer_vertical_cursor_with_goal(
                    &state.composer,
                    area,
                    true,
                    state.vertical_goal,
                ) {
                    state.composer.set_cursor(at);
                    state.vertical_goal = Some(goal);
                }
            }
            KeyCode::PageUp => {
                state.history_scroll = state.history_scroll.saturating_add(10).min(4096)
            }
            KeyCode::PageDown => state.history_scroll = state.history_scroll.saturating_sub(10),
            KeyCode::Char('a' | 'A') if control => state.composer.select_all(),
            KeyCode::Char(ch) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                if let Err(error) = state.composer.insert(&ch.to_string()) {
                    state.notice = format!("Edit rejected: {error}");
                    return Ok((false, true));
                }
                edited = true;
            }
            _ => return Ok((false, false)),
        }
    }
    if edited {
        state.notice.clear();
    }
    state.completion_dismissed = false;
    state.refresh_completion();
    Ok((false, true))
}

fn combine_results(
    first: Result<(), CliError>,
    second: Result<(), CliError>,
) -> Result<(), CliError> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(CliError::Ui(format!("{first}; cleanup: {second}"))),
    }
}
fn projection<'a>(
    state: &'a UiState,
    panel_lines: &'a [String],
    footer: &'a str,
) -> ScreenState<'a> {
    let screen = ScreenState::new(state.view(), &state.composer)
        .with_hover(state.hover)
        .with_presentation_tools(if state.demo_active {
            state.demo.presentation_tools()
        } else {
            &[]
        })
        .with_profile_label(Some(state.profile_label()))
        .with_model_label(if state.demo_active {
            state.demo_model_label
        } else {
            state.model_label.as_deref()
        })
        .with_editor_focus(
            !state.panel_focus
                && state.pending.is_none()
                && !state.switch_pending
                && !gallery_open(state),
        )
        .with_transcript_selection(state.transcript_selection.clone())
        .with_transcript_span_selection(state.transcript_span)
        .with_workspace(Some(&state.workspace))
        .with_notice(if state.notice.is_empty() {
            None
        } else {
            Some(&state.notice)
        })
        .with_activity(if state.activity.is_empty() || state.demo_active {
            None
        } else {
            Some(&state.activity)
        })
        .with_footer(Some(footer))
        .with_history_scroll(state.history_scroll)
        .with_panel(if state.panel {
            PanelVisibility::Open
        } else {
            PanelVisibility::Closed
        })
        .with_panel_content(Some(InfoSheet {
            title: state.panel_kind.title(),
            lines: panel_lines,
            footer: Some(if state.panel_focus {
                "Panel focus - F6 returns to draft"
            } else {
                "F6 focuses panel"
            }),
            scroll: state.panel_scroll,
        }))
        .with_completion(if state.completion.is_empty() {
            None
        } else {
            Some(CompletionOverlay {
                title: "Suggestions",
                items: &state.completion,
                selected: state.completion_selected,
            })
        })
        .with_sheet(state.sheet.as_ref().map(|sheet| InfoSheet {
            title: &sheet.title,
            lines: &sheet.lines,
            footer: Some(if state.effort_picker.is_some() {
                "Up/Down choose · Enter apply local effort · Esc close"
            } else if state.profile_picker.is_some() {
                "Up/Down choose · Enter switch · Esc close"
            } else if state.settings_open {
                "Arrows change · Enter apply field · Esc close"
            } else if state.chip_preview.is_some() {
                "C copy all · E expand · Enter/Esc close"
            } else if state.demo_active && state.demo.question().is_some() {
                "Tab notes · Left/Right questions · Enter submit all · Esc interrupt"
            } else {
                "Up/Down scroll · Enter/Esc close"
            }),
            scroll: sheet.scroll,
        }));
    if question_focused(state)
        && let Some(question) = state.demo.question()
    {
        return screen
            .with_questionnaire(question, state.question_review)
            .with_questionnaire_scroll(state.question_scroll);
    }
    screen
}
fn bounded_area(columns: u16, rows: u16) -> Rect {
    Rect::new(0, 0, columns.clamp(1, 400), rows.clamp(1, 200))
}
fn current_metadata(
    state: &mut UiState,
    options: &RenderOptions,
    area: Rect,
) -> screen::RenderMetadata {
    let lines = state.panel_lines();
    let mut cache = take_render_cache(state);
    let metadata = screen::prepare_metadata_with_cache(
        &projection(state, &lines, ""),
        options,
        area,
        &mut cache,
    );
    state.render_cache = Some(cache);
    metadata
}
fn selected_transcript_text(state: &UiState) -> Option<String> {
    if let Some(selection) = state.transcript_span {
        let (start, end) = if selection.anchor <= selection.head {
            (selection.anchor, selection.head)
        } else {
            (selection.head, selection.anchor)
        };
        if start == end {
            return None;
        }
        let mut output = String::new();
        for (index, entry) in state
            .view()
            .conversation
            .iter()
            .enumerate()
            .take(end.entry_index.saturating_add(1))
            .skip(start.entry_index)
        {
            if index > start.entry_index {
                output.push_str("\n\n");
            }
            let first = if index == start.entry_index {
                start.source_byte
            } else {
                0
            };
            let last = if index == end.entry_index {
                end.source_byte
            } else {
                entry.text.len()
            };
            output.push_str(entry.text.get(first..last)?);
        }
        return (!output.is_empty()).then_some(output);
    }
    let selection = state.transcript_selection.as_ref()?;
    state
        .view()
        .conversation
        .get(selection.entry_index)?
        .text
        .get(selection.source_bytes.clone())
        .map(str::to_owned)
}

fn selection_point(byte: usize) -> screen::TranscriptPoint {
    screen::TranscriptPoint {
        entry_index: 0,
        source_byte: byte,
    }
}
fn selection_unit(
    text: &str,
    at: usize,
    granularity: SelectionGranularity,
) -> std::ops::Range<usize> {
    use polaris_ui_model::composer::{source_grapheme_range, source_line_range, source_word_range};
    let range = match granularity {
        SelectionGranularity::Grapheme => source_grapheme_range(text, at),
        SelectionGranularity::Word => source_word_range(text, at),
        SelectionGranularity::Line => source_line_range(text, at),
    };
    match range {
        Some(range) => range,
        None => at..at,
    }
}
fn activate_selection_owner(
    state: &mut UiState,
    surface: SelectionSurface,
) -> Result<(), CliError> {
    if state.selection_owner != Some(surface) {
        state.selection_gesture = None;
    }
    if surface != SelectionSurface::Transcript {
        state.transcript_selection = None;
        state.transcript_span = None;
    }
    if surface != SelectionSurface::Composer && state.composer.selection().is_some() {
        state.composer.set_cursor(state.composer.cursor());
    }
    if let Some(question) = state.demo.question_mut() {
        use polaris_ui_model::questionnaire::{QuestionnaireAction as A, QuestionnaireFocus};
        let preserve = surface
            == (SelectionSurface::Question {
                index: question.question_index(),
                notes: question.focus() == QuestionnaireFocus::Notes,
            });
        if !preserve {
            let cursor = question.active_text_cursor();
            question.apply(A::SetCursor(cursor)).map_err(ui_error)?;
        }
    }
    state.selection_owner = Some(surface);
    Ok(())
}
fn selection_granularity(
    state: &mut UiState,
    surface: SelectionSurface,
    position: (u16, u16),
    source_hit: bool,
    shift: bool,
) -> SelectionGranularity {
    if !source_hit || shift {
        state.click_sequence = None;
        return SelectionGranularity::Grapheme;
    }
    let now = Instant::now();
    let count = state
        .click_sequence
        .filter(|last| {
            last.surface == surface
                && last
                    .position
                    .0
                    .abs_diff(position.0)
                    .saturating_add(last.position.1.abs_diff(position.1))
                    <= 1
                && now.saturating_duration_since(last.time) <= Duration::from_millis(450)
                && last.count < 3
        })
        .map_or(1, |last| last.count.saturating_add(1));
    state.click_sequence = Some(ClickSequence {
        surface,
        position,
        time: now,
        count,
    });
    match count {
        2 => SelectionGranularity::Word,
        3 => SelectionGranularity::Line,
        _ => SelectionGranularity::Grapheme,
    }
}
fn apply_surface_selection(
    state: &mut UiState,
    surface: SelectionSurface,
    anchor: screen::TranscriptPoint,
    head: screen::TranscriptPoint,
) -> Result<(), CliError> {
    match surface {
        SelectionSurface::Composer => state
            .composer
            .set_selection(anchor.source_byte..head.source_byte),
        SelectionSurface::Transcript => {
            state.transcript_selection = None;
            state.transcript_span = Some(screen::TranscriptSpanSelection { anchor, head });
        }
        SelectionSurface::Question { index, notes } => {
            if let Some(question) = state.demo.question_mut() {
                use polaris_ui_model::questionnaire::{
                    QuestionnaireAction as A, QuestionnaireFocus,
                };
                if question.question_index() == index
                    && (question.focus() == QuestionnaireFocus::Notes) == notes
                {
                    question
                        .apply(A::SetSelection {
                            anchor: anchor.source_byte,
                            head: head.source_byte,
                        })
                        .map_err(ui_error)?;
                }
            }
        }
    }
    Ok(())
}
fn begin_editor_selection(
    state: &mut UiState,
    surface: SelectionSurface,
    at: usize,
    source: Option<std::ops::Range<usize>>,
    position: (u16, u16),
    shift: bool,
) -> Result<(), CliError> {
    activate_selection_owner(state, surface)?;
    let granularity = selection_granularity(state, surface, position, source.is_some(), shift);
    let (existing_anchor, mut range) = match surface {
        SelectionSurface::Composer => {
            let unit_at = source.as_ref().map_or(at, |range| range.start);
            let range = if granularity == SelectionGranularity::Word {
                let _selected = state.composer.select_word_at(unit_at);
                state.composer.selection().map_or(at..at, |range| range)
            } else if granularity == SelectionGranularity::Line {
                selection_unit(state.composer.text(), unit_at, granularity)
            } else {
                at..at
            };
            (
                state
                    .composer
                    .selection_anchor()
                    .unwrap_or(state.composer.cursor()),
                range,
            )
        }
        SelectionSurface::Question { .. } => {
            let question = state
                .demo
                .question()
                .ok_or(CliError::Unavailable("question editor closed"))?;
            let editor = question
                .active_editor()
                .ok_or(CliError::Unavailable("question editor unavailable"))?;
            let range = if granularity == SelectionGranularity::Grapheme {
                at..at
            } else {
                selection_unit(
                    editor.text(),
                    source.as_ref().map_or(at, |range| range.start),
                    granularity,
                )
            };
            (editor.selection_anchor().unwrap_or(editor.cursor()), range)
        }
        SelectionSurface::Transcript => return Ok(()),
    };
    if shift {
        range = existing_anchor..at;
    }
    let start = selection_point(range.start);
    let end = selection_point(range.end);
    apply_surface_selection(state, surface, start, end)?;
    state.selection_gesture = Some(SelectionGesture {
        surface,
        granularity,
        anchor_start: start,
        anchor_end: end,
    });
    state.vertical_goal = None;
    Ok(())
}
fn begin_transcript_selection(
    state: &mut UiState,
    source: &screen::SourceCellSpan,
    position: (u16, u16),
) -> Result<(), CliError> {
    let surface = SelectionSurface::Transcript;
    activate_selection_owner(state, surface)?;
    let granularity = selection_granularity(state, surface, position, true, false);
    let entry = state
        .view()
        .conversation
        .get(source.entry_index)
        .ok_or(CliError::Unavailable("transcript entry unavailable"))?;
    let range = if granularity == SelectionGranularity::Grapheme {
        source.source_bytes.clone()
    } else {
        selection_unit(&entry.text, source.source_bytes.start, granularity)
    };
    let start = screen::TranscriptPoint {
        entry_index: source.entry_index,
        source_byte: range.start,
    };
    let end = screen::TranscriptPoint {
        entry_index: source.entry_index,
        source_byte: range.end,
    };
    apply_surface_selection(
        state,
        surface,
        start,
        if granularity == SelectionGranularity::Grapheme {
            start
        } else {
            end
        },
    )?;
    state.selection_gesture = Some(SelectionGesture {
        surface,
        granularity,
        anchor_start: start,
        anchor_end: end,
    });
    Ok(())
}
fn drag_selection(
    state: &mut UiState,
    metadata: &screen::RenderMetadata,
    mouse: crossterm::event::MouseEvent,
) -> Result<bool, CliError> {
    let Some(gesture) = state.selection_gesture else {
        return Ok(false);
    };
    let target = match gesture.surface {
        SelectionSurface::Composer
            if !question_focused(state) && state.sheet.is_none() && !gallery_open(state) =>
        {
            let rect = metadata.composer_rect;
            let x = mouse
                .column
                .clamp(rect.x, rect.right().saturating_sub(1).max(rect.x));
            let y = mouse.row.clamp(
                rect.y.saturating_add(1),
                rect.bottom()
                    .saturating_sub(2)
                    .max(rect.y.saturating_add(1)),
            );
            screen::composer_hit_at(&state.composer, rect, x, y).map(|hit| {
                let range = if gesture.granularity == SelectionGranularity::Grapheme {
                    hit.cursor..hit.cursor
                } else {
                    selection_unit(
                        state.composer.text(),
                        hit.source_bytes.map_or(hit.cursor, |range| range.start),
                        gesture.granularity,
                    )
                };
                (selection_point(range.start), selection_point(range.end))
            })
        }
        SelectionSurface::Transcript
            if !question_focused(state) && state.sheet.is_none() && !gallery_open(state) =>
        {
            let y = mouse.row.clamp(
                metadata.body.y,
                metadata
                    .body
                    .bottom()
                    .saturating_sub(1)
                    .max(metadata.body.y),
            );
            let source = metadata.source_at(mouse.column, y);
            let boundary = metadata.transcript_boundary_at(mouse.column, y);
            match (source, boundary) {
                (Some(source), _) => {
                    state
                        .view()
                        .conversation
                        .get(source.entry_index)
                        .map(|entry| {
                            let range = if gesture.granularity == SelectionGranularity::Grapheme {
                                source.source_bytes.clone()
                            } else {
                                selection_unit(
                                    &entry.text,
                                    source.source_bytes.start,
                                    gesture.granularity,
                                )
                            };
                            (
                                screen::TranscriptPoint {
                                    entry_index: source.entry_index,
                                    source_byte: range.start,
                                },
                                screen::TranscriptPoint {
                                    entry_index: source.entry_index,
                                    source_byte: range.end,
                                },
                            )
                        })
                }
                (None, Some(point)) => {
                    state
                        .view()
                        .conversation
                        .get(point.entry_index)
                        .map(|entry| {
                            let range = if gesture.granularity == SelectionGranularity::Grapheme {
                                point.source_byte..point.source_byte
                            } else {
                                selection_unit(&entry.text, point.source_byte, gesture.granularity)
                            };
                            (
                                screen::TranscriptPoint {
                                    entry_index: point.entry_index,
                                    source_byte: range.start,
                                },
                                screen::TranscriptPoint {
                                    entry_index: point.entry_index,
                                    source_byte: range.end,
                                },
                            )
                        })
                }
                _ => None,
            }
        }
        SelectionSurface::Question { index, notes }
            if question_focused(state) && !state.question_review =>
        {
            use polaris_ui_model::questionnaire::QuestionnaireFocus;
            match (
                state.demo.question(),
                metadata.questionnaire_editor.as_ref(),
            ) {
                (Some(question), Some(field))
                    if question.question_index() == index
                        && (question.focus() == QuestionnaireFocus::Notes) == notes =>
                {
                    let y = mouse.row.clamp(
                        field.rect.y,
                        field.rect.bottom().saturating_sub(1).max(field.rect.y),
                    );
                    let row = field
                        .visible_rows
                        .start
                        .saturating_add(usize::from(y.saturating_sub(field.rect.y)));
                    let column = usize::from(mouse.column.saturating_sub(field.rect.x));
                    let at = field.geometry.cursor_at_cell(row, column);
                    question.active_editor().map(|editor| {
                        let range = if gesture.granularity == SelectionGranularity::Grapheme {
                            at..at
                        } else {
                            selection_unit(
                                editor.text(),
                                field
                                    .geometry
                                    .source_at_cell(row, column)
                                    .map_or(at, |range| range.start),
                                gesture.granularity,
                            )
                        };
                        (selection_point(range.start), selection_point(range.end))
                    })
                }
                _ => None,
            }
        }
        _ => {
            state.selection_gesture = None;
            return Ok(false);
        }
    };
    let Some((start, end)) = target else {
        return Ok(false);
    };
    let (anchor, head) = if start < gesture.anchor_start {
        (gesture.anchor_end, start)
    } else {
        (gesture.anchor_start, end)
    };
    apply_surface_selection(state, gesture.surface, anchor, head)?;
    Ok(true)
}
fn finish_selection(state: &mut UiState) -> Result<bool, CliError> {
    let Some(gesture) = state.selection_gesture.take() else {
        return Ok(false);
    };
    match gesture.surface {
        SelectionSurface::Composer => {
            if state.composer.selection().is_some() {
                request_copy(state, false)?;
            }
        }
        SelectionSurface::Transcript => {
            if let Some(text) = selected_transcript_text(state) {
                request_copy_text(state, text)?;
            }
        }
        SelectionSurface::Question { index, notes } => {
            use polaris_ui_model::questionnaire::QuestionnaireFocus;
            let text = state
                .demo
                .question()
                .filter(|q| {
                    q.question_index() == index && (q.focus() == QuestionnaireFocus::Notes) == notes
                })
                .and_then(|q| q.active_editor())
                .and_then(|editor| editor.selected_text());
            if let Some(text) = text {
                request_copy_text(state, text)?;
            }
        }
    }
    Ok(true)
}

fn dispatch_footer(state: &mut UiState, action: screen::FooterAction) {
    match action {
        screen::FooterAction::Help => open_sheet(state, "Help", help_lines()),
        screen::FooterAction::ToggleMode => {
            state.view_mut().toggle_mode();
            state.notice = "Local mode preference changed".to_owned();
        }
        screen::FooterAction::Profile => open_profile_picker(state),
        screen::FooterAction::Model => {
            if state.demo_active {
                state.gallery.open(GalleryPage::Model);
            } else {
                open_sheet(
                    state,
                    "Model",
                    vec![
                        "Model execution unavailable: no provider profile is attached.".to_owned(),
                        "F5 selects the isolated Demo to inspect reference models.".to_owned(),
                    ],
                );
            }
        }
        screen::FooterAction::Effort => open_effort_picker(state),
    }
}
fn preview_chip(state: &mut UiState, id: u64) {
    let payload = state
        .composer
        .chips()
        .iter()
        .find(|chip| chip.id == id)
        .and_then(|chip| match chip.attachment {
            polaris_ui_model::composer::Attachment::Text { payload, .. } => Some(payload.clone()),
            polaris_ui_model::composer::Attachment::Image { .. } => None,
        });
    if let Some(payload) = payload {
        state.chip_preview = Some(id);
        // Exact payload stays in the composer. The preview displays a bounded window.
        let mut lines: Vec<String> = payload
            .lines()
            .take(512)
            .map(|line| line.to_owned())
            .collect();
        if payload.lines().count() > 512 {
            lines.push(
                "[Preview limited to 512 lines; Copy retains the complete payload]".to_owned(),
            );
        }
        open_sheet(state, "Pasted text", lines);
    }
}
fn handle_mouse(
    state: &mut UiState,
    mouse: crossterm::event::MouseEvent,
    commands: &mpsc::Sender<BackendRequest>,
    area: Rect,
    options: &RenderOptions,
) -> Result<(bool, bool), CliError> {
    use crossterm::event::MouseButton;
    use screen::HitTarget as H;
    if gallery_open(state) {
        let gallery_area = gallery_rect(state, area);
        state.gallery.prepare_hits(gallery_area, *options);
        let effect = state.gallery.handle_mouse(mouse);
        apply_gallery_effect(state, effect)?;
        return Ok((false, true));
    }
    if mouse.kind == MouseEventKind::Moved
        && !state.layout_dirty
        && let Some(metadata) = state.last_metadata.as_ref()
    {
        let target = metadata.hit_at(mouse.column, mouse.row);
        let changed = state.hover != target;
        state.hover = target;
        return Ok((false, changed));
    }
    let metadata = current_metadata(state, options, area);
    let target = metadata.hit_at(mouse.column, mouse.row);
    if mouse.kind == MouseEventKind::Moved {
        let changed = state.hover != target;
        state.hover = target;
        return Ok((false, changed));
    }
    if mouse.kind == MouseEventKind::Up(MouseButton::Left) {
        return Ok((false, finish_selection(state)?));
    }
    if mouse.kind == MouseEventKind::Drag(MouseButton::Left) {
        return Ok((false, drag_selection(state, &metadata, mouse)?));
    }
    if question_focused(state) {
        if matches!(
            mouse.kind,
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
        ) {
            if state.question_review {
                state.question_scroll = if mouse.kind == MouseEventKind::ScrollUp {
                    state.question_scroll.saturating_sub(3)
                } else {
                    state.question_scroll.saturating_add(3).min(32768)
                };
            } else if let Some(question) = state.demo.question_mut() {
                use polaris_ui_model::questionnaire::QuestionnaireAction as A;
                question
                    .apply(if mouse.kind == MouseEventKind::ScrollUp {
                        A::ChoiceUp
                    } else {
                        A::ChoiceDown
                    })
                    .map_err(ui_error)?;
            }
            return Ok((false, true));
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
            use polaris_ui_model::questionnaire::QuestionnaireAction as A;
            match target {
                Some(H::QuestionTab(index)) => {
                    navigate_question(state, index);
                    state.question_scroll = 0;
                }
                Some(H::QuestionSubmit) => {
                    let _handled = handle_question_key(
                        state,
                        KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    )?;
                }
                Some(H::QuestionChoice(index)) => {
                    if let Some(question) = state.demo.question_mut() {
                        question.apply(A::FocusChoices).map_err(ui_error)?;
                        for _ in 0..17 {
                            if question.choice_cursor() == index {
                                break;
                            }
                            question
                                .apply(if question.choice_cursor() > index {
                                    A::ChoiceUp
                                } else {
                                    A::ChoiceDown
                                })
                                .map_err(ui_error)?;
                        }
                        question.apply(A::ToggleChoice).map_err(ui_error)?;
                    }
                }
                Some(field_target @ (H::QuestionNotes | H::QuestionAnswer)) => {
                    let index = state
                        .demo
                        .question()
                        .map_or(0, |question| question.question_index());
                    let notes = field_target == H::QuestionNotes;
                    let surface = SelectionSurface::Question { index, notes };
                    activate_selection_owner(state, surface)?;
                    if let Some(question) = state.demo.question_mut() {
                        question
                            .apply(if notes {
                                A::FocusNotes
                            } else {
                                A::FocusChoices
                            })
                            .map_err(ui_error)?;
                    }
                    let focused = current_metadata(state, options, area);
                    if let Some(field) = focused.questionnaire_editor.as_ref() {
                        let row = field
                            .visible_rows
                            .start
                            .saturating_add(usize::from(mouse.row.saturating_sub(field.rect.y)));
                        let column = usize::from(mouse.column.saturating_sub(field.rect.x));
                        let at = field.geometry.cursor_at_cell(row, column);
                        let source = if field
                            .rect
                            .contains(ratatui::layout::Position::new(mouse.column, mouse.row))
                        {
                            field.geometry.source_at_cell(row, column)
                        } else {
                            None
                        };
                        begin_editor_selection(
                            state,
                            surface,
                            at,
                            source,
                            (mouse.column, mouse.row),
                            mouse.modifiers.contains(KeyModifiers::SHIFT),
                        )?;
                    }
                }
                Some(H::Footer(action)) => dispatch_footer(state, action),
                _ => {}
            }
            refresh_demo_question(state);
            return Ok((false, true));
        }
        return Ok((false, false));
    }
    if matches!(
        mouse.kind,
        MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
    ) {
        let up = mouse.kind == MouseEventKind::ScrollUp;
        if let Some(sheet) = state.sheet.as_mut() {
            if target == Some(H::Sheet) {
                sheet.scroll = if up {
                    sheet.scroll.saturating_sub(3)
                } else {
                    sheet
                        .scroll
                        .saturating_add(3)
                        .min(sheet.lines.len().saturating_sub(1))
                };
            }
        } else if !state.completion.is_empty() {
            state.completion_selected = Some(match state.completion_selected {
                Some(index) if up => index.saturating_sub(1),
                Some(index) => index
                    .saturating_add(1)
                    .min(state.completion.len().saturating_sub(1)),
                None => 0,
            });
        } else if matches!(target, Some(H::Panel | H::Tool(_))) {
            state.panel_scroll = if up {
                state.panel_scroll.saturating_sub(3)
            } else {
                state
                    .panel_scroll
                    .saturating_add(3)
                    .min(state.panel_lines().len().saturating_sub(1))
            };
        } else {
            state.history_scroll = if up {
                state.history_scroll.saturating_add(3).min(4096)
            } else {
                state.history_scroll.saturating_sub(3)
            };
        }
        return Ok((false, true));
    }
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return Ok((false, false));
    }
    if state.sheet.is_some() {
        if target == Some(H::Sheet) {
            handle_sheet_click(state, &metadata, mouse.row, commands)?;
            return Ok((false, true));
        }
        return Ok((false, false));
    }
    if state.pending.is_some() || state.switch_pending {
        return Ok((false, false));
    }
    match target {
        Some(H::Footer(action)) => dispatch_footer(state, action),
        Some(H::Completion(index)) => {
            state.completion_selected = Some(index);
            accept_completion(state)?;
            state.hover = None;
        }
        Some(H::CompletionBackdrop) => return Ok((false, false)),
        Some(H::Panel) => state.panel_focus = true,
        Some(H::Tool(index)) => {
            let _changed = state.view_mut().toggle_tool(index);
        }
        Some(H::ToLatest) => state.history_scroll = 0,
        Some(H::Composer) => {
            state.panel_focus = false;
            if let Some(hit) = screen::composer_hit_at(
                &state.composer,
                metadata.composer_rect,
                mouse.column,
                mouse.row,
            ) {
                let chip = state
                    .composer
                    .chips()
                    .into_iter()
                    .find(|chip| hit.cursor >= chip.range.start && hit.cursor < chip.range.end)
                    .map(|chip| chip.id);
                if let Some(id) = chip {
                    preview_chip(state, id);
                    return Ok((false, true));
                }
                begin_editor_selection(
                    state,
                    SelectionSurface::Composer,
                    hit.cursor,
                    hit.source_bytes,
                    (mouse.column, mouse.row),
                    mouse.modifiers.contains(KeyModifiers::SHIFT),
                )?;
                state.completion_dismissed = true;
                state.refresh_completion();
            }
        }
        Some(H::PresentationTool(index)) => {
            state.demo.toggle_tool(index);
        }
        Some(H::Transcript) => {
            if let Some(source) = metadata.source_at(mouse.column, mouse.row) {
                begin_transcript_selection(state, source, (mouse.column, mouse.row))?;
            } else {
                state.selection_gesture = None;
                state.click_sequence = None;
                state.transcript_selection = None;
                state.transcript_span = None;
            }
        }
        _ => return Ok((false, false)),
    }
    Ok((false, true))
}
fn handle_sheet_click(
    state: &mut UiState,
    metadata: &screen::RenderMetadata,
    row: u16,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<(), CliError> {
    let Some(region) = metadata
        .hits
        .iter()
        .find(|hit| hit.target == screen::HitTarget::Sheet)
    else {
        return Ok(());
    };
    let first = region.rect.y.saturating_add(3);
    if row < first || row >= region.rect.bottom().saturating_sub(2) {
        return Ok(());
    }
    let scroll = state.sheet.as_ref().map_or(0, |sheet| sheet.scroll);
    let index = usize::from(row.saturating_sub(first)).saturating_add(scroll);
    if state.profile_picker.is_some() && index < 2 {
        state.profile_picker = Some(index == 1);
        refresh_profile_picker(state);
        apply_profile_choice(state, commands)?;
    } else if state.effort_picker.is_some() && index < Effort::ALL.len() {
        state.effort_picker = Effort::ALL.get(index).copied();
        refresh_effort_picker(state);
    } else if state.settings_open && index < 6 {
        if let Some(settings) = state.settings.as_mut() {
            for _ in 0..6 {
                if settings.rows().get(index).is_some_and(|row| row.selected) {
                    break;
                }
                settings.navigate(Navigation::Next);
            }
            if !state.settings_pending {
                settings.cycle();
            }
        }
        refresh_settings_sheet(state);
    } else if state.demo_active && state.demo.question().is_some() && index >= 3 {
        use polaris_ui_model::questionnaire::{QuestionKind, QuestionnaireAction as A};
        if let Some(question) = state.demo.question_mut() {
            let count = question.current_question().map_or(0, |q| match &q.kind {
                QuestionKind::SingleChoice { options } | QuestionKind::MultiChoice { options } => {
                    options.len().saturating_add(1)
                }
                QuestionKind::FreeText => 0,
            });
            let choice = index.saturating_sub(3);
            if choice < count {
                question.apply(A::FocusChoices).map_err(ui_error)?;
                for _ in 0..count {
                    if question.choice_cursor() == choice {
                        break;
                    }
                    question
                        .apply(if question.choice_cursor() < choice {
                            A::ChoiceDown
                        } else {
                            A::ChoiceUp
                        })
                        .map_err(ui_error)?;
                }
                question.apply(A::ToggleChoice).map_err(ui_error)?;
            }
        }
        refresh_demo_question(state);
    }
    Ok(())
}
fn handle_input(
    state: &mut UiState,
    input: NormalizedInput,
    commands: &mpsc::Sender<BackendRequest>,
    area: &mut Rect,
    options: &RenderOptions,
) -> Result<(bool, bool), CliError> {
    state.viewport = *area;
    if !matches!(&input, NormalizedInput::Mouse(mouse) if mouse.kind == MouseEventKind::Moved)
        && !matches!(&input, NormalizedInput::Key(key) if key.kind == KeyEventKind::Release)
    {
        state.layout_dirty = true;
    }
    match input {
        NormalizedInput::Key(key) => handle_key(state, key, commands),
        NormalizedInput::Paste(text) => {
            state.vertical_goal = None;
            if gallery_open(state) {
                let effect = state.gallery.handle_paste(&text);
                apply_gallery_effect(state, effect)?;
                return Ok((false, true));
            }
            if question_focused(state)
                && let Some(question) = state.demo.question_mut()
            {
                use polaris_ui_model::questionnaire::{
                    QuestionKind, QuestionnaireAction, QuestionnaireFocus,
                };
                if question.focus() == QuestionnaireFocus::Notes
                    || question
                        .current_question()
                        .is_some_and(|q| q.kind == QuestionKind::FreeText)
                {
                    if let Err(error) = question.apply(QuestionnaireAction::InsertText(text)) {
                        state.notice = format!("DEMO paste rejected: {error}");
                    }
                    refresh_demo_question(state);
                    return Ok((false, true));
                }
                state.notice = "Tab focuses question notes before pasting".to_owned();
                return Ok((false, true));
            }
            if state.pending.is_some()
                || state.switch_pending
                || state.sheet.is_some()
                || state.panel_focus
            {
                return Ok((false, false));
            }
            match state.composer.insert(&text) {
                Ok(()) => {
                    state.notice.clear();
                    state.completion_dismissed = false;
                    state.refresh_completion()
                }
                Err(error) => state.notice = format!("Paste rejected: {error}"),
            };
            Ok((false, true))
        }
        NormalizedInput::Resize { columns, rows } => {
            *area = bounded_area(columns, rows);
            state.viewport = *area;
            state.hover = None;
            state.selection_gesture = None;
            state.click_sequence = None;
            state.last_metadata = None;
            if columns > 400 || rows > 200 {
                state.notice = "Viewport limited to 400x200 cells".to_owned()
            }
            Ok((false, true))
        }
        NormalizedInput::Focus { focused } => {
            if !focused {
                state.hover = None;
                state.selection_gesture = None;
                state.click_sequence = None;
            }
            Ok((false, true))
        }
        NormalizedInput::Mouse(mouse) => handle_mouse(state, mouse, commands, *area, options),
    }
}

fn terminal_loop(
    id: SessionId,
    workspace: String,
    initial: Option<String>,
    trace_path: Option<PathBuf>,
    options: RenderOptions,
    provenance: Vec<String>,
    demo: bool,
    initial_scenario: Option<String>,
    settings: CliSettings,
    commands: mpsc::Sender<BackendRequest>,
    mut events: mpsc::Receiver<BackendMessage>,
    stop: watch::Sender<bool>,
) -> Result<(), CliError> {
    let mut state = UiState::new(id, workspace, initial, provenance)?;
    state.demo_active = demo;
    state.offline_ready = !demo;
    state.panel = settings.effective().panel == polaris_config::CliPanelDefault::Open;
    state.settings = Some(settings);
    if let Some(scenario) = initial_scenario {
        load_demo_scenario(&mut state, &scenario)?;
    }
    state.clipboard = Some(
        ClipboardService::new(
            Duration::from_secs(2),
            polaris_ui_model::composer::MAX_TEXT_BYTES,
        )
        .map_err(ui_error)?,
    );
    state.settings_worker = Some(SettingsService::new().map_err(ui_error)?);
    let mut index =
        WorkspaceFileIndex::new(PathBuf::from(&state.workspace), IndexLimits::default())
            .map_err(ui_error)?;
    state.index_generation = 1;
    index.request(state.index_generation).map_err(ui_error)?;
    state.index = Some(index);
    state.refresh_completion();
    let mut trace = match trace_path {
        Some(path) => Some(KeyTrace::create(&path)?),
        None => None,
    };
    let (columns, rows) = crossterm::terminal::size()?;
    let mut area = bounded_area(columns, rows);
    let guard = TerminalGuard::acquire(CrosstermOps::default(), io::stdout()).map_err(ui_error)?;
    let mut input_reader = InputReader::new().map_err(ui_error)?;
    let mut presenter = BufferedPresenter::new_with_cursor_state(guard, CursorState::Hidden);
    let backend = CrosstermBackend::new(presenter.writer());
    let result = (|| -> Result<(), CliError> {
        let mut terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fixed(area),
            },
        )?;
        let started = Instant::now();
        let mut options = options;
        let mut preferences = state.settings.as_ref().map(CliSettings::effective);
        let mut scheduler = InputScheduler::new();
        scheduler.mark_dirty();
        while !*stop.borrow() {
            if poll_services(&mut state)? {
                scheduler.mark_dirty();
            }
            if state.demo_active && state.demo.tick(started.elapsed()).map_err(ui_error)? {
                state.layout_dirty = true;
                refresh_demo_question(&mut state);
                scheduler.mark_dirty();
            }
            if state.demo_active && state.demo.question().is_some() && state.sheet.is_none() {
                refresh_demo_question(&mut state);
            }
            if let Some((until, notice)) = state.transient_notice.as_ref()
                && Instant::now() >= *until
            {
                if state.notice == *notice {
                    state.notice.clear();
                    state.layout_dirty = true;
                    scheduler.mark_dirty();
                }
                state.transient_notice = None;
            }
            if let Some(turn) = state.turn.as_ref() {
                let label = turn.label();
                if label != state.activity {
                    state.activity = label;
                    scheduler.mark_dirty();
                }
            }
            let effective = state.settings.as_ref().map(CliSettings::effective);
            if effective != preferences {
                if let Some(effective) = effective {
                    options = options_from_preferences(effective, false);
                }
                preferences = effective;
                scheduler.mark_dirty();
            }
            let backend_started = Instant::now();
            for _ in 0..256 {
                if backend_started.elapsed() >= Duration::from_millis(4) {
                    break;
                }
                match events.try_recv() {
                    Ok(message) => {
                        state.receive(message)?;
                        scheduler.mark_dirty()
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        return Err(CliError::Unavailable("backend owner closed the UI stream"));
                    }
                }
            }
            let mut quit = false;
            let mut changed = false;
            {
                let mut batch = scheduler.begin_batch(started.elapsed());
                while let Some(input) = batch.next(started.elapsed()) {
                    if let Some(trace) = trace.as_mut() {
                        trace.record_input(&input)?
                    }
                    let (exit, dirty) =
                        handle_input(&mut state, input, &commands, &mut area, &options)?;
                    changed |= dirty;
                    if exit {
                        quit = true;
                        break;
                    }
                }
            }
            if quit {
                break;
            }
            if changed {
                scheduler.mark_dirty()
            }
            if scheduler.frame_due(started.elapsed()) {
                let _written =
                    present_frame(&mut terminal, &mut presenter, &mut state, &options, area)?;
                scheduler.mark_presented(started.elapsed());
            }
            let timeout = if scheduler.pending() > 0 {
                Duration::ZERO
            } else {
                match scheduler.next_frame_due() {
                    Some(due) => due
                        .saturating_sub(started.elapsed())
                        .min(Duration::from_millis(50)),
                    None => Duration::from_millis(50),
                }
            };
            // Bounded intake. Normalized semantic events keep their relative order.
            let mut intake_timeout = timeout;
            for _ in 0..256 {
                match input_reader.poll(intake_timeout) {
                    Ok(Some(input)) => scheduler.enqueue(input).map_err(ui_error)?,
                    Ok(None) => break,
                    Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                        state.notice = format!("Input rejected: {error}");
                        scheduler.mark_dirty();
                        break;
                    }
                    Err(error) => return Err(ui_error(error)),
                }
                intake_timeout = Duration::ZERO;
            }
        }
        Ok(())
    })();
    stop.send_replace(true);
    // Restore the reader's mode changes before TerminalGuard restores its earlier snapshot.
    let input_restore = input_reader.close().map_err(ui_error);
    drop(input_reader);
    let restore = match presenter.into_inner() {
        Ok(mut guard) => guard.close().map_err(ui_error),
        Err(error) => Err(ui_error(error)),
    };
    if let Some(worker) = state.clipboard.as_mut() {
        worker.shutdown();
    }
    if let Some(worker) = state.index.as_mut() {
        worker.shutdown();
    }
    if let Some(worker) = state.settings_worker.as_mut() {
        worker.shutdown();
    }
    let trace_result = match trace {
        Some(trace) => trace.finish().map_err(CliError::from),
        None => Ok(()),
    };
    combine_results(
        result,
        combine_results(input_restore, combine_results(restore, trace_result)),
    )
}

fn take_render_cache(state: &mut UiState) -> screen::RenderCache {
    match state.render_cache.take() {
        Some(cache) => cache,
        None => screen::RenderCache::new(),
    }
}
fn paint_ui_frame(
    frame: &mut ratatui::Frame<'_>,
    state: &mut UiState,
    options: &RenderOptions,
) -> screen::RenderMetadata {
    let area = frame.area();
    let lines = state.panel_lines();
    let footer = footer_text(state);
    let mut cache = take_render_cache(state);
    let mut metadata = {
        let projected = projection(state, &lines, &footer);
        screen::render_with_cache(frame, &projected, options, &mut cache)
    };
    state.render_cache = Some(cache);
    if gallery_open(state) && !metadata.below_minimum && !metadata.oversized {
        let gallery_area = gallery_rect(state, area);
        state.gallery.render(frame, gallery_area, *options);
        metadata.caret = None;
        metadata.source_cells.clear();
        metadata.source_spans.clear();
    }
    metadata
}

fn present_frame<W: Write>(
    terminal: &mut Terminal<CrosstermBackend<polaris_ui_cli::presenter::StagedWriter>>,
    presenter: &mut BufferedPresenter<W>,
    state: &mut UiState,
    options: &RenderOptions,
    area: Rect,
) -> Result<usize, CliError> {
    state.viewport = area;
    presenter.begin_frame().map_err(ui_error)?;
    let draw = (|| -> Result<(), CliError> {
        if terminal.get_frame().area() != area {
            terminal.resize(area)?;
        }
        let mut metadata = None;
        terminal.draw(|frame| metadata = Some(paint_ui_frame(frame, state, options)))?;
        state.last_metadata = metadata;
        state.layout_dirty = false;
        Ok(())
    })();
    if let Err(error) = draw {
        let abort = presenter.abort_frame().map_err(ui_error);
        combine_results(Err(error), abort)?
    }
    presenter.commit().map_err(ui_error)
}
#[cfg(test)]
#[path = "../../../tests/apps/polaris-cli/controller/mod.rs"]
mod controller_tests;
