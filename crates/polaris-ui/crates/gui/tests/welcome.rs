//! GUI Welcome + session screens (descriptors: specs/descriptors/ui_gui_welcome.json,
//! ui_gui_session_view.json).
//!
//! Runs the real `AppWindow` on Slint's headless testing backend (one backend per test thread),
//! wired to in-memory channels in place of the desktop bridge.

use std::error::Error;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, channel};

use i_slint_backend_testing::ElementHandle;
use polaris_ui_gui::backend::{
    BackendEvent, BackendLink, BackendRequest, MessageKind, SessionSummary,
};
use polaris_ui_gui::{AppOptions, AppWindow, Wiring, attach, build_window};
use slint::ComponentHandle as _;
use slint::Model as _;

type TestResult = Result<(), Box<dyn Error>>;

/// Models as the configuration would supply them (test fixture, not built into the GUI).
fn test_models() -> Vec<polaris_ui_gui::ModelOption> {
    [
        ("vendor/model-low", "model-low"),
        ("vendor/model-high", "model-high"),
        ("vendor/model-max", "model-max"),
    ]
    .into_iter()
    .map(|(id, label)| polaris_ui_gui::ModelOption {
        id: id.to_owned(),
        label: label.to_owned(),
    })
    .collect()
}

struct Harness {
    ui: AppWindow,
    wiring: Wiring,
    requests: Receiver<BackendRequest>,
    events: Sender<BackendEvent>,
}

fn harness() -> Result<Harness, Box<dyn Error>> {
    i_slint_backend_testing::init_no_event_loop();
    let options = AppOptions {
        version: String::from("0.0.0-test"),
        workspace: String::from("C:/work/polaris"),
        state_dir: String::from("C:/work/polaris/.polaris/state"),
        reduced_motion: true,
        models: test_models(),
        default_model: 1,
    };
    let ui = build_window(&options)?;
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    let (request_tx, request_rx) = channel();
    let (event_tx, event_rx) = channel();
    let wiring = attach(
        &ui,
        BackendLink {
            requests: request_tx,
            events: event_rx,
        },
        &options,
    )?;
    let _startup_list = request_rx.try_recv()?;
    Ok(Harness {
        ui,
        wiring,
        requests: request_rx,
        events: event_tx,
    })
}

fn element(ui: &AppWindow, label: &str) -> Result<ElementHandle, Box<dyn Error>> {
    let mut found = ElementHandle::find_by_accessible_label(ui, label);
    let Some(handle) = found.next() else {
        return Err(format!("no element labelled {label:?}").into());
    };
    Ok(handle)
}

fn opened(id: &str) -> BackendEvent {
    BackendEvent::Opened {
        id: id.to_owned(),
        title: String::from("Aurora planning"),
        workspace: PathBuf::from("C:/work/polaris"),
        branch: Some(String::from("main")),
    }
}

#[test]
fn context_panel_toggles() -> TestResult {
    let h = harness()?;
    let toggle = element(&h.ui, "Session context")?;
    toggle.invoke_accessible_default_action();
    assert!(h.ui.get_context_open(), "opens");
    toggle.invoke_accessible_default_action();
    assert!(!h.ui.get_context_open(), "closes");
    Ok(())
}

#[test]
fn about_panel_shows_slint_attribution() -> TestResult {
    let h = harness()?;
    element(&h.ui, "About Polaris")?.invoke_accessible_default_action();
    assert!(h.ui.get_about_open(), "About opens");
    let about_slint = ElementHandle::find_by_element_type_name(&h.ui, "AboutSlint").count();
    assert_eq!(
        about_slint, 1,
        "the About panel must contain exactly one AboutSlint widget"
    );
    Ok(())
}

#[test]
fn unbuilt_pages_say_so() -> TestResult {
    let h = harness()?;
    for page in ["Models", "Usage", "Settings"] {
        element(&h.ui, &format!("{page} · not in the desktop shell yet"))?;
    }
    Ok(())
}

#[test]
fn docked_context_panel_does_not_cover_the_composer() -> TestResult {
    let h = harness()?;
    element(&h.ui, "Session context")?.invoke_accessible_default_action();
    let panel_left = 1280.0 - 24.0 - 320.0;
    let send = element(&h.ui, "Send message")?;
    let send_right = send.absolute_position().x + send.size().width;
    assert!(
        send_right <= panel_left,
        "composer ends at {send_right}, docked panel starts at {panel_left}"
    );
    Ok(())
}

#[test]
fn send_flow_switches_to_session_and_shows_committed_message() -> TestResult {
    let h = harness()?;
    h.ui.set_draft("plan tomorrow".into());
    h.ui.invoke_draft_edited("plan tomorrow".into());
    h.ui.invoke_submit();
    let sent: Vec<BackendRequest> = h.requests.try_iter().collect();
    assert_eq!(sent.len(), 2, "open + submit: {sent:?}");
    assert!(h.ui.get_in_session(), "session screen shown");
    assert!(h.ui.get_starting(), "starting state while the kernel opens");
    assert_eq!(
        h.ui.get_atmosphere_width(),
        0.0,
        "no aurora area in the session screen"
    );

    h.events.send(opened("s1"))?;
    h.events.send(BackendEvent::Message {
        kind: MessageKind::UserInput,
        text: String::from("plan tomorrow"),
    })?;
    h.events.send(BackendEvent::Committed)?;
    h.wiring.pump(&h.ui);

    assert!(!h.ui.get_starting(), "started");
    assert!(!h.ui.get_pending(), "committed");
    assert_eq!(h.ui.get_draft(), "", "draft cleared after commit");
    assert_eq!(
        h.ui.get_session_title(),
        "Aurora planning",
        "title from the session"
    );
    assert_eq!(
        h.ui.get_status_branch(),
        "main",
        "branch in the status line"
    );
    let entries = h.ui.get_entries();
    let texts: Vec<String> = entries.iter().map(|e| e.text.to_string()).collect();
    assert_eq!(texts.len(), 2, "offline note + message: {texts:?}");
    assert_eq!(
        texts.get(1).map(String::as_str),
        Some("plan tomorrow"),
        "message shown"
    );
    let kinds: Vec<String> = entries.iter().map(|e| e.kind.to_string()).collect();
    assert_eq!(kinds, vec!["system", "user"], "kinds");
    Ok(())
}

#[test]
fn sidebar_lists_sessions_and_opens_one() -> TestResult {
    let h = harness()?;
    element(&h.ui, "Sessions")?.invoke_accessible_default_action();
    assert!(h.ui.get_in_session(), "history opens the session screen");
    assert_eq!(
        h.requests.try_recv()?,
        BackendRequest::List,
        "list refreshed"
    );
    h.events.send(BackendEvent::Sessions(vec![SessionSummary {
        id: String::from("s7"),
        title: String::from("Earlier work"),
        workspace: PathBuf::from("C:/work/polaris"),
        branch: None,
        updated_ms: 0,
    }]))?;
    h.wiring.pump(&h.ui);
    let row = element(&h.ui, "Session Earlier work")?;
    row.invoke_accessible_default_action();
    assert_eq!(
        h.requests.try_recv()?,
        BackendRequest::Open {
            resume: Some(String::from("s7"))
        },
        "row resumes the session"
    );
    Ok(())
}

#[test]
fn new_session_returns_to_welcome() -> TestResult {
    let h = harness()?;
    h.ui.invoke_show_sessions();
    h.events.send(opened("s1"))?;
    h.wiring.pump(&h.ui);
    let _drain: Vec<BackendRequest> = h.requests.try_iter().collect();
    element(&h.ui, "New session")?.invoke_accessible_default_action();
    assert!(!h.ui.get_in_session(), "welcome screen again");
    assert_eq!(
        h.requests.try_recv()?,
        BackendRequest::Close,
        "open session closed"
    );
    Ok(())
}

#[test]
fn backend_failure_is_visible() -> TestResult {
    let h = harness()?;
    h.ui.set_draft("hello".into());
    h.ui.invoke_draft_edited("hello".into());
    h.ui.invoke_submit();
    h.events.send(BackendEvent::Failed(String::from(
        "Session store is in use by another Polaris process",
    )))?;
    h.wiring.pump(&h.ui);
    assert!(
        h.ui.get_notice().contains("in use"),
        "notice: {}",
        h.ui.get_notice()
    );
    assert_eq!(h.ui.get_draft(), "hello", "draft kept");
    assert!(!h.ui.get_pending(), "not pending");
    Ok(())
}
