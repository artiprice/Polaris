//! Desktop bridge attached to a real `polarisd` (P9): `Open` attaches over the local endpoint,
//! typed lines are recorded by the daemon and come back from its event stream; a credential the
//! daemon does not trust is reported instead of opening.

#[path = "../../support/attach_daemon.rs"]
mod attach_daemon;

use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use attach_daemon::{AnyError, Daemon};
use polaris_desktop::bridge;
use polaris_ui_gui::backend::{BackendEvent, BackendRequest, MessageKind};

const WAIT: Duration = Duration::from_secs(20);

/// Collects events until `done` matches one (inclusive) or the deadline passes.
fn until(
    events: &Receiver<BackendEvent>,
    done: impl Fn(&BackendEvent) -> bool,
) -> Result<Vec<BackendEvent>, AnyError> {
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

fn started() -> Result<(tokio::runtime::Runtime, Daemon), AnyError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let daemon = runtime.block_on(async {
        let daemon = Daemon::start()?;
        daemon.ready().await?;
        Ok::<_, AnyError>(daemon)
    })?;
    Ok((runtime, daemon))
}

#[test]
fn the_window_records_into_an_attached_daemon_session() -> Result<(), AnyError> {
    let (runtime, daemon) = started()?;
    let dir = tempfile::tempdir()?;
    let (link, bridge) = bridge::spawn_attached(
        dir.path().join("state"),
        dir.path().to_path_buf(),
        daemon.target()?,
    )?;
    let send = |request| link.requests.send(request).map_err(|_| "bridge stopped");

    let notice = until(&link.events, |event| {
        matches!(event, BackendEvent::ProviderUnavailable(_))
    })?;
    assert!(
        matches!(notice.last(), Some(BackendEvent::ProviderUnavailable(reason)) if reason.contains("polarisd")),
        "{notice:?}"
    );

    send(BackendRequest::Open { resume: None })?;
    let opened = until(&link.events, |event| {
        matches!(event, BackendEvent::Opened { .. })
    })?;
    let session = daemon.session.to_string();
    assert!(
        matches!(opened.last(), Some(BackendEvent::Opened { id, .. }) if *id == session),
        "{opened:?}"
    );

    send(BackendRequest::Submit {
        text: String::from("hello daemon"),
    })?;
    until(
        &link.events,
        |event| matches!(event, BackendEvent::Message { kind: MessageKind::UserInput, text } if text == "hello daemon"),
    )?;
    send(BackendRequest::Submit {
        text: String::from("$websearch(off)"),
    })?;
    until(
        &link.events,
        |event| matches!(event, BackendEvent::Message { kind: MessageKind::Directive, text } if text == "$websearch(off)"),
    )?;

    send(BackendRequest::List)?;
    let listed = until(&link.events, |event| {
        matches!(event, BackendEvent::Sessions(_))
    })?;
    assert!(
        matches!(listed.last(), Some(BackendEvent::Sessions(rows)) if rows.len() == 1 && rows.iter().all(|row| row.id == session)),
        "{listed:?}"
    );
    send(BackendRequest::Search {
        query: String::from("hello"),
    })?;
    let search = until(&link.events, |event| {
        matches!(event, BackendEvent::Failed(_))
    })?;
    assert!(
        matches!(search.last(), Some(BackendEvent::Failed(reason)) if reason.contains("local session store")),
        "{search:?}"
    );

    send(BackendRequest::Close)?;
    until(&link.events, |event| matches!(event, BackendEvent::Closed))?;
    drop(link);
    bridge.join()?;
    runtime.block_on(daemon.stop())
}

#[test]
fn an_untrusted_credential_is_reported_instead_of_opening() -> Result<(), AnyError> {
    let (runtime, daemon) = started()?;
    let dir = tempfile::tempdir()?;
    let (link, bridge) = bridge::spawn_attached(
        dir.path().join("state"),
        dir.path().to_path_buf(),
        daemon.foreign_target()?,
    )?;
    link.requests
        .send(BackendRequest::Open { resume: None })
        .map_err(|_| "bridge stopped")?;
    let events = until(&link.events, |event| {
        matches!(event, BackendEvent::Failed(_))
    })?;
    assert!(
        matches!(events.last(), Some(BackendEvent::Failed(reason)) if reason.contains("Could not attach to polarisd")),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, BackendEvent::Opened { .. })),
        "{events:?}"
    );
    drop(link);
    bridge.join()?;
    runtime.block_on(daemon.stop())
}
