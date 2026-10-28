//! GUI Composer (descriptor: specs/descriptors/ui_gui_composer.json).
//!
//! Real `AppWindow` on Slint's headless testing backend, wired to in-memory channels instead of
//! the desktop bridge, driven through accessible labels and simulated keys.

use std::error::Error;
use std::sync::mpsc::{Receiver, channel};

use i_slint_backend_testing::ElementHandle;
use polaris_ui_gui::backend::{BackendEvent, BackendLink, BackendRequest};
use polaris_ui_gui::{AppOptions, AppWindow, Wiring, attach, build_window};
use slint::ComponentHandle as _;
use slint::platform::PointerEventButton;

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
    _events: std::sync::mpsc::Sender<BackendEvent>,
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
    // The startup sidebar refresh.
    assert_eq!(
        request_rx.try_recv()?,
        BackendRequest::List,
        "startup list request"
    );
    Ok(Harness {
        ui,
        wiring,
        requests: request_rx,
        _events: event_tx,
    })
}

fn element(ui: &AppWindow, label: &str) -> Result<ElementHandle, Box<dyn Error>> {
    let mut found = ElementHandle::find_by_accessible_label(ui, label);
    let Some(handle) = found.next() else {
        return Err(format!("no element labelled {label:?}").into());
    };
    Ok(handle)
}

/// Types `text` through the public window event API (press + release per character).
fn type_text(ui: &AppWindow, text: &str) {
    for ch in text.chars() {
        let key: slint::SharedString = ch.into();
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: key.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: key });
    }
}

fn drained(requests: &Receiver<BackendRequest>) -> Vec<BackendRequest> {
    requests.try_iter().collect()
}

#[test]
fn send_disabled_until_draft_has_text() -> TestResult {
    let h = harness()?;
    let send = element(&h.ui, "Send message")?;
    assert_eq!(
        send.accessible_enabled(),
        Some(false),
        "empty draft: disabled"
    );
    h.ui.set_draft("hello".into());
    assert_eq!(send.accessible_enabled(), Some(true), "with text: enabled");
    h.ui.set_pending(true);
    let waiting = element(&h.ui, "Send message — waiting for the previous message")?;
    assert_eq!(
        waiting.accessible_enabled(),
        Some(false),
        "pending: disabled"
    );
    drop(h.wiring);
    Ok(())
}

#[test]
fn send_action_invokes_submit() -> TestResult {
    let h = harness()?;
    h.ui.set_draft("plan tomorrow".into());
    h.ui.invoke_draft_edited("plan tomorrow".into());
    element(&h.ui, "Send message")?.invoke_accessible_default_action();
    assert_eq!(
        drained(&h.requests),
        vec![
            BackendRequest::Open { resume: None },
            BackendRequest::Submit {
                text: String::from("plan tomorrow")
            }
        ],
        "send opens a session and submits once"
    );
    assert!(
        h.ui.get_in_session(),
        "window switched to the session screen"
    );
    Ok(())
}

#[test]
fn enter_submits_from_the_input() -> TestResult {
    let h = harness()?;
    element(&h.ui, "Message for Polaris")?.mock_single_click(PointerEventButton::Left);
    type_text(&h.ui, "hi");
    assert_eq!(h.ui.get_draft(), "hi", "typed text reaches the draft");
    type_text(&h.ui, "\n");
    let requests = drained(&h.requests);
    assert!(
        requests.contains(&BackendRequest::Submit {
            text: String::from("hi")
        }),
        "Return submits: {requests:?}"
    );
    Ok(())
}

#[test]
fn mode_chip_toggles_plan_and_auto() -> TestResult {
    let h = harness()?;
    assert_eq!(h.ui.get_mode_index(), 0, "starts in plan");
    element(&h.ui, "Mode: plan, asks before acting")?.invoke_accessible_default_action();
    assert_eq!(h.ui.get_mode_index(), 1, "auto after toggle");
    element(&h.ui, "Mode: auto")?.invoke_accessible_default_action();
    assert_eq!(h.ui.get_mode_index(), 0, "back to plan");
    Ok(())
}

#[test]
fn tray_chip_appends_directive() -> TestResult {
    let h = harness()?;
    h.ui.set_draft("search the docs".into());
    element(&h.ui, "Insert $websearch — Enable web search")?.invoke_accessible_default_action();
    assert_eq!(
        h.ui.get_draft(),
        "search the docs $websearch ",
        "directive appended with a separating space"
    );
    Ok(())
}

#[test]
fn attach_and_mic_are_disabled_with_reasons() -> TestResult {
    let h = harness()?;
    let attach = element(
        &h.ui,
        "Attach files — Attachments need the model/ingest profile (not set up yet)",
    )?;
    assert_eq!(attach.accessible_enabled(), Some(false), "attach disabled");
    let mic = element(&h.ui, "Voice input — Speech input is not available yet")?;
    assert_eq!(mic.accessible_enabled(), Some(false), "mic disabled");
    Ok(())
}

#[test]
fn model_menu_selects_model_and_effort() -> TestResult {
    let h = harness()?;
    assert_eq!(
        h.ui.get_model_index(),
        1,
        "configured default model selected"
    );
    element(&h.ui, "Model and effort: model-high")?.invoke_accessible_default_action();
    assert!(h.ui.get_popover_open(), "menu opens");
    element(&h.ui, "vendor/model-max")?.invoke_accessible_default_action();
    element(&h.ui, "max")?.invoke_accessible_default_action();
    assert_eq!(h.ui.get_model_index(), 2, "model selected");
    assert_eq!(h.ui.get_effort_index(), 4, "effort selected");
    element(&h.ui, "Model and effort: model-max")?;
    Ok(())
}

#[test]
fn menu_lists_exactly_the_configured_models() -> TestResult {
    let h = harness()?;
    let ids: Vec<String> = slint::Model::iter(&h.ui.get_model_names())
        .map(|id| id.to_string())
        .collect();
    assert_eq!(
        ids,
        vec!["vendor/model-low", "vendor/model-high", "vendor/model-max"],
        "the menu shows the configured list, nothing built in"
    );
    element(&h.ui, "Model and effort: model-high")?.invoke_accessible_default_action();
    for id in &ids {
        element(&h.ui, id)?;
    }
    Ok(())
}

#[test]
fn empty_model_list_is_an_error() -> TestResult {
    i_slint_backend_testing::init_no_event_loop();
    let options = AppOptions {
        version: String::from("0.0.0-test"),
        workspace: String::from("C:/work/polaris"),
        state_dir: String::from("C:/work/polaris/.polaris/state"),
        reduced_motion: true,
        models: Vec::new(),
        default_model: 0,
    };
    assert!(
        matches!(
            build_window(&options),
            Err(polaris_ui_gui::GuiError::NoModels)
        ),
        "no configured model must not silently show an empty menu"
    );
    Ok(())
}

#[test]
fn welcome_composer_has_focus_on_start() -> TestResult {
    let h = harness()?;
    type_text(&h.ui, "typed without clicking");
    assert_eq!(
        h.ui.get_draft(),
        "typed without clicking",
        "the welcome composer is focused when the window opens"
    );
    Ok(())
}
