//! Ordered event folding. Sequence validation precedes every state mutation.

use crate::error::ReduceError;
use crate::view::{ProofOutcome, SessionRow, ToolActivity, ToolStatus, ViewState};
use polaris_sdk::contracts::{Event, EventRecord, WorkerKind};
use polaris_sdk::ids::{Cid, FrameId};

fn update_tool(
    state: &mut ViewState,
    frame: FrameId,
    call: Cid,
    update: impl FnOnce(&mut ToolActivity),
) {
    if let Some(activity) = state
        .tools
        .iter_mut()
        .find(|tool| tool.frame == frame && tool.call == call)
    {
        update(activity);
    } else {
        let mut activity = ToolActivity {
            frame,
            call,
            title: None,
            status: ToolStatus::Running,
            latency_us: None,
            expanded: None,
        };
        update(&mut activity);
        state.tools.push(activity);
    }
}

/// Fold a single record into frontend state.
///
/// # Errors
/// Returns `SeqGap` or `OutOfOrder` before mutation when the record does not follow the last one.
pub fn reduce(state: &mut ViewState, record: &EventRecord) -> Result<(), ReduceError> {
    let seq = record.seq();
    if let Some(last) = state.last_seq {
        if seq <= last {
            return Err(ReduceError::OutOfOrder { last, got: seq });
        }
        if let Some(expected) = last.next() {
            if seq != expected {
                return Err(ReduceError::SeqGap { expected, got: seq });
            }
        } else {
            return Err(ReduceError::OutOfOrder { last, got: seq });
        }
    }
    match *record.event() {
        Event::SessionStarted { workspace } => {
            if let Some(row) = state
                .sessions
                .iter_mut()
                .find(|row| row.session == record.session())
            {
                row.workspace = Some(workspace);
                row.closed = false;
                row.started_at_ms = record.at().as_millisecond();
                row.started_seq = seq;
            } else {
                state.sessions.push(SessionRow {
                    session: record.session(),
                    workspace: Some(workspace),
                    started_at_ms: record.at().as_millisecond(),
                    started_seq: seq,
                    closed: false,
                });
            }
            state.status.session_open = true;
        }
        Event::SessionConfigured { .. } => {}
        Event::SessionPaused | Event::SessionResumed => {
            if let Some(row) = state
                .sessions
                .iter_mut()
                .find(|row| row.session == record.session())
            {
                row.closed = false;
            } else {
                // Partial replay may begin at a lifecycle transition. Unknown workspace/start
                // remain explicit, using the same first-observed-row convention as close events.
                state.sessions.push(SessionRow {
                    session: record.session(),
                    workspace: None,
                    started_at_ms: record.at().as_millisecond(),
                    started_seq: seq,
                    closed: false,
                });
            }
            state.status.session_open = true;
        }
        Event::SessionClosed => {
            if let Some(row) = state
                .sessions
                .iter_mut()
                .find(|row| row.session == record.session())
            {
                row.closed = true;
            } else {
                state.sessions.push(SessionRow {
                    session: record.session(),
                    workspace: None,
                    started_at_ms: record.at().as_millisecond(),
                    started_seq: seq,
                    closed: true,
                });
            }
            state.status.session_open = false;
        }
        Event::ToolExecutionStarted { frame, call } => {
            update_tool(state, frame, call, |_| {});
        }
        Event::ToolExecutionCompleted {
            frame,
            call,
            ok,
            latency_us,
        } => {
            update_tool(state, frame, call, |activity| {
                activity.status = if ok {
                    ToolStatus::Succeeded
                } else {
                    ToolStatus::Failed
                };
                activity.latency_us = Some(latency_us);
            });
        }
        Event::ModelRequestCompleted {
            prompt_tokens,
            completion_tokens,
            cached_tokens,
            cost,
            latency_us,
            ..
        } => {
            let hud = &mut state.hud;
            hud.requests = hud.requests.saturating_add(1);
            hud.prompt_tokens = hud
                .prompt_tokens
                .saturating_add(u64::from(prompt_tokens.get()));
            hud.completion_tokens = hud
                .completion_tokens
                .saturating_add(u64::from(completion_tokens.get()));
            hud.cached_tokens = hud
                .cached_tokens
                .saturating_add(u64::from(cached_tokens.get()));
            hud.cost_micro_usd = hud.cost_micro_usd.saturating_add(cost.get());
            hud.last_prompt_tokens = prompt_tokens.get();
            hud.last_s2_latency_us = Some(latency_us);
        }
        Event::DecisionMade { latency_us, .. } => {
            state.hud.last_s1_latency_us = Some(latency_us);
            state.hud.decisions = state.hud.decisions.saturating_add(1);
        }
        Event::WorkerRestarted { worker, restarts } => match worker {
            WorkerKind::S1 => state.status.worker_restarts.s1 = restarts,
            WorkerKind::Ingest => state.status.worker_restarts.ingest = restarts,
            WorkerKind::Nano => state.status.worker_restarts.nano = restarts,
        },
        Event::CircuitTripped { .. } => {
            state.status.circuit_trips = state.status.circuit_trips.saturating_add(1)
        }
        Event::DeadLettered { .. } => {
            state.status.dead_letters = state.status.dead_letters.saturating_add(1)
        }
        Event::DiffApproved { .. } => {
            state.status.diffs_approved = state.status.diffs_approved.saturating_add(1)
        }
        Event::DiffRejected { .. } => {
            state.status.diffs_rejected = state.status.diffs_rejected.saturating_add(1)
        }
        Event::ProofOfDonePassed { .. } => state.status.proof_of_done = Some(ProofOutcome::Passed),
        Event::ProofOfDoneBlocked { .. } => {
            state.status.proof_of_done = Some(ProofOutcome::Blocked)
        }
        Event::DirectiveApplied { .. } => {
            state.status.directives_applied = state.status.directives_applied.saturating_add(1)
        }
        Event::FileLeaseGranted { .. }
        | Event::FileLeaseRevoked { .. }
        | Event::SandboxSnapshotBranchCreated { .. }
        | Event::SandboxExecutionCompleted { .. }
        | Event::BlastRadiusAlert { .. }
        | Event::FrameStepRecorded { .. } => {}
    }
    state.last_seq = Some(seq);
    Ok(())
}
