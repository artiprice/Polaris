//! GUI Session View (descriptor: specs/descriptors/ui_gui_session_view.json).

use std::error::Error;
use std::path::PathBuf;

use polaris_sdk::ids::ModelId;
use polaris_ui_gui::backend::{
    BackendEvent, BackendRequest, MessageKind, SearchHit, SessionSummary,
};
use polaris_ui_gui::session_view::{OFFLINE_NOTE, Screen, SessionController, relative_time};
use polaris_ui_model::view::EntryKind;

type TestResult = Result<(), Box<dyn Error>>;

fn controller() -> Result<SessionController, Box<dyn Error>> {
    Ok(SessionController::new(ModelId::new("vendor/model-high")?))
}

fn opened(id: &str) -> BackendEvent {
    BackendEvent::Opened {
        id: id.to_owned(),
        title: String::new(),
        workspace: PathBuf::from("C:/work/polaris"),
        branch: Some(String::from("main")),
    }
}

fn summary(id: &str, workspace: &str, updated_ms: i64) -> SessionSummary {
    SessionSummary {
        id: id.to_owned(),
        title: format!("title {id}"),
        workspace: PathBuf::from(workspace),
        branch: None,
        updated_ms,
    }
}

#[test]
fn submit_without_session_opens_then_submits() -> TestResult {
    let mut c = controller()?;
    c.edit("plan tomorrow");
    let requests = c.submit();
    assert_eq!(
        requests,
        vec![
            BackendRequest::Open { resume: None },
            BackendRequest::Submit {
                text: String::from("plan tomorrow")
            }
        ],
        "first send opens a session then submits"
    );
    assert_eq!(
        c.screen(),
        Screen::Session,
        "switches to the session screen"
    );
    assert!(c.pending(), "submit is pending");
    assert!(c.starting(), "kernel is starting");
    Ok(())
}

#[test]
fn submit_blank_draft_is_ignored() -> TestResult {
    let mut c = controller()?;
    assert!(c.submit().is_empty(), "empty draft sends nothing");
    c.edit("   \n  ");
    assert!(c.submit().is_empty(), "blank draft sends nothing");
    assert_eq!(c.screen(), Screen::Welcome, "stays on the welcome screen");
    assert!(!c.pending(), "nothing pending");
    Ok(())
}

#[test]
fn second_submit_while_pending_is_refused() -> TestResult {
    let mut c = controller()?;
    c.edit("first");
    let _first = c.submit();
    c.edit("second");
    assert!(c.submit().is_empty(), "no request while pending");
    assert!(
        c.notice().contains("Waiting"),
        "notice explains: {}",
        c.notice()
    );
    assert_eq!(c.draft(), "second", "draft kept");
    Ok(())
}

#[test]
fn committed_clears_draft_and_pending() -> TestResult {
    let mut c = controller()?;
    c.edit("plan tomorrow");
    let _requests = c.submit();
    let _list = c.apply(opened("s1"));
    let follow_up = c.apply(BackendEvent::Committed);
    assert_eq!(c.draft(), "", "draft cleared after commit");
    assert!(!c.pending(), "no longer pending");
    assert_eq!(
        follow_up,
        vec![BackendRequest::List],
        "sidebar refresh requested"
    );
    Ok(())
}

#[test]
fn rejected_keeps_draft_and_shows_reason() -> TestResult {
    let mut c = controller()?;
    c.edit("$ratelimit(rpm=oops)");
    let _requests = c.submit();
    let _none = c.apply(BackendEvent::Rejected(String::from(
        "invalid directive grammar",
    )));
    assert_eq!(c.draft(), "$ratelimit(rpm=oops)", "draft kept");
    assert!(!c.pending(), "no longer pending");
    assert!(
        c.notice().contains("invalid directive grammar"),
        "reason shown: {}",
        c.notice()
    );
    Ok(())
}

#[test]
fn message_events_append_user_entries() -> TestResult {
    let mut c = controller()?;
    let _list = c.apply(opened("s1"));
    let _a = c.apply(BackendEvent::Message {
        kind: MessageKind::UserInput,
        text: String::from("plan tomorrow"),
    });
    let _b = c.apply(BackendEvent::Message {
        kind: MessageKind::Directive,
        text: String::from("$websearch(on)"),
    });
    let users: Vec<&str> = c
        .entries()
        .iter()
        .filter(|entry| entry.kind == EntryKind::User)
        .map(|entry| entry.text.as_str())
        .collect();
    assert_eq!(
        users,
        vec!["plan tomorrow", "$websearch(on)"],
        "user entries in order"
    );
    Ok(())
}

#[test]
fn opened_resets_view_and_adds_offline_note() -> TestResult {
    let mut c = controller()?;
    let _list = c.apply(opened("s1"));
    let _m = c.apply(BackendEvent::Message {
        kind: MessageKind::UserInput,
        text: String::from("old"),
    });
    let _list2 = c.apply(opened("s2"));
    let entries = c.entries();
    assert_eq!(entries.len(), 1, "previous conversation cleared");
    let first = entries.first().ok_or("no entries")?;
    assert_eq!(
        first.kind,
        EntryKind::System,
        "offline note is a system entry"
    );
    assert_eq!(first.text, OFFLINE_NOTE, "exact offline note");
    let open = c.open_session().ok_or("no open session")?;
    assert_eq!(open.id, "s2", "open session id");
    assert_eq!(open.branch.as_deref(), Some("main"), "branch kept");
    assert_eq!(c.title(), "Untitled session", "empty title falls back");
    Ok(())
}

#[test]
fn sessions_group_by_workspace_newest_first() -> TestResult {
    let mut c = controller()?;
    let _list = c.apply(opened("b2"));
    let _none = c.apply(BackendEvent::Sessions(vec![
        summary("a1", "C:/work/alpha", 1_000),
        summary("b1", "C:/work/beta", 2_000),
        summary("b2", "C:/work/beta", 5_000),
        summary("a2", "C:/work/alpha", 3_000),
    ]));
    let groups = c.groups(10_000);
    let names: Vec<&str> = groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["beta", "alpha"],
        "group order by newest session"
    );
    let beta: Vec<&str> = groups
        .first()
        .ok_or("no group")?
        .rows
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(beta, vec!["b2", "b1"], "rows newest first");
    let active: Vec<&str> = groups
        .iter()
        .flat_map(|g| g.rows.iter())
        .filter(|r| r.active)
        .map(|r| r.id.as_str())
        .collect();
    assert_eq!(active, vec!["b2"], "only the open session is active");
    Ok(())
}

#[test]
fn relative_time_labels() -> TestResult {
    let now = 100 * 86_400_000;
    assert_eq!(relative_time(now, now), "now", "same instant");
    assert_eq!(relative_time(now, now - 30_000), "now", "under a minute");
    assert_eq!(relative_time(now, now - 5 * 60_000), "5m", "minutes");
    assert_eq!(relative_time(now, now - 3 * 3_600_000), "3h", "hours");
    assert_eq!(relative_time(now, now - 2 * 86_400_000), "2d", "days");
    assert_eq!(relative_time(now, now + 60_000), "now", "future");
    Ok(())
}

#[test]
fn search_query_requests_search_and_empty_clears_hits() -> TestResult {
    let mut c = controller()?;
    assert_eq!(
        c.search("  aurora "),
        vec![BackendRequest::Search {
            query: String::from("aurora")
        }],
        "trimmed query is searched"
    );
    let _none = c.apply(BackendEvent::Hits(vec![SearchHit {
        session_id: String::from("s1"),
        title: String::new(),
        snippet: String::from("aurora"),
    }]));
    assert_eq!(c.hits().len(), 1, "hits stored");
    assert!(c.search("   ").is_empty(), "blank query sends nothing");
    assert!(c.hits().is_empty(), "blank query clears hits");
    Ok(())
}

#[test]
fn new_session_closes_current_and_returns_to_welcome() -> TestResult {
    let mut c = controller()?;
    assert!(c.new_session().is_empty(), "nothing to close");
    let _list = c.apply(opened("s1"));
    assert_eq!(
        c.new_session(),
        vec![BackendRequest::Close],
        "closes the open session"
    );
    assert_eq!(c.screen(), Screen::Welcome, "back to welcome");
    assert!(c.entries().is_empty(), "conversation cleared");
    assert!(c.open_session().is_none(), "no open session");
    Ok(())
}

#[test]
fn open_session_requests_resume() -> TestResult {
    let mut c = controller()?;
    assert_eq!(
        c.open_listed("s9"),
        vec![BackendRequest::Open {
            resume: Some(String::from("s9"))
        }],
        "resume requested"
    );
    assert_eq!(c.screen(), Screen::Session, "session screen");
    let _list = c.apply(opened("s9"));
    assert!(
        c.open_listed("s9").is_empty(),
        "already open: nothing to do"
    );
    Ok(())
}

#[test]
fn failed_event_surfaces_notice_and_clears_pending() -> TestResult {
    let mut c = controller()?;
    c.edit("hello");
    let _requests = c.submit();
    let _none = c.apply(BackendEvent::Failed(String::from(
        "Session store is in use by another Polaris process",
    )));
    assert!(!c.pending(), "pending cancelled");
    assert!(!c.starting(), "starting ended");
    assert_eq!(c.draft(), "hello", "draft kept");
    assert!(
        c.notice().contains("in use"),
        "reason shown: {}",
        c.notice()
    );
    Ok(())
}
