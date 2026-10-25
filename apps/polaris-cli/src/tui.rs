//! Application-owned terminal loop, deterministic dumps and real offline backend coordination.
use crate::{
    Args, CliError,
    key_trace::KeyTrace,
    session::{OfflineSession, ResolvedRecord},
};
use crossterm::event::{self, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind};
use polaris_sdk::{
    effort::Effort,
    ids::{ModelId, SessionId},
};
use polaris_ui_cli::{
    input::{NormalizedInput, normalize_event},
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
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend, layout::Rect};
use std::{
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
    if args.command.is_some()
        && (args.dump.is_some()
            || args.prompt.is_some()
            || args.scenario.is_some()
            || args.key_trace.is_some())
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
    if let Some(s) = args.scenario.as_deref() {
        if !matches!(
            s,
            "welcome" | "conversation" | "error" | "completion" | "panel" | "help"
        ) {
            return Err(CliError::Unavailable(
                "unknown --scenario; use welcome, conversation, error, completion, panel or help",
            ));
        }
    }
    Ok(())
}
fn render_options(args: &Args) -> RenderOptions {
    let disabled = matches!(args.color, ColorPolicy::Never)
        || (matches!(args.color, ColorPolicy::Auto)
            && (std::env::var_os("NO_COLOR").is_some()
                || std::env::var("TERM").ok().as_deref() == Some("dumb")));
    let colors = if disabled {
        ColorMode::NoColor
    } else if args.theme == ThemeMode::Terminal {
        ColorMode::Terminal
    } else {
        match args.color_depth {
            ColorDepth::Truecolor => ColorMode::TrueColor,
            ColorDepth::Ansi => ColorMode::Ansi,
            ColorDepth::Auto => {
                if std::env::var("COLORTERM")
                    .ok()
                    .is_some_and(|s| s == "truecolor" || s == "24bit")
                {
                    ColorMode::TrueColor
                } else {
                    ColorMode::Ansi
                }
            }
        }
    };
    RenderOptions {
        colors,
        ascii: args.ascii,
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
    let notice = if args.scenario.is_some() {
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
                if chord.key == keymap::Key::Enter && (chord.mods.ctrl || chord.mods.shift) {
                    return None;
                }
                if chord.key == keymap::Key::Char('j') && chord.mods.ctrl {
                    effective.footer = Some("newline")
                }
            }
            if binding.action == keymap::Action::ToggleMode {
                effective.description =
                    "Toggle the local mode preference; model execution unavailable"
            } else if binding.action == keymap::Action::EffortUp {
                effective.description =
                    "Raise the local effort preference; model execution unavailable"
            } else if binding.action == keymap::Action::EffortDown {
                effective.description =
                    "Lower the local effort preference; model execution unavailable"
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
    let mut lines=vec!["Offline: /help /status /stats /sessions /session resume <id> /clear /quit".to_owned(),
  "Cockpit: /panel [tasks|agents|questions|review|proof|context|system|off], F4 toggle, F6 focus".to_owned(),
  "Enter sends a directive; Ctrl+J or Alt+Enter inserts a newline. Shift+Enter depends on the terminal.".to_owned(),
  "Model execution, live human questions and advanced services require their real execution profiles.".to_owned()];
    lines.push(
        "Use terminal paste; native clipboard, image paste and selection copy are unavailable."
            .to_owned(),
    );
    lines.extend(keymap::help_lines(&effective_bindings()));
    lines
}

#[derive(Debug)]
enum BackendRequest {
    Directive { session: SessionId, text: String },
    Sessions,
    Resume(SessionId),
}
#[derive(Debug)]
enum BackendMessage {
    Record(ResolvedRecord),
    Committed,
    Rejected(String),
    QueryFailed(String),
    ResumeRejected(String),
    Sheet { title: String, lines: Vec<String> },
    Resumed { id: SessionId, workspace: String },
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
    _dirs: &polaris_config::Dirs,
) -> Result<(), CliError> {
    let id = match args.session {
        Some(id) => id,
        None => SessionId::new_v7(),
    };
    let mut session = Some(OfflineSession::open(&args.state_dir, &args.workspace, id).await?);
    let workspace = session
        .as_ref()
        .ok_or(CliError::Unavailable("session missing"))?
        .workspace()
        .display()
        .to_string();
    let (commands_tx, mut commands_rx) = mpsc::channel(4);
    // At most 32 resolved 64 KiB payloads, plus fixed envelopes: bounded independent intake.
    let (events_tx, events_rx) = mpsc::channel(32);
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let options = render_options(args);
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
            commands_tx,
            events_rx,
            signal_stop,
        )
    });
    let mut ui_observed = false;
    let backend_result=async {loop {
  let active=session.as_mut().ok_or(CliError::Unavailable("session owner is absent"))?;
  tokio::select! {
   result=&mut ui=>{ui_observed=true;break result.map_err(|_|CliError::Unavailable("terminal owner failed"))?},
   signal=crate::app::shutdown_signal()=>{
    signal?;stop_tx.send_replace(true);break Ok(());
   },
   changed=stop_rx.changed()=>{if changed.is_err()||*stop_rx.borrow(){break Ok(())}},
   item=active.next_record()=>match item {
    Ok(Some(item))=>{if let Err(error)=send_message(&events_tx,BackendMessage::Record(item)){break Err(error)}},
    Ok(None)=>break Err(CliError::Unavailable("kernel event stream closed")),
    Err(error)=>break Err(error),
   },
   request=commands_rx.recv()=>{
    let Some(request)=request else {break Ok(())};
    match request {
     BackendRequest::Directive{session:expected,text}=>{
      if expected!=active.id {send_message(&events_tx,BackendMessage::Rejected("Session changed; directive was not executed".to_owned()))?;continue}
      let result=active.apply_directive(&text).await;
      let message=match result{Ok(_)=>BackendMessage::Committed,Err(error)=>BackendMessage::Rejected(error.to_string())};
      if let Err(error)=send_message(&events_tx,message){break Err(error)}
     },
     BackendRequest::Sessions=>{
      let result=active.list_sessions().await;
      let message=match result{Ok(rows)=>BackendMessage::Sheet{title:"Sessions".to_owned(),lines:rows.into_iter().map(|s|format!("{}  {:?}  {}  {}",s.id,s.status,s.name,s.workspace.display())).collect()},Err(error)=>BackendMessage::QueryFailed(error.to_string())};
      if let Err(error)=send_message(&events_tx,message){break Err(error)}
     },
     BackendRequest::Resume(target)=>{
      if !active.session_exists(target).await?{send_message(&events_tx,BackendMessage::ResumeRejected("Unknown session; no new session was created".to_owned()))?;continue}
      active.close().await?;
      drop(session.take());
      match OfflineSession::open(&root,&config_workspace,target).await {
       Ok(next)=>{let workspace=next.workspace().display().to_string();session=Some(next);send_message(&events_tx,BackendMessage::Resumed{id:target,workspace})?;},
       Err(error)=>break Err(error),
      }
     },
    }
   }
  }
 }}.await;
    stop_tx.send_replace(true);
    let cleanup = match session.as_mut() {
        Some(session) => session.close().await,
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
        Ok(Self {
            id,
            workspace,
            views,
            empty,
            composer,
            pending: None,
            switch_pending: false,
            notice: "Offline local session - model execution unavailable; Ctrl+J newline"
                .to_owned(),
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
        })
    }
    fn view(&self) -> &ViewState {
        match self.views.get(self.id) {
            Some(v) => v,
            None => &self.empty,
        }
    }
    fn view_mut(&mut self) -> &mut ViewState {
        match self.views.get_mut(self.id) {
            Some(v) => v,
            None => &mut self.empty,
        }
    }
    fn refresh_completion(&mut self) {
        self.completion = if self.sheet.is_some() || self.completion_dismissed {
            Vec::new()
        } else {
            completion::complete(self.composer.text(), self.composer.cursor(), &self.paths)
        };
        if self
            .completion_selected
            .is_some_and(|i| i >= self.completion.len())
        {
            self.completion_selected = None
        }
    }
    fn receive(&mut self, message: BackendMessage) -> Result<(), CliError> {
        match message {
            BackendMessage::Record(item) => {
                if let Some(text) = item.text.as_ref() {
                    if item.record.session() == self.id {
                        self.view().check_user_capacity(text).map_err(ui_error)?;
                    }
                }
                self.views
                    .apply_selected(&item.record, self.id)
                    .map_err(|e| CliError::Ui(format!("projection requires recovery: {e}")))?;
                if item.record.session() == self.id {
                    if let Some(text) = item.text {
                        self.view_mut().push_user(text).map_err(ui_error)?;
                    }
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
            BackendMessage::Resumed { id, workspace } => {
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
                "No task execution profile is attached.".to_owned(),
                "Plan/goal task controls require P3.".to_owned(),
            ],
            PanelKind::Agents => vec![
                format!("Main - local control session {}", self.id),
                "Child-frame execution is unavailable in this profile.".to_owned(),
            ],
            PanelKind::Questions => vec![
                "Human request service is not connected.".to_owned(),
                "No pending request count can be inferred.".to_owned(),
                "Live ask/answer requires P3-X-5.".to_owned(),
            ],
            PanelKind::Review => vec![
                format!("Approved diffs observed: {}", view.status.diffs_approved),
                format!("Rejected diffs observed: {}", view.status.diffs_rejected),
                "Interactive execution approval is not connected.".to_owned(),
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
                "Folded-reference navigation requires the context profile.".to_owned(),
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
                "Provider quota, model capacity and billing: unavailable".to_owned(),
            ],
        }
    }
}

fn ui_error(error: impl std::fmt::Display) -> CliError {
    CliError::Ui(error.to_string())
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
    let parsed = match polaris_kernel::directives::parse(
        text,
        &polaris_kernel::directives::ParseLimits::default(),
    ) {
        Ok(parsed) => parsed,
        Err(error) => {
            state.notice = format!("Invalid input: {error}");
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
            SlashCommand::Status | SlashCommand::Settings => {
                let mut lines = vec![
                    format!("Session {}", state.id),
                    format!("Workspace {}", state.workspace),
                    "Actual configuration source layers:".to_owned(),
                ];
                lines.extend(state.provenance.clone());
                lines.push("Preference editing is unavailable in this slice; existing config layers remain authoritative.".to_owned());
                open_sheet(state, "Status / settings", lines)
            }
            SlashCommand::Stats | SlashCommand::Usage | SlashCommand::Quota => {
                let view = state.view();
                let lines=vec![format!("Observed model requests: {}",view.hud.requests),format!("Observed directives: {}",view.status.directives_applied),format!("Observed generated tokens: {}",view.hud.completion_tokens),"Provider capacity, current quotas, prices and billing availability are not reported by this local profile.".to_owned()];
                open_sheet(state, "Observed statistics", lines)
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
                state.view_mut().conversation.clear();
                state.history_scroll = 0;
                state.notice =
                    "Visible conversation cleared; durable audit records retained".to_owned()
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
            SlashCommand::Model(_)
            | SlashCommand::Plan
            | SlashCommand::Goal(_)
            | SlashCommand::Diff
            | SlashCommand::Rollback(_)
            | SlashCommand::Undo
            | SlashCommand::SessionReplay { .. }
            | SlashCommand::Providers
            | SlashCommand::Onboarding
            | SlashCommand::NvProfile
            | SlashCommand::NvVerify
            | SlashCommand::NebiusStatus
            | SlashCommand::ContreeBranches
            | SlashCommand::DatalabSync
            | SlashCommand::DatalabFinetune
            | SlashCommand::ConcurrencyStats
            | SlashCommand::CacheStats
            | SlashCommand::CachePurge
            | SlashCommand::Research(_)
            | SlashCommand::ContextMap
            | SlashCommand::ContextExpand(_)
            | SlashCommand::ParallelDag
            | SlashCommand::Screenshot
            | SlashCommand::Browser(_)
            | SlashCommand::UiDiff
            | SlashCommand::Hack
            | SlashCommand::Redteam
            | SlashCommand::SearchLocal(_)
            | SlashCommand::Ci
            | SlashCommand::CiRun
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
    } else {
        state.composer.cancel_submit(prepared);
        state.notice="This local profile accepts one standalone $directive. Model task execution is unavailable; draft retained.".to_owned()
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
    match &state.last_metadata {
        Some(m) => m.composer_rect,
        None => Rect::new(2, 0, 76, 3),
    }
}
fn accept_completion(state: &mut UiState) -> Result<(), CliError> {
    if let Some(index) = state.completion_selected {
        if let Some(s) = state.completion.get(index) {
            let range = s.range.clone();
            let insert = s.insert.clone();
            state
                .composer
                .apply_completion(range, &insert)
                .map_err(ui_error)?;
        }
    }
    state.completion_dismissed = true;
    state.refresh_completion();
    Ok(())
}
fn handle_key(
    state: &mut UiState,
    key: KeyEvent,
    commands: &mpsc::Sender<BackendRequest>,
) -> Result<(bool, bool), CliError> {
    if key.kind == KeyEventKind::Release {
        return Ok((false, false));
    }
    let control = key.modifiers.contains(KeyModifiers::CONTROL);
    if key.code == KeyCode::Esc
        || (control && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C')))
    {
        if state.sheet.take().is_some() {
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
        state.notice = "No model work is running; draft retained".to_owned();
        return Ok((false, true));
    }
    state.exit_armed = None;
    if key.code == KeyCode::F(2) {
        let lines = vec![
            format!("Session {}", state.id),
            format!("Workspace {}", state.workspace),
            "Signed local controls active; model execution unavailable.".to_owned(),
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
    if action == Some(keymap::Action::Newline) {
        if let Err(error) = state.composer.newline() {
            state.notice = format!("Edit rejected: {error}");
            return Ok((false, true));
        }
    } else if action == Some(keymap::Action::OpenHelp) {
        open_sheet(state, "Help", help_lines())
    } else if action == Some(keymap::Action::Undo) {
        let _changed = state.composer.undo();
    } else if action == Some(keymap::Action::Redo) {
        let _changed = state.composer.redo();
    } else if action == Some(keymap::Action::ToggleTools) {
        state.view_mut().toggle_all_tools();
    } else if action == Some(keymap::Action::ToggleMode) {
        state.view_mut().toggle_mode();
        state.notice = "Mode preference changed; no model execution profile is attached".to_owned();
    } else if action == Some(keymap::Action::EffortUp) {
        let _changed = state.view_mut().effort_up();
        state.notice = "Effort preference only; model capabilities unavailable".to_owned();
    } else if action == Some(keymap::Action::EffortDown) {
        let _changed = state.view_mut().effort_down();
        state.notice = "Effort preference only; model capabilities unavailable".to_owned();
    } else if action == Some(keymap::Action::Paste) {
        state.notice =
            "Native clipboard adapter is unavailable; use the terminal's paste action".to_owned();
    } else {
        match key.code {
            KeyCode::Backspace => {
                let _changed = state.composer.backspace();
            }
            KeyCode::Delete => {
                let _changed = state.composer.delete();
            }
            KeyCode::Left => state.composer.move_left(),
            KeyCode::Right => state.composer.move_right(),
            KeyCode::Home => state.composer.move_home(),
            KeyCode::End => state.composer.move_end(),
            KeyCode::Up => {
                if first {
                    let _changed = state.composer.history_prev(true);
                } else if let Some(at) =
                    screen::composer_vertical_cursor(&state.composer, area, false)
                {
                    state.composer.set_cursor(at)
                }
            }
            KeyCode::Down => {
                if last {
                    let _changed = state.composer.history_next(true);
                } else if let Some(at) =
                    screen::composer_vertical_cursor(&state.composer, area, true)
                {
                    state.composer.set_cursor(at)
                }
            }
            KeyCode::PageUp => {
                state.history_scroll = state.history_scroll.saturating_add(10).min(4096)
            }
            KeyCode::PageDown => state.history_scroll = state.history_scroll.saturating_sub(10),
            KeyCode::Char(ch) if !control && !key.modifiers.contains(KeyModifiers::ALT) => {
                if let Err(error) = state.composer.insert(&ch.to_string()) {
                    state.notice = format!("Edit rejected: {error}");
                    return Ok((false, true));
                }
            }
            _ => return Ok((false, false)),
        }
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
    ScreenState::new(state.view(), &state.composer)
        .with_workspace(Some(&state.workspace))
        .with_notice(Some(&state.notice))
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
            footer: Some("Up/Down scroll - Enter/Esc close"),
            scroll: sheet.scroll,
        }))
}
fn bounded_area(columns: u16, rows: u16) -> Rect {
    Rect::new(0, 0, columns.clamp(1, 400), rows.clamp(1, 200))
}
fn handle_input(
    state: &mut UiState,
    input: NormalizedInput,
    commands: &mpsc::Sender<BackendRequest>,
    area: &mut Rect,
    options: &RenderOptions,
) -> Result<(bool, bool), CliError> {
    match input {
        NormalizedInput::Key(key) => handle_key(state, key, commands),
        NormalizedInput::Paste(text) => {
            if state.pending.is_some()
                || state.switch_pending
                || state.sheet.is_some()
                || state.panel_focus
            {
                return Ok((false, false));
            }
            match state.composer.insert(&text) {
                Ok(()) => {
                    state.completion_dismissed = false;
                    state.refresh_completion()
                }
                Err(error) => state.notice = format!("Paste rejected: {error}"),
            };
            Ok((false, true))
        }
        NormalizedInput::Resize { columns, rows } => {
            *area = bounded_area(columns, rows);
            state.last_metadata = None;
            if columns > 400 || rows > 200 {
                state.notice = "Viewport limited to 400x200 cells".to_owned()
            }
            Ok((false, true))
        }
        NormalizedInput::Focus { focused } => Ok((false, focused)),
        NormalizedInput::Mouse(mouse) => {
            match mouse.kind {
                MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                    let up = mouse.kind == MouseEventKind::ScrollUp;
                    if let Some(sheet) = state.sheet.as_mut() {
                        sheet.scroll = if up {
                            sheet.scroll.saturating_sub(3)
                        } else {
                            sheet
                                .scroll
                                .saturating_add(3)
                                .min(sheet.lines.len().saturating_sub(1))
                        }
                    } else if state.panel_focus {
                        state.panel_scroll = if up {
                            state.panel_scroll.saturating_sub(3)
                        } else {
                            state
                                .panel_scroll
                                .saturating_add(3)
                                .min(state.panel_lines().len().saturating_sub(1))
                        }
                    } else if !state.completion.is_empty() {
                        state.completion_selected = Some(match state.completion_selected {
                            Some(index) if up => index.saturating_sub(1),
                            Some(index) => index
                                .saturating_add(1)
                                .min(state.completion.len().saturating_sub(1)),
                            None => 0,
                        });
                    } else {
                        state.history_scroll = if up {
                            state.history_scroll.saturating_add(3).min(4096)
                        } else {
                            state.history_scroll.saturating_sub(3)
                        }
                    }
                    Ok((false, true))
                }
                MouseEventKind::Down(crossterm::event::MouseButton::Left)
                    if state.pending.is_none()
                        && !state.switch_pending
                        && state.sheet.is_none()
                        && !state.panel_focus =>
                {
                    // Resolve geometry from current state, including earlier edits in this batch.
                    let lines = state.panel_lines();
                    let footer = footer_text(state);
                    let current = screen::dump_cells(
                        &projection(state, &lines, &footer),
                        options,
                        area.width,
                        area.height,
                    )
                    .map_err(ui_error)?
                    .metadata;
                    if let Some(at) = screen::composer_cursor_at(
                        &state.composer,
                        current.composer_rect,
                        mouse.column,
                        mouse.row,
                    ) {
                        state.composer.set_cursor(at);
                        state.completion_dismissed = false;
                        state.refresh_completion();
                        state.last_metadata = Some(current);
                        Ok((false, true))
                    } else {
                        Ok((false, false))
                    }
                }
                _ => Ok((false, false)),
            }
        }
    }
}
fn terminal_loop(
    id: SessionId,
    workspace: String,
    initial: Option<String>,
    trace_path: Option<PathBuf>,
    options: RenderOptions,
    provenance: Vec<String>,
    commands: mpsc::Sender<BackendRequest>,
    mut events: mpsc::Receiver<BackendMessage>,
    stop: watch::Sender<bool>,
) -> Result<(), CliError> {
    let mut state = UiState::new(id, workspace, initial, provenance)?;
    state.refresh_completion();
    let mut trace = match trace_path {
        Some(path) => Some(KeyTrace::create(&path)?),
        None => None,
    };
    let (columns, rows) = crossterm::terminal::size()?;
    let mut area = bounded_area(columns, rows);
    let guard = TerminalGuard::acquire(CrosstermOps, io::stdout()).map_err(ui_error)?;
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
        let mut scheduler = InputScheduler::new();
        scheduler.mark_dirty();
        while !*stop.borrow() {
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
            if event::poll(timeout)? {
                // Bounded intake. Normalized semantic events keep their relative order.
                for _ in 0..256 {
                    let event = event::read()?;
                    match normalize_event(event) {
                        Ok(input) => scheduler.enqueue(input).map_err(ui_error)?,
                        Err(error) => {
                            state.notice = format!("Input rejected: {error}");
                            scheduler.mark_dirty()
                        }
                    }
                    if !event::poll(Duration::ZERO)? {
                        break;
                    }
                }
            }
        }
        Ok(())
    })();
    stop.send_replace(true);
    let restore = match presenter.into_inner() {
        Ok(mut guard) => guard.close().map_err(ui_error),
        Err(error) => Err(ui_error(error)),
    };
    let trace_result = match trace {
        Some(trace) => trace.finish().map_err(CliError::from),
        None => Ok(()),
    };
    combine_results(result, combine_results(restore, trace_result))
}

fn present_frame<W: Write>(
    terminal: &mut Terminal<CrosstermBackend<polaris_ui_cli::presenter::StagedWriter>>,
    presenter: &mut BufferedPresenter<W>,
    state: &mut UiState,
    options: &RenderOptions,
    area: Rect,
) -> Result<usize, CliError> {
    presenter.begin_frame().map_err(ui_error)?;
    let draw = (|| -> Result<(), CliError> {
        if terminal.get_frame().area() != area {
            terminal.resize(area)?;
        }
        let lines = state.panel_lines();
        let footer = footer_text(state);
        let screen = projection(state, &lines, &footer);
        let mut metadata = None;
        terminal.draw(|frame| metadata = Some(screen::render(frame, &screen, options)))?;
        state.last_metadata = metadata;
        Ok(())
    })();
    if let Err(error) = draw {
        let abort = presenter.abort_frame().map_err(ui_error);
        combine_results(Err(error), abort)?
    }
    presenter.commit().map_err(ui_error)
}
#[cfg(test)]
#[path = "../tests/controller/mod.rs"]
mod controller_tests;
