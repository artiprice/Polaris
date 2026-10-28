use polaris_runtime_frames::{
    FRAME_STEP_RECORD_SCHEMA_VERSION, FrameJournalError, FrameJournalLimits, FrameStepRecord,
    WatchdogFrameMemory, reconstruct_watchdog_memory,
};
use polaris_sdk::{contracts::FrameStepOutcome, ids::FrameId};
use polaris_system_one_watchdog::{ProposedStep, StepIndex, StepOutcome, build_step_record};

fn record(frame: FrameId, watchdog_memory: Option<WatchdogFrameMemory>) -> FrameStepRecord {
    FrameStepRecord {
        schema: FRAME_STEP_RECORD_SCHEMA_VERSION,
        frame,
        step: 2,
        goal: "inspect source".to_owned(),
        request: None,
        assistant_response: None,
        result: None,
        executed_tool_results: Vec::new(),
        watchdog_memory,
        outcome: FrameStepOutcome::Failed,
    }
}

#[test]
fn watchdog_memory_roundtrips_and_rejects_foreign_frame() -> Result<(), String> {
    let frame = FrameId::from_u128(701);
    let mut step = build_step_record(
        StepIndex::new(2),
        &ProposedStep {
            thought: "retry with a smaller change".to_owned(),
            tool_calls: Vec::new(),
        },
        3,
        1024,
        StepOutcome::Unknown,
    )
    .map_err(|error| error.to_string())?;
    let hypothesis = step.fingerprint;
    step.outcome = StepOutcome::RejectedPatch { hypothesis };
    let memory = WatchdogFrameMemory {
        version: 1,
        frame,
        step: 2,
        recent: vec![step],
        negative_constraints: Vec::new(),
    };
    let bytes = serde_json::to_vec(&record(frame, Some(memory.clone())))
        .map_err(|error| error.to_string())?;
    let recovered =
        reconstruct_watchdog_memory([bytes.as_slice()], frame, FrameJournalLimits::default())
            .map_err(|error| error.to_string())?;
    if recovered != Some(memory) {
        return Err("watchdog memory did not roundtrip from journal bytes".to_owned());
    }
    let foreign = reconstruct_watchdog_memory(
        [bytes.as_slice()],
        FrameId::from_u128(702),
        FrameJournalLimits::default(),
    );
    if !matches!(foreign, Err(FrameJournalError::InvalidWatchdogMemory)) {
        return Err("foreign frame watchdog memory was not rejected".to_owned());
    }
    Ok(())
}

#[test]
fn legacy_frame_records_without_watchdog_memory_remain_replayable() -> Result<(), String> {
    let frame = FrameId::from_u128(703);
    let bytes = serde_json::to_vec(&record(frame, None)).map_err(|error| error.to_string())?;
    let recovered =
        reconstruct_watchdog_memory([bytes.as_slice()], frame, FrameJournalLimits::default())
            .map_err(|error| error.to_string())?;
    if recovered.is_some() {
        return Err("legacy record unexpectedly produced watchdog state".to_owned());
    }
    Ok(())
}
