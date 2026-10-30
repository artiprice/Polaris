//! The Leptos web UI: the page state machine, Markdown blocks, and the server-rendered page
//! with its security headers, PWA files and the shared JSON API.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use polaris_web_app::events::WebEvent;
use polaris_web_app::markdown::{Block, Inline, blocks, diff_class};
use polaris_web_app::state::{Connection, Effect, EntryKind, ReviewState, UiState};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn event(json: &str) -> Result<WebEvent, serde_json::Error> {
    serde_json::from_str(json)
}

#[test]
fn a_first_message_opens_a_session_then_submits() -> TestResult {
    let mut ui = UiState::default();
    ui.apply(event(
        r#"{"type":"provider","provider":"openrouter","model":"nvidia/nemotron","effort":"medium","models":[["medium","nvidia/nemotron"]]}"#,
    )?);
    assert_eq!(
        ui.status(0),
        ("ready", "nvidia/nemotron · openrouter".to_owned())
    );
    ui.composer = "fix the typo".to_owned();
    assert_eq!(ui.submit(), vec![Effect::Open { resume: None }]);
    assert!(ui.submit().is_empty(), "one message at a time");
    let opened = ui.apply(event(
        r#"{"type":"opened","id":"s1","title":"","workspace":"C:/work"}"#,
    )?);
    assert_eq!(
        opened,
        vec![
            Effect::Submit {
                text: "fix the typo".to_owned(),
                plan: false
            },
            Effect::RefreshSessions
        ]
    );
    ui.apply(event(
        r#"{"type":"message","kind":"user","text":"fix the typo"}"#,
    )?);
    ui.apply(event(r#"{"type":"committed"}"#)?);
    assert!(ui.composer.is_empty() && ui.pending.is_none());
    Ok(())
}

#[test]
fn a_reply_with_a_review_ends_in_order() -> TestResult {
    let mut ui = UiState::default();
    ui.apply(event(
        r#"{"type":"opened","id":"s1","title":"Notes","workspace":"/w"}"#,
    )?);
    ui.apply(event(r#"{"type":"reply-started"}"#)?);
    assert!(ui.working);
    ui.apply(event(r#"{"type":"reply-text","text":"Updating."}"#)?);
    ui.apply(event(
        r#"{"type":"review","request_id":"r1","prompt":"Change `notes.md`?\n\n```diff\n-a\n+b\n```"}"#,
    )?);
    assert_eq!(ui.status(3).0, "waiting");
    assert_eq!(
        ui.decide("r1", true, "  "),
        vec![Effect::Review {
            request_id: "r1".to_owned(),
            apply: true,
            note: None
        }]
    );
    assert!(ui.decide("r1", true, "").is_empty(), "decided once");
    ui.apply(event(r#"{"type":"review-closed","request_id":"r1"}"#)?);
    ui.apply(event(r#"{"type":"applied","path":"notes.md"}"#)?);
    ui.apply(event(r#"{"type":"reply-text","text":"Done."}"#)?);
    ui.apply(event(
        r#"{"type":"reply-finished","answer":"Done.","summary":"Worked for 2s · 3 requests · 1 file changed"}"#,
    )?);
    let kinds: Vec<EntryKind> = ui.entries.iter().map(|entry| entry.kind.clone()).collect();
    assert_eq!(
        kinds,
        vec![
            EntryKind::Assistant {
                text: "Updating.".to_owned(),
                streaming: false
            },
            EntryKind::Review {
                request_id: "r1".to_owned(),
                prompt: "Change `notes.md`?\n\n```diff\n-a\n+b\n```".to_owned(),
                state: ReviewState::Applied
            },
            EntryKind::Change("Edited notes.md".to_owned()),
            EntryKind::Assistant {
                text: "Done.".to_owned(),
                streaming: false
            },
            EntryKind::System("Worked for 2s · 3 requests · 1 file changed".to_owned()),
        ]
    );
    assert!(!ui.working);
    Ok(())
}

#[test]
fn stopping_offline_and_unknown_events_are_handled() -> TestResult {
    let mut ui = UiState::default();
    ui.apply(event(
        r#"{"type":"provider-unavailable","reason":"provider.order is empty"}"#,
    )?);
    assert_eq!(ui.status(0).0, "offline");
    assert!(matches!(ui.connection, Connection::Offline(_)));
    ui.apply(event(
        r#"{"type":"opened","id":"s1","title":"","workspace":"/w"}"#,
    )?);
    assert!(matches!(
        ui.entries.first().map(|entry| &entry.kind),
        Some(EntryKind::System(_))
    ));
    ui.apply(event(r#"{"type":"reply-started"}"#)?);
    assert_eq!(ui.submit(), vec![Effect::Interrupt]);
    ui.apply(event(
        r#"{"type":"review","request_id":"r9","prompt":"Delete `x`?"}"#,
    )?);
    ui.apply(event(r#"{"type":"reply-failed","reason":"Interrupted"}"#)?);
    assert!(ui.entries.iter().any(|entry| matches!(
        &entry.kind,
        EntryKind::Review {
            state: ReviewState::Withdrawn,
            ..
        }
    )));
    assert!(
        matches!(ui.entries.last().map(|entry| &entry.kind), Some(EntryKind::Warning(text)) if text == "Stopped.")
    );
    assert_eq!(
        event(r#"{"type":"something-new","x":1}"#)?,
        WebEvent::Unknown
    );
    Ok(())
}

#[test]
fn an_attached_page_says_where_messages_are_recorded() -> TestResult {
    let mut ui = UiState::default();
    let reason = "Attached to polarisd: messages are recorded in the daemon's session; model replies are not served in attached mode yet.";
    ui.apply(event(
        &serde_json::json!({"type": "provider-unavailable", "reason": reason}).to_string(),
    )?);
    assert!(ui.attached());
    assert_eq!(
        ui.status(0),
        ("offline", "polarisd · no model replies".to_owned())
    );
    ui.apply(event(
        r#"{"type":"opened","id":"s1","title":"polarisd","workspace":"polaris-main"}"#,
    )?);
    assert!(
        matches!(ui.entries.first().map(|entry| &entry.kind), Some(EntryKind::System(text)) if text == reason)
    );
    let mut local = UiState::default();
    local.apply(event(
        r#"{"type":"provider-unavailable","reason":"provider.order is empty"}"#,
    )?);
    assert!(!local.attached());
    Ok(())
}

#[test]
fn a_command_report_becomes_a_finished_markdown_entry() -> TestResult {
    let mut ui = UiState::default();
    ui.apply(event(
        r#"{"type":"report","title":"CUDA advisor","lines":["Checked 1 CUDA source: 0 warnings, 0 suggestions.","Lexical rules: advice"]}"#,
    )?);
    assert!(matches!(
        ui.entries.last().map(|entry| &entry.kind),
        Some(EntryKind::Assistant { text, streaming: false })
            if text == "**CUDA advisor**\n\n```text\nChecked 1 CUDA source: 0 warnings, 0 suggestions.\nLexical rules: advice\n```"
    ));
    Ok(())
}

#[test]
fn markdown_becomes_blocks_with_diff_classes() {
    assert_eq!(
        blocks("## Plan\nUse `cargo test` **now**.\n- one\n\n```diff\n-old\n+new\n```"),
        vec![
            Block::Heading(vec![Inline::Text("Plan".to_owned())]),
            Block::Paragraph(vec![vec![
                Inline::Text("Use ".to_owned()),
                Inline::Code("cargo test".to_owned()),
                Inline::Text(" now.".to_owned()),
            ]]),
            Block::Item(vec![Inline::Text("one".to_owned())]),
            Block::Code {
                language: "diff".to_owned(),
                lines: vec!["-old".to_owned(), "+new".to_owned()],
            },
        ]
    );
    assert_eq!(diff_class("diff", "+new"), "add");
    assert_eq!(diff_class("diff", "--- a/x"), "");
    assert_eq!(diff_class("rust", "-1"), "");
}

struct Running {
    base: String,
    token: String,
    _dir: tempfile::TempDir,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: tokio::task::JoinHandle<Result<(), polaris_ui_web::WebError>>,
}

async fn start() -> Result<Running, Box<dyn std::error::Error>> {
    let config = polaris_config::load(
        &[polaris_config::LayerInput::parse(
            polaris_config::Layer::Global,
            "[provider]\norder = []\n",
            None,
        )?],
        &polaris_config::TrustGrant::none(),
    )?
    .config;
    let dir = tempfile::tempdir()?;
    let workspace = dir.path().join("work");
    std::fs::create_dir_all(&workspace)?;
    let site = dir.path().join("site");
    std::fs::create_dir_all(site.join("pkg"))?;
    std::fs::write(site.join("pkg").join("polaris-web.js"), "export default 1;")?;
    let pages = polaris_web_app::server::pages(polaris_web_app::server::site_options(
        site.to_str().ok_or("path")?,
    ));
    let server = polaris_ui_web::WebServer::bind_with_pages(
        polaris_ui_web::WebOptions {
            bind: "127.0.0.1:0".parse()?,
            state_dir: workspace.join(".polaris").join("state"),
            workspace,
            config,
            env_file: None,
        },
        Some(pages),
    )
    .await?;
    let url = server.url().to_owned();
    let (base, token) = url.split_once("/#token=").ok_or("token")?;
    let (shutdown, signal) = tokio::sync::oneshot::channel::<()>();
    let task = tokio::spawn(server.serve(async move {
        let _ = signal.await;
    }));
    Ok(Running {
        base: base.to_owned(),
        token: token.to_owned(),
        _dir: dir,
        shutdown: Some(shutdown),
        task,
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_renders_the_page_with_a_nonce_policy_and_pwa_files() -> TestResult {
    let running = start().await?;
    let client = reqwest::Client::new();
    let page = client.get(format!("{}/", running.base)).send().await?;
    assert_eq!(page.status(), 200);
    let csp = page
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .ok_or("no CSP header")?
        .to_owned();
    assert_eq!(
        page.headers()
            .get("x-frame-options")
            .and_then(|v| v.to_str().ok()),
        Some("DENY")
    );
    let html = page.text().await?;
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .ok_or("no nonce in CSP")?;
    assert!(
        csp.contains("'wasm-unsafe-eval'") && csp.contains("frame-ancestors 'none'"),
        "{csp}"
    );
    assert!(
        html.contains(&format!("nonce=\"{nonce}\"")),
        "hydration script carries the nonce"
    );
    assert!(html.contains("/pkg/polaris-web"), "loads the bundle");
    assert!(html.contains("<title>Polaris</title>"));
    assert!(html.contains("rel=\"manifest\""));
    assert!(html.contains("What’s on your mind for Polaris?"));
    assert!(html.contains("composer-text"));

    let second = client.get(format!("{}/", running.base)).send().await?;
    let second_csp = second
        .headers()
        .get("content-security-policy")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert_ne!(second_csp, csp, "every response gets its own nonce");

    let worker = client.get(format!("{}/sw.js", running.base)).send().await?;
    assert_eq!(worker.status(), 200);
    assert!(worker.text().await?.contains("addEventListener(\"fetch\""));
    let manifest: serde_json::Value = client
        .get(format!("{}/manifest.webmanifest", running.base))
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(manifest["start_url"], "/");
    assert_eq!(
        client
            .get(format!("{}/pkg/polaris-web.js", running.base))
            .send()
            .await?
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{}/app.css", running.base))
            .send()
            .await?
            .status(),
        200,
        "the shared stylesheet stays"
    );
    assert_eq!(
        client
            .get(format!("{}/app.js", running.base))
            .send()
            .await?
            .status(),
        404,
        "the embedded JavaScript page is replaced"
    );
    assert_eq!(
        client
            .get(format!("{}/api/sessions", running.base))
            .send()
            .await?
            .status(),
        401,
        "the API still needs the token"
    );
    assert_eq!(
        client
            .get(format!("{}/api/sessions", running.base))
            .bearer_auth(&running.token)
            .send()
            .await?
            .status(),
        200
    );
    let Running {
        mut shutdown, task, ..
    } = running;
    if let Some(stop) = shutdown.take() {
        let _ = stop.send(());
    }
    tokio::time::timeout(std::time::Duration::from_secs(20), task).await???;
    Ok(())
}
