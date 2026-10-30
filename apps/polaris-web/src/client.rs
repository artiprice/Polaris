//! The browser side: the access token from the URL fragment, the event stream, the JSON API
//! and the service worker. Compiled only for the `hydrate` (wasm32) build.

use std::cell::RefCell;

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{EventSource, MessageEvent, Request, RequestInit, Response};

use crate::events::{SessionRow, WebEvent};
use crate::state::{Effect, UiState};

thread_local! {
    static TOKEN: RefCell<String> = const { RefCell::new(String::new()) };
}

const NO_TOKEN: &str =
    "This page has no valid access token. Open the link that polaris-web printed.";

/// Connect the page: token, service worker, event stream and the session list.
pub fn start(ui: RwSignal<UiState>) {
    let token = read_token();
    TOKEN.with(|cell| cell.borrow_mut().clone_from(&token));
    register_service_worker();
    if token.is_empty() {
        ui.update(|state| state.notice = Some(NO_TOKEN.to_owned()));
        return;
    }
    if let Err(error) = connect(ui, &token) {
        ui.update(|state| state.notice = Some(format!("Event stream failed: {error:?}")));
    }
    run(ui, vec![Effect::RefreshSessions]);
}

/// The token from `#token=…` (then kept in this tab's storage and removed from the address
/// bar), else the stored one.
fn read_token() -> String {
    let Some(window) = web_sys::window() else {
        return String::new();
    };
    let storage = window.session_storage().ok().flatten();
    let hash = window.location().hash().unwrap_or_default();
    if let Some(token) = hash
        .strip_prefix("#token=")
        .filter(|token| token.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        if let Some(storage) = storage.as_ref() {
            let _ = storage.set_item("polaris-token", token);
        }
        let path = window.location().pathname().unwrap_or_default();
        if let Ok(history) = window.history() {
            let _ = history.replace_state_with_url(&JsValue::NULL, "", Some(&path));
        }
        return token.to_owned();
    }
    storage
        .and_then(|storage| storage.get_item("polaris-token").ok().flatten())
        .unwrap_or_default()
}

fn register_service_worker() {
    if let Some(window) = web_sys::window() {
        let _registration = window.navigator().service_worker().register("/sw.js");
    }
}

fn connect(ui: RwSignal<UiState>, token: &str) -> Result<(), JsValue> {
    let source = EventSource::new(&format!("/api/events?token={token}"))?;
    let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |message: MessageEvent| {
        let Some(text) = message.data().as_string() else {
            return;
        };
        let Ok(event) = serde_json::from_str::<WebEvent>(&text) else {
            return;
        };
        let effects = ui
            .try_update(|state| state.apply(event))
            .unwrap_or_default();
        run(ui, effects);
    });
    source.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // The stream and its handler live as long as the page.
    on_message.forget();
    std::mem::forget(source);
    Ok(())
}

/// Perform effects without blocking the page.
pub fn run(ui: RwSignal<UiState>, effects: Vec<Effect>) {
    for effect in effects {
        spawn_local(perform(ui, effect));
    }
}

async fn perform(ui: RwSignal<UiState>, effect: Effect) {
    let outcome = match effect {
        Effect::RefreshSessions => match request("GET", "/api/sessions", None).await {
            Ok(text) => {
                if let Ok(rows) = serde_json::from_str::<Vec<SessionRow>>(&text) {
                    ui.update(|state| state.sessions = rows);
                }
                Ok(())
            }
            Err(error) => Err(error),
        },
        Effect::Open { resume } => {
            post("/api/open", &serde_json::json!({ "resume": resume })).await
        }
        Effect::Submit { text, plan } => {
            post(
                "/api/submit",
                &serde_json::json!({ "text": text, "plan": plan }),
            )
            .await
        }
        Effect::Interrupt => post("/api/interrupt", &serde_json::json!({})).await,
        Effect::Review {
            request_id,
            apply,
            note,
        } => {
            post(
                "/api/review",
                &serde_json::json!({ "request_id": request_id, "apply": apply, "note": note }),
            )
            .await
        }
        Effect::Effort(effort) => {
            post("/api/effort", &serde_json::json!({ "effort": effort })).await
        }
    };
    if let Err(message) = outcome {
        ui.update(|state| {
            state.pending = None;
            state.notice = Some(message);
        });
    }
}

async fn post(path: &str, body: &serde_json::Value) -> Result<(), String> {
    request("POST", path, Some(body.to_string()))
        .await
        .map(|_| ())
}

async fn request(method: &str, path: &str, body: Option<String>) -> Result<String, String> {
    let token = TOKEN.with(|cell| cell.borrow().clone());
    let failed = |error: JsValue| format!("Request failed: {error:?}");
    let window = web_sys::window().ok_or("no window")?;
    let init = RequestInit::new();
    init.set_method(method);
    let headers = web_sys::Headers::new().map_err(failed)?;
    headers
        .set("Authorization", &format!("Bearer {token}"))
        .map_err(failed)?;
    if let Some(body) = body {
        headers
            .set("Content-Type", "application/json")
            .map_err(failed)?;
        init.set_body(&JsValue::from_str(&body));
    }
    init.set_headers(&headers);
    let request = Request::new_with_str_and_init(path, &init).map_err(failed)?;
    let response: Response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(failed)?
        .dyn_into()
        .map_err(failed)?;
    if response.status() == 401 {
        return Err(NO_TOKEN.to_owned());
    }
    if !response.ok() {
        return Err(format!("{path} answered {}", response.status()));
    }
    let text = JsFuture::from(response.text().map_err(failed)?)
        .await
        .map_err(failed)?;
    Ok(text.as_string().unwrap_or_default())
}
