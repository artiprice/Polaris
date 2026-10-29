//! Builds the window, connects it to a [`SessionController`] and a backend link, and runs it.

use core::cell::RefCell;
use core::time::Duration;
use std::path::Path;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use polaris_sdk::ids::ModelId;
use polaris_ui_model::completion::DIRECTIVE_HELP;
use polaris_ui_model::view::EntryKind;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::aurora_driver::{AuroraDriver, FRAME_STEP};
use crate::backend::{BackendEvent, BackendLink, BackendRequest};
use crate::generated::{
    BlockRow, DiffLine, DirectiveChip, EntryRow, ModelRowUi, SearchHitUi,
    SessionGroup as SlintGroup, SessionRowUi,
};
use crate::session_view::{Screen, SessionController};
use crate::{AppWindow, FrameHistogram, GuiError};

/// Monospace font the aurora glyphs are baked from (the same file the UI registers).
const AURORA_FONT: &[u8] = include_bytes!("../assets/fonts/jetbrains-mono.ttf");

/// How often backend events are drained on the UI thread.
const DRAIN_STEP: Duration = Duration::from_millis(30);

/// One model the composer menu offers. The list comes from the application's configuration
/// (`[nebius.effort]` in `polaris-config`), never from this crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// Model id as configured (for example a Token Factory catalog id).
    pub id: String,
    /// Short label for the pill and status line.
    pub label: String,
}

/// Session directives offered as chips in the composer tray (help text from
/// [`DIRECTIVE_HELP`]).
pub const TRAY_DIRECTIVES: [&str; 5] = ["websearch", "subagents", "skills", "mcp", "rag"];

/// What the window shows that comes from outside the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppOptions {
    /// Application version shown in the Settings panel's About section.
    pub version: String,
    /// Workspace directory shown in the Context panel and status line.
    pub workspace: String,
    /// Session store directory shown in the session sidebar.
    pub state_dir: String,
    /// Draw one still aurora frame instead of animating.
    pub reduced_motion: bool,
    /// Models offered by the composer menu, in display order (must not be empty).
    pub models: Vec<ModelOption>,
    /// Index into `models` selected at start.
    pub default_model: usize,
}

/// State left when the window closes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppReport {
    /// Aurora frame-time statistics of the run.
    pub histogram: FrameHistogram,
    /// Selected model (index into [`AppOptions::models`]).
    pub model_index: i32,
    /// Selected mode: 0 plan, 1 auto.
    pub mode_index: i32,
    /// Selected effort: 0 low .. 4 max.
    pub effort_index: i32,
}

/// Creates the window with its static data, without a backend or the aurora.
///
/// # Errors
/// [`GuiError::Platform`] when no windowing platform is available, [`GuiError::NoModels`] when
/// the configured model list is empty.
pub fn build_window(options: &AppOptions) -> Result<AppWindow, GuiError> {
    let ui = AppWindow::new()?;
    ui.set_app_version(SharedString::from(options.version.as_str()));
    ui.set_workspace_path(SharedString::from(options.workspace.as_str()));
    ui.set_state_dir(SharedString::from(options.state_dir.as_str()));
    ui.set_motion_static(options.reduced_motion);
    ui.set_status_workspace(SharedString::from(folder_label(&options.workspace)));
    if options.models.is_empty() {
        return Err(GuiError::NoModels);
    }
    let names: Vec<SharedString> = options
        .models
        .iter()
        .map(|model| SharedString::from(model.id.as_str()))
        .collect();
    let labels: Vec<SharedString> = options
        .models
        .iter()
        .map(|model| SharedString::from(model.label.as_str()))
        .collect();
    ui.set_model_names(ModelRc::new(VecModel::from(names)));
    ui.set_model_short_names(ModelRc::new(VecModel::from(labels)));
    let default_model = match options.models.get(options.default_model) {
        Some(_) => options.default_model,
        None => 0,
    };
    ui.set_model_index(i32::try_from(default_model).map_err(|_| GuiError::NoModels)?);
    let chips: Vec<DirectiveChip> = TRAY_DIRECTIVES
        .iter()
        .filter_map(|name| DIRECTIVE_HELP.iter().find(|help| help.name == *name))
        .map(|help| DirectiveChip {
            name: SharedString::from(help.name),
            description: SharedString::from(help.description),
        })
        .collect();
    ui.set_directives(ModelRc::new(VecModel::from(chips)));
    Ok(ui)
}

fn folder_label(path: &str) -> String {
    match Path::new(path).file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_owned(),
    }
}

fn now_ms() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX),
        Err(_) => 0,
    }
}

fn entry_kind(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::User => "user",
        EntryKind::Assistant => "assistant",
        EntryKind::System => "system",
        EntryKind::Warning => "warning",
        EntryKind::Error => "error",
    }
}

/// Copies the controller's snapshot into window properties.
fn sync(ui: &AppWindow, controller: &SessionController, fallback_workspace: &str) {
    ui.set_in_session(controller.screen() == Screen::Session);
    if ui.get_draft().as_str() != controller.draft() {
        ui.set_draft(SharedString::from(controller.draft()));
    }
    ui.set_notice(SharedString::from(controller.notice()));
    ui.set_pending(controller.pending());
    ui.set_working(controller.working());
    ui.set_status_connection(SharedString::from(controller.status_line()));
    ui.set_status_state(SharedString::from(controller.status_state()));
    ui.set_review_title(SharedString::from(controller.review().unwrap_or_default()));
    ui.set_starting(controller.starting());
    let (provider_models, current) = controller.composer_models();
    if !provider_models.is_empty() {
        let labels: Vec<SharedString> = provider_models
            .iter()
            .map(|name| SharedString::from(crate::short_model_label(name)))
            .collect();
        let names: Vec<SharedString> = provider_models
            .iter()
            .map(|name| SharedString::from(name.as_str()))
            .collect();
        ui.set_model_names(ModelRc::new(VecModel::from(names)));
        ui.set_model_short_names(ModelRc::new(VecModel::from(labels)));
        if let Some(index) = current.and_then(|index| i32::try_from(index).ok()) {
            ui.set_model_index(index);
        }
        if let Ok(index) = i32::try_from(controller.effort_index()) {
            ui.set_effort_index(index);
        }
    }
    ui.set_session_title(SharedString::from(controller.title()));
    let (workspace, branch) = match controller.open_session() {
        Some(open) => (
            folder_label(&open.workspace),
            match &open.branch {
                Some(branch) => branch.clone(),
                None => String::new(),
            },
        ),
        None => (folder_label(fallback_workspace), String::new()),
    };
    ui.set_status_workspace(SharedString::from(workspace));
    ui.set_status_branch(SharedString::from(branch));
    let entries: Vec<EntryRow> = controller
        .entries()
        .iter()
        .map(|entry| EntryRow {
            kind: SharedString::from(entry_kind(entry.kind)),
            text: SharedString::from(entry.text.as_str()),
            blocks: ModelRc::new(VecModel::from(if entry.kind == EntryKind::Assistant {
                crate::markdown::blocks(&entry.text)
                    .into_iter()
                    .map(|block| BlockRow {
                        kind: SharedString::from(block.kind),
                        lines: ModelRc::new(VecModel::from(if block.kind == "diff" {
                            block
                                .text
                                .lines()
                                .map(|line| DiffLine {
                                    kind: SharedString::from(crate::markdown::diff_line_kind(line)),
                                    text: SharedString::from(line),
                                })
                                .collect()
                        } else {
                            Vec::new()
                        })),
                        text: SharedString::from(block.text),
                    })
                    .collect()
            } else {
                Vec::new()
            })),
        })
        .collect();
    ui.set_entries(ModelRc::new(VecModel::from(entries)));
    let models: Vec<ModelRowUi> = controller
        .model_rows()
        .into_iter()
        .map(|row| ModelRowUi {
            effort: SharedString::from(row.effort),
            model: SharedString::from(row.model),
            active: row.active,
        })
        .collect();
    ui.set_model_rows(ModelRc::new(VecModel::from(models)));
    let usage: Vec<SharedString> = controller
        .usage_lines()
        .into_iter()
        .map(SharedString::from)
        .collect();
    ui.set_usage_lines(ModelRc::new(VecModel::from(usage)));
    let groups: Vec<SlintGroup> = controller
        .groups(now_ms())
        .into_iter()
        .map(|group| SlintGroup {
            name: SharedString::from(group.name),
            path: SharedString::from(group.path),
            rows: ModelRc::new(VecModel::from(
                group
                    .rows
                    .into_iter()
                    .map(|row| SessionRowUi {
                        id: SharedString::from(row.id),
                        title: SharedString::from(row.title),
                        when: SharedString::from(row.when),
                        active: row.active,
                    })
                    .collect::<Vec<_>>(),
            )),
        })
        .collect();
    ui.set_groups(ModelRc::new(VecModel::from(groups)));
    let hits: Vec<SearchHitUi> = controller
        .hits()
        .iter()
        .map(|hit| SearchHitUi {
            id: SharedString::from(hit.session_id.as_str()),
            title: SharedString::from(if hit.title.trim().is_empty() {
                crate::session_view::UNTITLED
            } else {
                hit.title.as_str()
            }),
            snippet: SharedString::from(hit.snippet.as_str()),
        })
        .collect();
    ui.set_hits(ModelRc::new(VecModel::from(hits)));
}

/// Sends requests; a closed backend becomes a visible notice instead of a silent drop.
fn send_all(
    requests: &Sender<BackendRequest>,
    batch: Vec<BackendRequest>,
    controller: &mut SessionController,
) {
    for request in batch {
        if requests.send(request).is_err() {
            let _ignored = controller.apply(BackendEvent::Failed(String::from(
                "The session backend stopped; restart Polaris to record messages.",
            )));
            break;
        }
    }
}

/// Keeps the controller, the channels and the drain timer alive for the window's lifetime.
pub struct Wiring {
    controller: Rc<RefCell<SessionController>>,
    requests: Sender<BackendRequest>,
    events: Rc<Receiver<BackendEvent>>,
    workspace: Rc<String>,
    timer: Timer,
}

impl core::fmt::Debug for Wiring {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Wiring")
            .field("controller", &self.controller)
            .field("workspace", &self.workspace)
            .field("timer_running", &self.timer.running())
            .finish_non_exhaustive()
    }
}

impl Wiring {
    /// Sends `Close` so the backend can persist the session before exit.
    pub fn close(&self) {
        self.timer.stop();
        let _closed = self.requests.send(BackendRequest::Close);
    }

    /// Runs `f` with the controller (tests and diagnostics).
    pub fn with_controller<R>(&self, f: impl FnOnce(&SessionController) -> R) -> Option<R> {
        match self.controller.try_borrow() {
            Ok(controller) => Some(f(&controller)),
            Err(_) => None,
        }
    }

    /// Drains pending backend events now instead of waiting for the timer (tests).
    pub fn pump(&self, ui: &AppWindow) {
        drain(
            ui,
            &self.controller,
            &self.events,
            &self.requests,
            &self.workspace,
        );
    }
}

fn drain(
    ui: &AppWindow,
    controller: &Rc<RefCell<SessionController>>,
    events: &Receiver<BackendEvent>,
    requests: &Sender<BackendRequest>,
    workspace: &str,
) {
    let Ok(mut controller) = controller.try_borrow_mut() else {
        return;
    };
    let mut changed = false;
    loop {
        match events.try_recv() {
            Ok(event) => {
                changed = true;
                let follow_up = controller.apply(event);
                send_all(requests, follow_up, &mut controller);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                if !controller.notice().contains("backend stopped") {
                    changed = true;
                    let _ignored = controller.apply(BackendEvent::Failed(String::from(
                        "The session backend stopped; restart Polaris to record messages.",
                    )));
                }
                break;
            }
        }
    }
    if changed {
        sync(ui, &controller, workspace);
    } else if controller.working() {
        // Keep the elapsed time in the status bar moving while a reply streams.
        ui.set_status_connection(SharedString::from(controller.status_line()));
    }
}

/// Connects the window's callbacks to a controller and the backend link, and starts draining
/// backend events on the UI thread. The window keeps working without events (offline welcome).
///
/// # Errors
/// [`GuiError::Model`] when the selected model id is invalid, [`GuiError::NoModels`] when the
/// list is empty.
pub fn attach(ui: &AppWindow, link: BackendLink, options: &AppOptions) -> Result<Wiring, GuiError> {
    let selected = match options.models.get(options.default_model) {
        Some(model) => model,
        None => options.models.first().ok_or(GuiError::NoModels)?,
    };
    let model = ModelId::new(&selected.id).map_err(|_| GuiError::Model(selected.id.clone()))?;
    let controller = Rc::new(RefCell::new(SessionController::new(model)));
    let BackendLink { requests, events } = link;
    let events = Rc::new(events);
    let workspace = Rc::new(options.workspace.clone());

    macro_rules! on {
        ($setter:ident, |$c:ident $(, $arg:ident)*| $body:expr) => {{
            let weak = ui.as_weak();
            let controller = Rc::clone(&controller);
            let requests = requests.clone();
            let workspace = Rc::clone(&workspace);
            ui.$setter(move |$($arg),*| {
                let Some(ui) = weak.upgrade() else { return; };
                let Ok(mut $c) = controller.try_borrow_mut() else { return; };
                let batch: Vec<BackendRequest> = $body;
                send_all(&requests, batch, &mut $c);
                sync(&ui, &$c, &workspace);
            });
        }};
    }
    on!(on_draft_edited, |c, text| {
        c.edit(text.as_str());
        Vec::new()
    });
    on!(on_submit, |c| c.submit());
    on!(on_interrupt, |c| c.interrupt());
    on!(on_review_apply, |c, note| c
        .decide_review(true, note.as_str()));
    on!(on_review_decline, |c, note| c
        .decide_review(false, note.as_str()));
    on!(on_choose_model, |c, index| match usize::try_from(index) {
        Ok(index) => c.choose_model(index),
        Err(_) => Vec::new(),
    });
    on!(on_usage_opened, |c| c.refresh_usage());
    on!(on_mode_chosen, |c, index| c.set_mode(index));
    on!(on_model_chosen, |c, index| match usize::try_from(index) {
        Ok(index) => c.choose_composer_model(index),
        Err(_) => Vec::new(),
    });
    on!(on_effort_chosen, |c, index| match usize::try_from(index) {
        Ok(index) => c.choose_effort(index),
        Err(_) => Vec::new(),
    });
    on!(on_new_conversation, |c| c.new_session());
    on!(on_open_session, |c, id| c.open_listed(id.as_str()));
    on!(on_search_edited, |c, query| c.search(query.as_str()));
    on!(on_show_sessions, |c| c.show_sessions());

    // Initial list for the sidebar (registry only, no kernel).
    if requests.send(BackendRequest::List).is_err() {
        tracing::warn!("session backend unavailable at startup");
    }

    let timer = Timer::default();
    let weak = ui.as_weak();
    let drain_controller = Rc::clone(&controller);
    let drain_requests = requests.clone();
    let drain_events = Rc::clone(&events);
    let drain_workspace = Rc::clone(&workspace);
    timer.start(TimerMode::Repeated, DRAIN_STEP, move || {
        if let Some(ui) = weak.upgrade() {
            drain(
                &ui,
                &drain_controller,
                &drain_events,
                &drain_requests,
                &drain_workspace,
            );
        }
    });
    Ok(Wiring {
        controller,
        requests,
        events,
        workspace,
        timer,
    })
}

/// Opens the window, runs the event loop until it is closed, then closes the session.
///
/// # Errors
/// [`GuiError`] when the platform cannot start or the aurora font cannot be baked.
pub fn run_app(options: &AppOptions, link: BackendLink) -> Result<AppReport, GuiError> {
    let ui = build_window(options)?;
    let wiring = attach(&ui, link, options)?;
    let driver = Rc::new(RefCell::new(AuroraDriver::new(
        AURORA_FONT,
        options.reduced_motion,
    )?));

    let timer = Timer::default();
    let weak = ui.as_weak();
    let tick_driver = Rc::clone(&driver);
    let mut last_summary = Instant::now();
    timer.start(TimerMode::Repeated, FRAME_STEP, move || {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let Ok(mut driver) = tick_driver.try_borrow_mut() else {
            return;
        };
        if let Err(error) = driver.tick(&ui) {
            tracing::warn!(%error, "aurora frame failed; background stays on the last frame");
        }
        if last_summary.elapsed().as_secs() >= 5 {
            last_summary = Instant::now();
            tracing::debug!(histogram = %driver.histogram().summary(), "aurora frame times");
        }
    });

    ui.run()?;
    timer.stop();
    wiring.close();

    let histogram = match driver.try_borrow() {
        Ok(driver) => driver.histogram().clone(),
        Err(_) => FrameHistogram::default(),
    };
    tracing::info!(histogram = %histogram.summary(), "aurora frame times at exit");
    Ok(AppReport {
        histogram,
        model_index: ui.get_model_index(),
        mode_index: ui.get_mode_index(),
        effort_index: ui.get_effort_index(),
    })
}
