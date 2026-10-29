//! Hud View (descriptor: specs/descriptors/telemetry_hud_view.json).

use std::error::Error;

use polaris_sdk::contracts::{Event, EventRecord, Timestamp};
use polaris_sdk::ids::{Cid, FrameId, MicroUsd, Seq, SessionId, TokenCount};
use polaris_telemetry::{HudView, Telemetry};

type TestResult = Result<(), Box<dyn Error>>;

const SESSION: SessionId = SessionId::from_u128(7);

fn request(prompt: u32, completion: u32, cached: u32, latency_us: u64) -> Event {
    Event::ModelRequestCompleted {
        frame: FrameId::from_u128(1),
        request: Cid::from_bytes([1; 32]),
        prompt_tokens: TokenCount::new(prompt),
        completion_tokens: TokenCount::new(completion),
        cached_tokens: TokenCount::new(cached),
        cost: MicroUsd::new(0),
        latency_us,
    }
}

fn decision(latency_us: u64) -> Event {
    Event::DecisionMade {
        frame: FrameId::from_u128(1),
        decision: Cid::from_bytes([2; 32]),
        latency_us,
    }
}

fn record_at(secs: i64, event: Event) -> Result<EventRecord, Box<dyn Error>> {
    Ok(EventRecord::new(
        Seq::new(1),
        Timestamp::from_second(secs)?,
        SESSION,
        event,
    ))
}

fn hud_after(events: &[Event], publish_secs: i64) -> Result<HudView, Box<dyn Error>> {
    let mut telemetry = Telemetry::new(SESSION)?;
    for event in events {
        telemetry.record(event);
    }
    Ok(telemetry
        .publish(Timestamp::from_second(publish_secs)?)
        .hud())
}

#[test]
fn empty_snapshot_hud_is_zero_and_unreported() -> TestResult {
    let telemetry = Telemetry::new(SESSION)?;
    assert_eq!(
        telemetry.snapshot().hud(),
        HudView {
            cache_hit_pct: None,
            burn_tokens_per_min: 0,
            s1_avg_us: 0,
            s2_avg_us: 0
        }
    );
    Ok(())
}

#[test]
fn cache_hit_pct_none_until_provider_reports() -> TestResult {
    let hud = hud_after(&[request(100, 10, 0, 1), request(200, 10, 0, 1)], 0)?;
    assert_eq!(hud.cache_hit_pct, None);
    Ok(())
}

#[test]
fn cache_hit_pct_is_cached_over_prompt() -> TestResult {
    let hud = hud_after(&[request(200, 5, 50, 1), request(100, 5, 0, 1)], 0)?;
    assert_eq!(hud.cache_hit_pct, Some(16));
    Ok(())
}

#[test]
fn cache_hit_pct_is_capped_at_100() -> TestResult {
    let hud = hud_after(&[request(10, 5, 50, 1)], 0)?;
    assert_eq!(hud.cache_hit_pct, Some(100));
    Ok(())
}

#[test]
fn burn_counts_only_the_last_60_seconds() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_record(&record_at(100, request(900, 100, 0, 1))?)?;
    telemetry.record_record(&record_at(130, request(1_500, 500, 0, 1))?)?;
    telemetry.record_record(&record_at(160, request(3_000, 1_000, 0, 1))?)?;

    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(160)?)
            .hud()
            .burn_tokens_per_min,
        6_000
    );
    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(189)?)
            .hud()
            .burn_tokens_per_min,
        6_000
    );
    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(190)?)
            .hud()
            .burn_tokens_per_min,
        4_000
    );
    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(220)?)
            .hud()
            .burn_tokens_per_min,
        0
    );
    Ok(())
}

#[test]
fn burn_ignores_events_without_timestamp() -> TestResult {
    let hud = hud_after(&[request(1_000_000, 1_000_000, 0, 1)], 100)?;
    assert_eq!(hud.burn_tokens_per_min, 0);
    Ok(())
}

#[test]
fn out_of_order_timestamps_inside_window_are_counted() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_record(&record_at(200, request(60, 40, 0, 1))?)?;
    telemetry.record_record(&record_at(180, request(30, 20, 0, 1))?)?;
    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(200)?)
            .hud()
            .burn_tokens_per_min,
        150
    );

    telemetry.record_record(&record_at(100, request(500, 500, 0, 1))?)?;
    let s = telemetry.publish(Timestamp::from_second(200)?);
    assert_eq!(s.hud().burn_tokens_per_min, 150);
    assert_eq!(s.usage.requests, 3);
    Ok(())
}

#[test]
fn burn_saturates_into_u32() -> TestResult {
    let mut telemetry = Telemetry::new(SESSION)?;
    telemetry.record_record(&record_at(10, request(u32::MAX, u32::MAX, 0, 1))?)?;
    assert_eq!(
        telemetry
            .publish(Timestamp::from_second(10)?)
            .hud()
            .burn_tokens_per_min,
        u32::MAX
    );
    Ok(())
}

#[test]
fn stage_averages_are_total_over_count() -> TestResult {
    let hud = hud_after(
        &[
            decision(100),
            decision(201),
            request(1, 1, 0, 10),
            request(1, 1, 0, 10),
            request(1, 1, 0, 11),
        ],
        0,
    )?;
    assert_eq!(hud.s1_avg_us, 150);
    assert_eq!(hud.s2_avg_us, 10);
    Ok(())
}
