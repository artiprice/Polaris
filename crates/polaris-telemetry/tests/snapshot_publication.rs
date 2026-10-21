//! Snapshot Publication (descriptor: specs/descriptors/telemetry_snapshot_publication.json).

use std::error::Error;
use std::fmt::Write;
use std::sync::atomic::{AtomicBool, Ordering};

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, ModelId, Seq, SessionId, TokenCount};
use polaris_telemetry::{StageStats, Telemetry, TelemetrySnapshot};

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn frame() -> FrameId {
    FrameId::from_u128(1)
}

fn cid(byte: u8) -> Cid {
    Cid::from_bytes([byte; 32])
}

fn request(prompt: u32, completion: u32, cached: u32, cost: u64, latency_us: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: frame(),
        request: cid(1),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(completion),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(cost),
        latency_us,
    }
}

fn record_at(seq: u64, secs: i64, event: Event) -> Result<EventRecord, Box<dyn Error>> {
    Ok(EventRecord::new(
        Seq::new(seq),
        Timestamp::from_second(secs)?,
        SESSION,
        event,
    ))
}

#[test]
fn new_telemetry_has_an_empty_snapshot() -> TestResult {
    let telemetry = Telemetry::new(SESSION)?;
    let s = telemetry.snapshot();
    assert_eq!(s.session, SESSION);
    assert_eq!(s.at, Timestamp::UNIX_EPOCH);
    assert_eq!(s.events_seen, 0);
    assert_eq!(s.usage.requests, 0);
    assert!(s.models.is_empty());
    assert_eq!(s.s1, StageStats::default());
    assert_eq!(s.s2, StageStats::default());
    assert!(!s.cache.reported);
    assert_eq!(s.cache.hit_pct(), None);
    assert_eq!(s.recent_tokens_60s, 0);
    assert!(!s.saturated);
    Ok(())
}

#[test]
fn publish_replaces_snapshot_and_old_arc_stays_unchanged() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    let old = telemetry.snapshot();
    telemetry.record(&request(10, 5, 0, 100, 50));
    assert_eq!(telemetry.snapshot().usage.requests, 0);

    let at = Timestamp::from_second(1_000)?;
    let published = telemetry.publish(at);
    assert_eq!(published.usage.requests, 1);
    assert_eq!(published.at, at);
    assert_eq!(telemetry.snapshot().usage.requests, 1);
    assert_eq!(old.usage.requests, 0);
    assert_eq!(old.at, Timestamp::UNIX_EPOCH);
    Ok(())
}

#[test]
fn reader_handle_sees_published_snapshots() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    let reader = telemetry.reader();
    assert_eq!(reader.snapshot().usage.requests, 0);
    telemetry.record(&request(1, 1, 0, 1, 1));
    assert_eq!(reader.snapshot().usage.requests, 0);
    telemetry.publish(Timestamp::UNIX_EPOCH);
    assert_eq!(reader.snapshot().usage.requests, 1);
    assert_eq!(reader.clone().snapshot().usage.requests, 1);
    Ok(())
}

#[test]
fn readers_on_four_threads_do_not_block_the_writer() -> TestResult {
    const REQUESTS: u64 = 20_000;
    let mut telemetry = Telemetry::new(SESSION)?;
    let done = AtomicBool::new(false);

    let outcomes: Vec<Result<(), String>> = std::thread::scope(|scope| {
        let readers: Vec<_> = (0..4)
            .map(|_| {
                let reader = telemetry.reader();
                let done = &done;
                scope.spawn(move || -> Result<(), String> {
                    let mut last = 0u64;
                    while !done.load(Ordering::Acquire) {
                        let s = reader.snapshot();
                        if s.usage.requests < last {
                            return Err(format!(
                                "requests went backwards: {} after {last}",
                                s.usage.requests
                            ));
                        }
                        if s.usage.requests != s.s2.count {
                            return Err("snapshot is internally inconsistent".to_owned());
                        }
                        last = s.usage.requests;
                    }
                    Ok(())
                })
            })
            .collect();

        for i in 0..REQUESTS {
            telemetry.record(&request(1, 1, 0, 1, 1 + i % 500));
            if i % 100 == 0 {
                telemetry.publish(Timestamp::UNIX_EPOCH);
            }
        }
        telemetry.publish(Timestamp::UNIX_EPOCH);
        done.store(true, Ordering::Release);

        readers
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("reader thread panicked".to_owned()))
            })
            .collect()
    });

    for outcome in outcomes {
        outcome?;
    }
    assert_eq!(telemetry.snapshot().usage.requests, REQUESTS);
    Ok(())
}

#[test]
fn snapshot_serde_round_trip() -> TestResult {
    let model = ModelId::new("m-a")?;
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_for_model(
        &model,
        &record_at(1, 500, request(100, 10, 20, 300, 1_500))?,
    )?;
    telemetry.record(&Event::SessionClosed);
    let snapshot = telemetry.publish(Timestamp::from_second(560)?);

    let json = serde_json::to_string(&*snapshot)?;
    let back: TelemetrySnapshot = serde_json::from_str(&json)?;
    assert_eq!(back, *snapshot);
    assert!(json.contains("\"m-a\""));
    Ok(())
}

fn render(s: &TelemetrySnapshot) -> Result<String, std::fmt::Error> {
    let mut out = String::new();
    writeln!(
        out,
        "events={} requests={} prompt={} completion={}",
        s.events_seen, s.usage.requests, s.usage.prompt_tokens, s.usage.completion_tokens
    )?;
    writeln!(
        out,
        "cost total={} avg={}",
        s.cost.total.get(),
        s.cost.per_request_avg.get()
    )?;
    writeln!(
        out,
        "cache reported={} prompt={} cached={} hit_pct={:?}",
        s.cache.reported,
        s.cache.prompt_tokens,
        s.cache.cached_tokens,
        s.cache.hit_pct()
    )?;
    for (name, stage, extra) in [
        ("s1", &s.s1, String::new()),
        ("s2", &s.s2, String::new()),
        (
            "tools",
            &s.tools.stage,
            format!(" failed={}", s.tools.failed),
        ),
        (
            "sandbox",
            &s.sandbox.stage,
            format!(
                " failed={} branches={}",
                s.sandbox.failed, s.sandbox.snapshot_branches
            ),
        ),
    ] {
        writeln!(
            out,
            "{name} count={} total_us={} p50={} p99={} max={}{extra}",
            stage.count, stage.total_us, stage.p50_us, stage.p99_us, stage.max_us
        )?;
    }
    for m in &s.models {
        writeln!(
            out,
            "model {} requests={} prompt={} completion={} cached={:?} cost={}",
            m.model,
            m.requests,
            m.prompt_tokens,
            m.completion_tokens,
            m.cached_tokens,
            m.cost.get()
        )?;
    }
    writeln!(
        out,
        "outcomes diffs_approved={} diffs_rejected={} proof_passed={} proof_blocked={} worker_restarts={} circuits={} dead_letters={}",
        s.outcomes.diffs_approved,
        s.outcomes.diffs_rejected,
        s.outcomes.proof_passed,
        s.outcomes.proof_blocked,
        s.outcomes.worker_restarts,
        s.outcomes.circuits_tripped,
        s.outcomes.dead_letters
    )?;
    write!(
        out,
        "recent_tokens_60s={} saturated={}",
        s.recent_tokens_60s, s.saturated
    )?;
    Ok(out)
}

/// SYNTHETIC stream (hand-built, not recorded from a live provider). All latencies stay below
/// 2048 us so the 3-significant-figure histogram reports them exactly.
#[test]
fn synthetic_stream_matches_fixture() -> TestResult {
    let m_a = ModelId::new("m-a")?;
    let m_b = ModelId::new("m-b")?;
    let mut telemetry = Telemetry::new(SESSION)?;

    telemetry.record_record(&record_at(
        1,
        990,
        Event::DecisionMade {
            frame: frame(),
            decision: cid(2),
            latency_us: 150,
        },
    )?)?;
    telemetry.record_record(&record_at(
        2,
        991,
        Event::DecisionMade {
            frame: frame(),
            decision: cid(2),
            latency_us: 350,
        },
    )?)?;
    telemetry.record_for_model(
        &m_a,
        &record_at(3, 1_000, request(1_000, 200, 0, 4_000, 900))?,
    )?;
    telemetry.record_for_model(
        &m_a,
        &record_at(4, 1_030, request(2_000, 300, 1_500, 7_000, 1_100))?,
    )?;
    telemetry.record_for_model(
        &m_b,
        &record_at(5, 1_050, request(500, 100, 0, 1_500, 400))?,
    )?;
    for (seq, ok, latency_us) in [(6, true, 25), (7, false, 75)] {
        telemetry.record_record(&record_at(
            seq,
            1_051,
            Event::ToolExecutionCompleted {
                frame: frame(),
                call: cid(3),
                ok,
                latency_us,
            },
        )?)?;
    }
    for (seq, exit_code, latency_us) in [(8, 0, 1_500), (9, 2, 1_800)] {
        telemetry.record_record(&record_at(
            seq,
            1_052,
            Event::SandboxExecutionCompleted {
                frame: frame(),
                exec: cid(4),
                exit_code,
                latency_us,
            },
        )?)?;
    }
    telemetry.record_record(&record_at(
        10,
        1_053,
        Event::DiffApproved {
            frame: frame(),
            diff: cid(5),
        },
    )?)?;

    let snapshot = telemetry.publish(Timestamp::from_second(1_060)?);
    insta::assert_snapshot!(render(&snapshot)?, @r"
    events=10 requests=3 prompt=3500 completion=600
    cost total=12500 avg=4166
    cache reported=true prompt=3500 cached=1500 hit_pct=Some(42)
    s1 count=2 total_us=500 p50=150 p99=350 max=350
    s2 count=3 total_us=2400 p50=900 p99=1100 max=1100
    tools count=2 total_us=100 p50=25 p99=75 max=75 failed=1
    sandbox count=2 total_us=3300 p50=1500 p99=1800 max=1800 failed=1 branches=0
    model m-a requests=2 prompt=3000 completion=500 cached=Some(1500) cost=11000
    model m-b requests=1 prompt=500 completion=100 cached=None cost=1500
    outcomes diffs_approved=1 diffs_rejected=0 proof_passed=0 proof_blocked=0 worker_restarts=0 circuits=0 dead_letters=0
    recent_tokens_60s=2900 saturated=false
    ");
    Ok(())
}
