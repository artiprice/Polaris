//! Desktop Bridge (descriptor: specs/descriptors/ui_desktop_bridge.json).
//!
//! Drives the real bridge thread (tokio runtime + offline kernel session) through its channels,
//! with a fresh session store in a temporary directory.

use std::error::Error;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use polaris_desktop::bridge;
use polaris_ui_gui::backend::{BackendEvent, BackendLink, BackendRequest, MessageKind};

type TestResult = Result<(), Box<dyn Error>>;

const WAIT: Duration = Duration::from_secs(20);

struct Fixture {
    _dir: tempfile::TempDir,
    link: BackendLink,
    bridge: bridge::Bridge,
    workspace: std::path::PathBuf,
}

fn fixture() -> Result<Fixture, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let workspace = dir.path().join("work");
    std::fs::create_dir_all(&workspace)?;
    let state = bridge::default_state_dir(&workspace);
    let (link, bridge) = bridge::spawn(state, workspace.clone())?;
    Ok(Fixture {
        _dir: dir,
        link,
        bridge,
        workspace,
    })
}

/// Collects events until `done` matches one (inclusive) or the deadline passes.
fn until(
    events: &Receiver<BackendEvent>,
    done: impl Fn(&BackendEvent) -> bool,
) -> Result<Vec<BackendEvent>, Box<dyn Error>> {
    let deadline = Instant::now() + WAIT;
    let mut seen = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match events.recv_timeout(left) {
            Ok(event) => {
                let stop = done(&event);
                seen.push(event);
                if stop {
                    return Ok(seen);
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                return Err(format!("timed out; events so far: {seen:?}").into());
            }
            Err(RecvTimeoutError::Disconnected) => {
                return Err(format!("bridge stopped; events so far: {seen:?}").into());
            }
        }
    }
}

fn open_new(f: &Fixture) -> Result<String, Box<dyn Error>> {
    f.link
        .requests
        .send(BackendRequest::Open { resume: None })?;
    let seen = until(&f.link.events, |e| {
        matches!(e, BackendEvent::Opened { .. } | BackendEvent::Failed(_))
    })?;
    assert!(
        matches!(seen.first(), Some(BackendEvent::Starting)),
        "Starting first: {seen:?}"
    );
    match seen.last() {
        Some(BackendEvent::Opened { id, .. }) => Ok(id.clone()),
        other => Err(format!("open failed: {other:?}").into()),
    }
}

fn message_text(kind: MessageKind, text: &str) -> impl Fn(&BackendEvent) -> bool + '_ {
    move |event| matches!(event, BackendEvent::Message { kind: k, text: t } if *k == kind && t == text)
}

#[test]
fn list_without_kernel_returns_sessions() -> TestResult {
    let f = fixture()?;
    f.link.requests.send(BackendRequest::List)?;
    let seen = until(&f.link.events, |e| {
        matches!(e, BackendEvent::Sessions(_) | BackendEvent::Failed(_))
    })?;
    assert_eq!(
        seen,
        vec![BackendEvent::Sessions(Vec::new())],
        "empty store lists nothing"
    );
    drop(f.link);
    f.bridge.join()?;
    Ok(())
}

#[test]
fn open_submit_commits_and_streams_message() -> TestResult {
    let f = fixture()?;
    let _id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("plan tomorrow"),
    })?;
    let seen = until(
        &f.link.events,
        message_text(MessageKind::UserInput, "plan tomorrow"),
    )?;
    assert!(
        seen.contains(&BackendEvent::Committed),
        "committed: {seen:?}"
    );
    drop(f.link);
    f.bridge.join()?;
    Ok(())
}

#[test]
fn directive_submit_streams_directive_message() -> TestResult {
    let f = fixture()?;
    let _id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("$websearch(on)"),
    })?;
    let seen = until(&f.link.events, |e| {
        matches!(
            e,
            BackendEvent::Message {
                kind: MessageKind::Directive,
                ..
            }
        )
    })?;
    assert!(
        seen.contains(&BackendEvent::Committed),
        "committed: {seen:?}"
    );
    let text = match seen.last() {
        Some(BackendEvent::Message { text, .. }) => text.clone(),
        other => return Err(format!("no directive message: {other:?}").into()),
    };
    assert!(
        text.starts_with("$websearch"),
        "canonical directive text: {text}"
    );
    Ok(())
}

#[test]
fn blank_submit_is_rejected() -> TestResult {
    let f = fixture()?;
    let _id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("   "),
    })?;
    let seen = until(&f.link.events, |e| {
        matches!(e, BackendEvent::Rejected(_) | BackendEvent::Committed)
    })?;
    assert!(
        matches!(seen.last(), Some(BackendEvent::Rejected(_))),
        "blank input rejected: {seen:?}"
    );
    Ok(())
}

#[test]
fn submit_without_session_is_rejected() -> TestResult {
    let f = fixture()?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("hello"),
    })?;
    let seen = until(&f.link.events, |e| matches!(e, BackendEvent::Rejected(_)))?;
    assert_eq!(
        seen.last(),
        Some(&BackendEvent::Rejected(String::from("no session is open"))),
        "rejected without a session"
    );
    Ok(())
}

#[test]
fn resume_replays_messages() -> TestResult {
    let f = fixture()?;
    let id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("remember this"),
    })?;
    let _seen = until(
        &f.link.events,
        message_text(MessageKind::UserInput, "remember this"),
    )?;
    f.link.requests.send(BackendRequest::Close)?;
    let _closed = until(&f.link.events, |e| matches!(e, BackendEvent::Closed))?;
    f.link.requests.send(BackendRequest::Open {
        resume: Some(id.clone()),
    })?;
    let seen = until(
        &f.link.events,
        message_text(MessageKind::UserInput, "remember this"),
    )?;
    assert!(
        seen.iter()
            .any(|e| matches!(e, BackendEvent::Opened { id: opened, .. } if *opened == id)),
        "reopened the same session: {seen:?}"
    );
    Ok(())
}

#[test]
fn listed_session_has_workspace_and_recent_time() -> TestResult {
    let f = fixture()?;
    let id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("listed"),
    })?;
    let _seen = until(
        &f.link.events,
        message_text(MessageKind::UserInput, "listed"),
    )?;
    f.link.requests.send(BackendRequest::List)?;
    let seen = until(&f.link.events, |e| matches!(e, BackendEvent::Sessions(_)))?;
    let Some(BackendEvent::Sessions(rows)) = seen.last() else {
        return Err("no session list".into());
    };
    let row = rows
        .iter()
        .find(|row| row.id == id)
        .ok_or("session not listed")?;
    assert_eq!(
        dunce_like(&row.workspace),
        dunce_like(&f.workspace),
        "workspace recorded"
    );
    let now = i64::try_from(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis())?;
    assert!(
        now.saturating_sub(row.updated_ms) < 60_000,
        "updated within a minute: now {now}, updated {}",
        row.updated_ms
    );
    Ok(())
}

/// Compares paths after canonicalising, tolerant of `\\?\` prefixes.
fn dunce_like(path: &std::path::Path) -> String {
    let text = match std::fs::canonicalize(path) {
        Ok(path) => path.display().to_string(),
        Err(_) => path.display().to_string(),
    };
    match text.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_lowercase(),
        None => text.to_lowercase(),
    }
}

#[test]
fn dropping_the_link_shuts_down_cleanly() -> TestResult {
    let f = fixture()?;
    let _id = open_new(&f)?;
    let Fixture {
        _dir, link, bridge, ..
    } = f;
    drop(link.requests);
    let seen = until(&link.events, |e| matches!(e, BackendEvent::Closed))?;
    assert!(
        matches!(seen.last(), Some(BackendEvent::Closed)),
        "session closed on shutdown: {seen:?}"
    );
    bridge.join()?;
    Ok(())
}

/// P5: `/nv-verify` typed in the window is answered with the CUDA advisor's report for the
/// session's workspace instead of being recorded as a message.
#[test]
fn nv_verify_is_answered_with_a_report() -> TestResult {
    let f = fixture()?;
    std::fs::write(
        f.workspace.join("kernel.cu"),
        "__global__ void zero(float* x) {\n    int i = blockIdx.x * blockDim.x + threadIdx.x;\n    x[i] = 0.0f;\n}\n",
    )?;
    let _id = open_new(&f)?;
    f.link.requests.send(BackendRequest::Submit {
        text: String::from("/nv-verify"),
    })?;
    let seen = until(&f.link.events, |e| {
        matches!(e, BackendEvent::Report { .. } | BackendEvent::Rejected(_))
    })?;
    assert!(seen.contains(&BackendEvent::Committed), "{seen:?}");
    match seen.last() {
        Some(BackendEvent::Report { title, lines }) => {
            assert_eq!(title, "CUDA advisor");
            assert_eq!(
                lines.first().map(String::as_str),
                Some("Checked 1 CUDA source: 1 warning, 0 suggestions.")
            );
            assert!(
                lines
                    .iter()
                    .any(|line| line.contains("kernel.cu:2 warning [missing-grid-bounds]"))
            );
        }
        other => return Err(format!("no report: {other:?}").into()),
    }
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, BackendEvent::Message { .. })),
        "the command is not recorded as a message: {seen:?}"
    );
    drop(f.link);
    f.bridge.join()?;
    Ok(())
}
