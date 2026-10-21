//! View Reducer (descriptor: specs/descriptors/uimodel_view_reducer.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp, WorkerKind};
use polaris_sdk::effort::Effort;
use polaris_sdk::error::{ErrorKind, HasKind};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, ModelId, Seq, SessionId, TokenCount};
use polaris_ui_model::error::ReduceError;
use polaris_ui_model::reduce::reduce;
use polaris_ui_model::view::{
    EntryKind, GoalState, Mode, ProofOutcome, ToolStatus, ViewState, format_duration_ms,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

fn cid(b: u8) -> Cid {
    Cid::from_bytes([b; 32])
}

fn frame() -> FrameId {
    FrameId::from_u128(7)
}

fn session() -> SessionId {
    SessionId::from_u128(1)
}

fn rec(seq: u64, event: Event) -> EventRecord {
    EventRecord::new(Seq::new(seq), Timestamp::UNIX_EPOCH, session(), event)
}

fn model(name: &str) -> Result<ModelId, Box<dyn Error>> {
    Ok(ModelId::new(name)?)
}

fn fresh() -> Result<ViewState, Box<dyn Error>> {
    Ok(ViewState::new(
        model("nvidia/nemotron-3-super-120b-a12b")?,
        Effort::High,
    ))
}

fn fold(state: &mut ViewState, first_seq: u64, events: &[Event]) -> Result<(), ReduceError> {
    for (offset, event) in (0u64..).zip(events) {
        reduce(state, &rec(first_seq + offset, *event))?;
    }
    Ok(())
}

fn model_request(prompt: u32, completion: u32, cached: u32, cost: u64, latency_us: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: frame(),
        request: cid(9),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(completion),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(cost),
        latency_us,
    }
}

#[test]
fn message_events_append_conversation() -> TestResult {
    let mut s = fresh()?;
    s.push_user("hi");
    reduce(&mut s, &rec(0, Event::SessionStarted { workspace: cid(1) }))?;
    s.push_assistant("hello");
    s.push_system("model switched");
    s.push_error("boom");
    let kinds: Vec<_> = s.conversation.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            EntryKind::User,
            EntryKind::Assistant,
            EntryKind::System,
            EntryKind::Error
        ]
    );
    let texts: Vec<_> = s.conversation.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(texts, ["hi", "hello", "model switched", "boom"]);
    Ok(())
}

#[test]
fn seq_gap_returns_error() -> TestResult {
    let mut s = fresh()?;
    reduce(&mut s, &rec(0, Event::SessionStarted { workspace: cid(1) }))?;
    reduce(&mut s, &rec(1, model_request(10, 5, 0, 100, 1000)))?;
    let before = s.clone();
    let err = reduce(&mut s, &rec(3, model_request(10, 5, 0, 100, 1000)))
        .err()
        .ok_or("a gap must be an error")?;
    assert_eq!(
        err,
        ReduceError::SeqGap {
            expected: Seq::new(2),
            got: Seq::new(3)
        }
    );
    assert_eq!(err.kind(), ErrorKind::Permanent);
    assert_eq!(s, before, "state unchanged on error");
    // The expected record still applies afterwards.
    reduce(&mut s, &rec(2, model_request(1, 1, 0, 1, 1)))?;
    assert_eq!(s.last_seq, Some(Seq::new(2)));
    Ok(())
}

#[test]
fn out_of_order_returns_error() -> TestResult {
    let mut s = fresh()?;
    reduce(&mut s, &rec(5, Event::SessionStarted { workspace: cid(1) }))?;
    let before = s.clone();
    for stale in [5u64, 4, 0] {
        let err = reduce(&mut s, &rec(stale, Event::SessionClosed))
            .err()
            .ok_or("stale seq must be an error")?;
        assert_eq!(
            err,
            ReduceError::OutOfOrder {
                last: Seq::new(5),
                got: Seq::new(stale)
            }
        );
    }
    assert_eq!(s, before);
    Ok(())
}

#[test]
fn first_record_may_start_anywhere() -> TestResult {
    let mut s = fresh()?;
    assert_eq!(s.last_seq, None);
    reduce(&mut s, &rec(41, Event::SessionClosed))?;
    assert_eq!(s.last_seq, Some(Seq::new(41)));
    let mut zero = fresh()?;
    reduce(&mut zero, &rec(0, Event::SessionClosed))?;
    assert_eq!(zero.last_seq, Some(Seq::ZERO));
    Ok(())
}

#[test]
fn nothing_follows_the_maximum_sequence() -> TestResult {
    let mut s = fresh()?;
    reduce(&mut s, &rec(u64::MAX, Event::SessionClosed))?;
    let err = reduce(&mut s, &rec(0, Event::SessionClosed))
        .err()
        .ok_or("nothing follows u64::MAX")?;
    assert_eq!(
        err,
        ReduceError::OutOfOrder {
            last: Seq::new(u64::MAX),
            got: Seq::ZERO
        }
    );
    Ok(())
}

#[test]
fn tool_title_collapsed_until_toggle() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(1),
            },
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(2),
            },
        ],
    )?;
    assert_eq!(s.tools.len(), 2);
    assert!(
        !s.tool_expanded(0) && !s.tool_expanded(1),
        "collapsed by default"
    );
    assert_eq!(s.tools.first().map(|t| t.status), Some(ToolStatus::Running));
    // Title falls back to the short ref until the frontend resolves the call record from CAS.
    let fallback = s.tools.first().map(|t| t.title_text()).ok_or("tool 0")?;
    assert!(fallback.starts_with("tool call §ref:"), "{fallback}");
    s.set_tool_title(cid(1), "Edit src/lib.rs");
    assert_eq!(
        s.tools.first().map(|t| t.title_text()).as_deref(),
        Some("Edit src/lib.rs")
    );

    assert!(s.toggle_tool(0));
    assert!(s.tool_expanded(0));
    assert!(!s.tool_expanded(1), "only the clicked entry expands");
    assert!(!s.toggle_tool(9), "unknown index is ignored");
    assert!(!s.tool_expanded(9));

    s.toggle_all_tools();
    assert!(s.tools_expanded);
    assert!(s.tool_expanded(0) && s.tool_expanded(1));
    // A per-entry override beats the global flag.
    assert!(s.toggle_tool(0));
    assert!(!s.tool_expanded(0) && s.tool_expanded(1));
    Ok(())
}

#[test]
fn global_toggle_clears_overrides() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(1),
            },
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(2),
            },
        ],
    )?;
    s.toggle_tool(0);
    s.toggle_all_tools();
    s.toggle_tool(1);
    s.toggle_all_tools();
    assert!(!s.tools_expanded);
    assert!(!s.tool_expanded(0) && !s.tool_expanded(1));
    assert!(s.tools.iter().all(|t| t.expanded.is_none()));
    Ok(())
}

#[test]
fn tool_completion_marks_status_and_latency() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(1),
            },
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(2),
            },
            Event::ToolExecutionCompleted {
                frame: frame(),
                call: cid(2),
                ok: false,
                latency_us: 700,
            },
            Event::ToolExecutionCompleted {
                frame: frame(),
                call: cid(1),
                ok: true,
                latency_us: 250,
            },
        ],
    )?;
    let first = s.tools.first().ok_or("tool 0")?;
    assert_eq!(
        (first.status, first.latency_us),
        (ToolStatus::Succeeded, Some(250))
    );
    let second = s.tools.get(1).ok_or("tool 1")?;
    assert_eq!(
        (second.status, second.latency_us),
        (ToolStatus::Failed, Some(700))
    );
    assert_eq!(s.tools.len(), 2);
    Ok(())
}

#[test]
fn tool_completion_without_start_creates_entry() -> TestResult {
    let mut s = fresh()?;
    reduce(
        &mut s,
        &rec(
            10,
            Event::ToolExecutionCompleted {
                frame: frame(),
                call: cid(5),
                ok: true,
                latency_us: 12,
            },
        ),
    )?;
    assert_eq!(s.tools.len(), 1);
    let tool = s.tools.first().ok_or("tool 0")?;
    assert_eq!(
        (tool.status, tool.latency_us, tool.call),
        (ToolStatus::Succeeded, Some(12), cid(5))
    );
    Ok(())
}

#[test]
fn model_request_updates_token_weather() -> TestResult {
    let mut s = fresh()?;
    assert_eq!(s.hud.cache_hit_permille(), None);
    fold(
        &mut s,
        0,
        &[
            model_request(1000, 200, 800, 5000, 1_500_000),
            model_request(500, 100, 0, 2500, 700_000),
        ],
    )?;
    assert_eq!(s.hud.requests, 2);
    assert_eq!(s.hud.prompt_tokens, 1500);
    assert_eq!(s.hud.completion_tokens, 300);
    assert_eq!(s.hud.cached_tokens, 800);
    assert_eq!(s.hud.cost_micro_usd, 7500);
    assert_eq!(s.hud.last_s2_latency_us, Some(700_000));
    assert_eq!(s.hud.last_prompt_tokens, 500);
    assert_eq!(s.hud.cache_hit_permille(), Some(533));
    Ok(())
}

#[test]
fn token_weather_saturates_instead_of_overflowing() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[
            model_request(u32::MAX, u32::MAX, u32::MAX, u64::MAX, 1),
            model_request(u32::MAX, u32::MAX, u32::MAX, u64::MAX, 1),
        ],
    )?;
    assert_eq!(s.hud.cost_micro_usd, u64::MAX);
    assert_eq!(s.hud.cache_hit_permille(), Some(1000));
    Ok(())
}

#[test]
fn decision_updates_s1_latency() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[Event::DecisionMade {
            frame: frame(),
            decision: cid(3),
            latency_us: 4200,
        }],
    )?;
    assert_eq!(s.hud.last_s1_latency_us, Some(4200));
    assert_eq!(s.hud.decisions, 1);
    assert_eq!(s.hud.last_s2_latency_us, None);
    Ok(())
}

#[test]
fn status_from_worker_circuit_dead_letter() -> TestResult {
    let mut s = fresh()?;
    fold(
        &mut s,
        0,
        &[
            Event::WorkerRestarted {
                worker: WorkerKind::S1,
                restarts: 2,
            },
            Event::WorkerRestarted {
                worker: WorkerKind::Ingest,
                restarts: 1,
            },
            Event::WorkerRestarted {
                worker: WorkerKind::Nano,
                restarts: 4,
            },
            Event::WorkerRestarted {
                worker: WorkerKind::S1,
                restarts: 3,
            },
            Event::CircuitTripped { breaker: cid(1) },
            Event::CircuitTripped { breaker: cid(1) },
            Event::DeadLettered { message: cid(2) },
            Event::DiffApproved {
                frame: frame(),
                diff: cid(3),
            },
            Event::DiffRejected {
                frame: frame(),
                diff: cid(3),
                reason: cid(4),
            },
            Event::ProofOfDoneBlocked {
                frame: frame(),
                report: cid(5),
            },
            Event::ProofOfDonePassed {
                frame: frame(),
                report: cid(5),
            },
            Event::DirectiveApplied { directive: cid(6) },
        ],
    )?;
    let st = &s.status;
    assert_eq!(
        (
            st.worker_restarts.s1,
            st.worker_restarts.ingest,
            st.worker_restarts.nano
        ),
        (3, 1, 4)
    );
    assert_eq!(st.circuit_trips, 2);
    assert_eq!(st.dead_letters, 1);
    assert_eq!((st.diffs_approved, st.diffs_rejected), (1, 1));
    assert_eq!(st.proof_of_done, Some(ProofOutcome::Passed));
    assert_eq!(st.directives_applied, 1);
    Ok(())
}

#[test]
fn session_started_and_closed() -> TestResult {
    let mut s = fresh()?;
    let at = Timestamp::from_second(1_800_000_000)?;
    reduce(
        &mut s,
        &EventRecord::new(
            Seq::new(0),
            at,
            session(),
            Event::SessionStarted { workspace: cid(1) },
        ),
    )?;
    let row = s.sessions.first().ok_or("session row")?;
    assert_eq!(row.session, session());
    assert_eq!(row.workspace, Some(cid(1)));
    assert_eq!(row.started_at_ms, 1_800_000_000_000);
    assert_eq!(row.started_seq, Seq::ZERO);
    assert!(!row.closed);
    assert!(s.status.session_open);

    reduce(&mut s, &rec(1, Event::SessionClosed))?;
    assert!(s.sessions.first().map(|r| r.closed).unwrap_or(false));
    assert!(!s.status.session_open);

    // A close for a session we never saw start (partial replay) still produces a closed row.
    let other = SessionId::from_u128(2);
    reduce(
        &mut s,
        &EventRecord::new(
            Seq::new(2),
            Timestamp::UNIX_EPOCH,
            other,
            Event::SessionClosed,
        ),
    )?;
    assert_eq!(s.sessions.len(), 2);
    let late = s.sessions.get(1).ok_or("second row")?;
    assert_eq!(
        (late.session, late.workspace, late.closed),
        (other, None, true)
    );
    Ok(())
}

#[test]
fn backend_lifecycle_reopen_updates_existing_session_row() -> TestResult {
    let mut state = fresh()?;
    fold(
        &mut state,
        0,
        &[
            Event::SessionConfigured {
                configuration: cid(1),
            },
            Event::SessionStarted { workspace: cid(2) },
            Event::SessionClosed,
            Event::SessionPaused,
            Event::SessionResumed,
        ],
    )?;
    assert_eq!(
        state.sessions.len(),
        1,
        "backend reopen does not duplicate the row"
    );
    assert_eq!(
        state.sessions.first().map(|row| row.closed),
        Some(false),
        "reopened session is no longer archived"
    );
    assert!(state.status.session_open, "resumed backend is open");
    Ok(())
}

#[test]
fn goal_time_excludes_pauses() -> TestResult {
    let mut s = fresh()?;
    assert!(s.goal.is_none());
    s.goal_start("ship it", 1_000);
    let goal = s.goal.as_ref().ok_or("goal")?;
    assert_eq!(goal.state, GoalState::Active);
    assert_eq!(goal.active_ms(5_000), 4_000);
    s.goal_pause(5_000);
    let paused = s.goal.as_ref().ok_or("goal")?;
    assert_eq!(paused.state, GoalState::Paused);
    assert_eq!(paused.active_ms(9_000), 4_000, "the pause adds nothing");
    assert_eq!(paused.label(9_000), "goal paused (4s)");
    s.goal_pause(9_500); // pausing twice changes nothing
    s.goal_resume(10_000);
    s.goal_resume(10_500); // resuming twice changes nothing
    let resumed = s.goal.as_ref().ok_or("goal")?;
    assert_eq!(resumed.active_ms(13_000), 7_000);
    assert_eq!(resumed.label(13_000), "goal active (7s)");
    // A clock that moves backwards never underflows.
    assert_eq!(resumed.active_ms(0), 4_000);
    s.goal_pause(14_000);
    assert_eq!(s.goal.as_ref().map(|g| g.active_ms(99_000)), Some(8_000));
    s.goal_clear();
    assert!(s.goal.is_none());
    s.goal_pause(1); // no goal: all of these are no-ops
    s.goal_resume(2);
    assert!(s.goal.is_none());
    Ok(())
}

#[test]
fn duration_format() {
    assert_eq!(format_duration_ms(0), "0s");
    assert_eq!(format_duration_ms(59_999), "59s");
    assert_eq!(format_duration_ms(394_000), "6m 34s");
    assert_eq!(format_duration_ms(3_661_000), "1h 1m 1s");
    assert!(!format_duration_ms(u64::MAX).is_empty());
}

#[test]
fn unknown_event_ignored_not_panicking() -> TestResult {
    let mut s = fresh()?;
    reduce(&mut s, &rec(0, Event::SessionStarted { workspace: cid(1) }))?;
    let mut expected = s.clone();
    fold(
        &mut s,
        1,
        &[
            Event::FileLeaseGranted {
                frame: frame(),
                path: cid(1),
                lease: 1,
            },
            Event::FileLeaseRevoked {
                frame: frame(),
                lease: 1,
            },
            Event::SandboxSnapshotBranchCreated {
                frame: frame(),
                parent: cid(1),
                branch: cid(2),
            },
            Event::SandboxExecutionCompleted {
                frame: frame(),
                exec: cid(3),
                exit_code: -1,
                latency_us: 5,
            },
        ],
    )?;
    expected.last_seq = Some(Seq::new(4));
    assert_eq!(s, expected, "only the sequence advanced");
    Ok(())
}

#[test]
fn effort_clamps_at_endpoints() -> TestResult {
    let mut s = fresh()?;
    assert!(s.effort_up());
    assert_eq!(s.effort, Effort::XHigh);
    assert!(s.effort_up());
    assert_eq!(s.effort, Effort::Max);
    assert!(!s.effort_up());
    assert_eq!(s.effort, Effort::Max);
    for expected in [Effort::XHigh, Effort::High, Effort::Medium, Effort::Low] {
        assert!(s.effort_down());
        assert_eq!(s.effort, expected);
    }
    assert!(!s.effort_down());
    assert_eq!(s.effort, Effort::Low);
    Ok(())
}

#[test]
fn per_model_effort_preference() -> TestResult {
    let a = model("model-a")?;
    let b = model("model-b")?;
    let mut s = ViewState::new(a.clone(), Effort::High);
    s.set_effort(Effort::Max);
    s.set_model(b.clone());
    assert_eq!(
        s.effort,
        Effort::Max,
        "no preference yet: the effort carries over"
    );
    s.set_effort(Effort::Low);
    s.set_model(a.clone());
    assert_eq!(s.effort, Effort::Max);
    s.set_model(b);
    assert_eq!(s.effort, Effort::Low);
    s.effort_up();
    s.set_model(a);
    s.set_model(model("model-b")?);
    assert_eq!(s.effort, Effort::Medium, "effort_up is remembered too");
    Ok(())
}

#[test]
fn mode_toggles() -> TestResult {
    let mut s = fresh()?;
    assert_eq!(s.mode, Mode::Auto);
    s.toggle_mode();
    assert_eq!(s.mode, Mode::Plan);
    s.toggle_mode();
    assert_eq!(s.mode, Mode::Auto);
    s.set_mode(Mode::Plan);
    assert_eq!(s.mode, Mode::Plan);
    assert_eq!(Mode::Auto.toggle(), Mode::Plan);
    Ok(())
}

fn script_state() -> Result<ViewState, Box<dyn Error>> {
    let mut s = fresh()?;
    s.push_user("fix the failing test");
    fold(
        &mut s,
        0,
        &[
            Event::SessionStarted { workspace: cid(1) },
            Event::ToolExecutionStarted {
                frame: frame(),
                call: cid(2),
            },
            Event::ToolExecutionCompleted {
                frame: frame(),
                call: cid(2),
                ok: true,
                latency_us: 1500,
            },
            model_request(2000, 300, 1500, 12_000, 900_000),
        ],
    )?;
    s.set_tool_title(cid(2), "Read crates/polaris-config/src/lib.rs");
    s.push_assistant("Done: the test passes.");
    Ok(s)
}

#[test]
fn snapshot_welcome() -> TestResult {
    insta::assert_json_snapshot!(fresh()?);
    Ok(())
}

#[test]
fn snapshot_conversation() -> TestResult {
    insta::assert_json_snapshot!(script_state()?);
    Ok(())
}

#[test]
fn snapshot_error_and_status() -> TestResult {
    let mut s = script_state()?;
    s.push_error("provider returned 429");
    fold(
        &mut s,
        4,
        &[
            Event::WorkerRestarted {
                worker: WorkerKind::Nano,
                restarts: 1,
            },
            Event::CircuitTripped { breaker: cid(7) },
            Event::DeadLettered { message: cid(8) },
        ],
    )?;
    insta::assert_json_snapshot!(s);
    Ok(())
}

#[test]
fn snapshot_goal_active_and_paused() -> TestResult {
    let mut active = script_state()?;
    active.goal_start("make the suite green", 1_000);
    insta::assert_json_snapshot!("goal_active", active);
    let mut paused = active.clone();
    paused.goal_pause(61_000);
    insta::assert_json_snapshot!("goal_paused", paused);
    Ok(())
}

#[test]
fn view_state_serde_round_trip() -> TestResult {
    let mut s = script_state()?;
    s.goal_start("g", 5);
    s.toggle_tool(0);
    s.set_model(model("other/model")?);
    s.set_effort(Effort::Low);
    let json = serde_json::to_string(&s)?;
    let back: ViewState = serde_json::from_str(&json)?;
    assert_eq!(back, s);
    Ok(())
}

fn event_strategy() -> impl Strategy<Value = Event> {
    let c = any::<u8>().prop_map(cid);
    let latency = 0u64..5_000_000;
    let worker = prop_oneof![
        Just(WorkerKind::S1),
        Just(WorkerKind::Ingest),
        Just(WorkerKind::Nano)
    ];
    prop_oneof![
        c.clone()
            .prop_map(|workspace| Event::SessionStarted { workspace })
            .boxed(),
        c.clone()
            .prop_map(|directive| Event::DirectiveApplied { directive })
            .boxed(),
        (c.clone(), latency.clone())
            .prop_map(|(decision, latency_us)| Event::DecisionMade {
                frame: frame(),
                decision,
                latency_us,
            })
            .boxed(),
        (
            any::<u32>(),
            any::<u32>(),
            any::<u32>(),
            any::<u64>(),
            latency.clone()
        )
            .prop_map(|(p, q, k, cost, latency_us)| model_request(p, q, k, cost, latency_us))
            .boxed(),
        c.clone()
            .prop_map(|call| Event::ToolExecutionStarted {
                frame: frame(),
                call,
            })
            .boxed(),
        (c.clone(), any::<bool>(), latency.clone())
            .prop_map(|(call, ok, latency_us)| Event::ToolExecutionCompleted {
                frame: frame(),
                call,
                ok,
                latency_us,
            })
            .boxed(),
        c.clone()
            .prop_map(|diff| Event::DiffApproved {
                frame: frame(),
                diff,
            })
            .boxed(),
        (c.clone(), c.clone())
            .prop_map(|(diff, reason)| Event::DiffRejected {
                frame: frame(),
                diff,
                reason,
            })
            .boxed(),
        (c.clone(), any::<u64>())
            .prop_map(|(path, lease)| Event::FileLeaseGranted {
                frame: frame(),
                path,
                lease,
            })
            .boxed(),
        any::<u64>()
            .prop_map(|lease| Event::FileLeaseRevoked {
                frame: frame(),
                lease,
            })
            .boxed(),
        (c.clone(), c.clone())
            .prop_map(|(parent, branch)| Event::SandboxSnapshotBranchCreated {
                frame: frame(),
                parent,
                branch,
            })
            .boxed(),
        (c.clone(), any::<i32>(), latency)
            .prop_map(
                |(exec, exit_code, latency_us)| Event::SandboxExecutionCompleted {
                    frame: frame(),
                    exec,
                    exit_code,
                    latency_us,
                }
            )
            .boxed(),
        c.clone()
            .prop_map(|report| Event::ProofOfDonePassed {
                frame: frame(),
                report,
            })
            .boxed(),
        c.clone()
            .prop_map(|report| Event::ProofOfDoneBlocked {
                frame: frame(),
                report,
            })
            .boxed(),
        (worker, any::<u32>())
            .prop_map(|(worker, restarts)| Event::WorkerRestarted { worker, restarts })
            .boxed(),
        c.clone()
            .prop_map(|breaker| Event::CircuitTripped { breaker })
            .boxed(),
        c.prop_map(|message| Event::DeadLettered { message })
            .boxed(),
        Just(Event::SessionClosed).boxed(),
    ]
}

proptest! {
    #[test]
    fn prop_reducer_is_deterministic(
        events in proptest::collection::vec(event_strategy(), 0..40),
        first_seq in 0u64..1000,
    ) {
        let mut a = fresh().map_err(|e| TestCaseError::fail(e.to_string()))?;
        let mut b = fresh().map_err(|e| TestCaseError::fail(e.to_string()))?;
        fold(&mut a, first_seq, &events).map_err(|e| TestCaseError::fail(e.to_string()))?;
        fold(&mut b, first_seq, &events).map_err(|e| TestCaseError::fail(e.to_string()))?;
        prop_assert_eq!(&a, &b);
        prop_assert_eq!(
            serde_json::to_string(&a).map_err(|e| TestCaseError::fail(e.to_string()))?,
            serde_json::to_string(&b).map_err(|e| TestCaseError::fail(e.to_string()))?
        );
        let expected_last = first_seq + events.len() as u64;
        if events.is_empty() {
            prop_assert_eq!(a.last_seq, None);
        } else {
            prop_assert_eq!(a.last_seq, Some(Seq::new(expected_last - 1)));
        }
    }

    #[test]
    fn prop_error_leaves_state_unchanged(
        events in proptest::collection::vec(event_strategy(), 1..20),
        bad_offset in 2u64..50,
        stale in any::<bool>(),
    ) {
        let mut s = fresh().map_err(|e| TestCaseError::fail(e.to_string()))?;
        fold(&mut s, 10, &events).map_err(|e| TestCaseError::fail(e.to_string()))?;
        let before = s.clone();
        let next = 10 + events.len() as u64;
        let bad = if stale { next - 1 } else { next + bad_offset };
        let result = reduce(&mut s, &rec(bad, Event::SessionClosed));
        prop_assert!(result.is_err());
        prop_assert_eq!(s, before);
    }
}
