//! Synthetic detector fixtures prove vault/wire validation only; live GLiNER inference is covered
//! by the separately ignored real-artifact test under the System-One engine crate.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use polaris_sdk::registry::DecisionError;
use polaris_sdk::registry::SandboxHandle;
use polaris_sdk::system_one::{PiiEntitySpan, PiiSpanDetector};
use polaris_security_automata::EntropyCfg;
use polaris_security_pii::{PiiError, PiiLimits, PiiVault, SandboxUnmaskAuthorizer};

#[derive(Debug)]
struct Detector(Result<Vec<PiiEntitySpan>, DecisionError>);

impl PiiSpanDetector for Detector {
    async fn detect_pii_spans(&self, _raw_text: &str) -> Result<Vec<PiiEntitySpan>, DecisionError> {
        self.0.clone()
    }
}

#[derive(Debug)]
struct CountingDetector(Arc<AtomicUsize>);

impl PiiSpanDetector for CountingDetector {
    async fn detect_pii_spans(&self, _raw_text: &str) -> Result<Vec<PiiEntitySpan>, DecisionError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct SlowDetector;

impl PiiSpanDetector for SlowDetector {
    async fn detect_pii_spans(&self, _raw_text: &str) -> Result<Vec<PiiEntitySpan>, DecisionError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(Vec::new())
    }
}

#[derive(Debug)]
struct Allow;

impl SandboxUnmaskAuthorizer for Allow {
    fn permits(
        &self,
        _session: polaris_sdk::ids::SessionId,
        _frame: polaris_sdk::ids::FrameId,
        _sandbox: &SandboxHandle,
    ) -> bool {
        true
    }
}

fn pii_span(start: u32, end: u32) -> Result<PiiEntitySpan, polaris_sdk::error::ParseError> {
    PiiEntitySpan::new(start, end, 0.9)
}

#[tokio::test]
async fn ner_spans_are_utf8_byte_ranges_and_reversible() -> Result<(), Box<dyn std::error::Error>> {
    let session = polaris_sdk::ids::SessionId::from_u128(1);
    let frame = polaris_sdk::ids::FrameId::from_u128(2);
    let vault = PiiVault::new(session, frame, EntropyCfg::default(), PiiLimits::default())?;
    let input = "Привет Alice alice@example.com";
    let start = u32::try_from(input.find("Alice").ok_or("missing fixture")?)?;
    let end = start.checked_add(5).ok_or("span overflow")?;
    let detector = Detector(Ok(vec![pii_span(start, end)?]));
    let masked = vault
        .mask_with_ner(&detector, input, Duration::from_secs(1))
        .await?;
    assert!(masked.report.ner_used);
    assert!(!masked.report.degraded);
    assert!(!masked.text.contains("Alice"));
    assert!(!masked.text.contains("alice@example.com"));
    let sandbox = SandboxHandle::try_from("ner-test".to_owned())?;
    let restored = vault.unmask_for_sandbox(&masked.text, &sandbox, &Allow)?;
    assert_eq!(restored.as_str(), input);
    Ok(())
}

#[tokio::test]
async fn invalid_ner_spans_fall_back_to_deterministic_masking()
-> Result<(), Box<dyn std::error::Error>> {
    let session = polaris_sdk::ids::SessionId::from_u128(3);
    let frame = polaris_sdk::ids::FrameId::from_u128(4);
    let vault = PiiVault::new(session, frame, EntropyCfg::default(), PiiLimits::default())?;
    let input = "Привет Alice alice@example.com";
    let detector = Detector(Ok(vec![pii_span(1, 2)?]));
    let masked = vault
        .mask_with_ner(&detector, input, Duration::from_secs(1))
        .await?;
    assert!(masked.report.degraded);
    assert!(!masked.report.ner_used);
    assert!(!masked.text.contains("alice@example.com"));
    Ok(())
}

#[tokio::test]
async fn unavailable_or_timed_out_ner_keeps_deterministic_masking()
-> Result<(), Box<dyn std::error::Error>> {
    let session = polaris_sdk::ids::SessionId::from_u128(5);
    let frame = polaris_sdk::ids::FrameId::from_u128(6);
    let vault = PiiVault::new(session, frame, EntropyCfg::default(), PiiLimits::default())?;
    let input = "alice@example.com";
    let unavailable = Detector(Err(DecisionError::Unavailable(
        "companion model unavailable".to_owned(),
    )));
    let masked = vault
        .mask_with_ner(&unavailable, input, Duration::from_secs(1))
        .await?;
    assert!(masked.report.degraded);
    assert!(!masked.text.contains(input));
    let timed_out = vault
        .mask_with_ner(&SlowDetector, input, Duration::from_millis(1))
        .await?;
    assert!(timed_out.report.degraded);
    assert!(!timed_out.text.contains(input));
    Ok(())
}

#[tokio::test]
async fn oversized_input_is_rejected_before_detector_call() -> Result<(), Box<dyn std::error::Error>>
{
    let limits = PiiLimits {
        max_input_bytes: 8,
        ..PiiLimits::default()
    };
    let vault = PiiVault::new(
        polaris_sdk::ids::SessionId::from_u128(7),
        polaris_sdk::ids::FrameId::from_u128(8),
        EntropyCfg::default(),
        limits,
    )?;
    let calls = Arc::new(AtomicUsize::new(0));
    let detector = CountingDetector(Arc::clone(&calls));
    assert!(matches!(
        vault
            .mask_with_ner(&detector, "alice@example.com", Duration::from_secs(1))
            .await,
        Err(PiiError::InputLimit)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn overlapping_ner_and_dfa_masks_are_combined_and_existing_marker_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let vault = PiiVault::new(
        polaris_sdk::ids::SessionId::from_u128(9),
        polaris_sdk::ids::FrameId::from_u128(10),
        EntropyCfg::default(),
        PiiLimits::default(),
    )?;
    let input = "alice@example.com";
    let end = u32::try_from(input.len())?;
    let masked = vault
        .mask_with_ner(
            &Detector(Ok(vec![pii_span(0, end)?])),
            input,
            Duration::from_secs(1),
        )
        .await?;
    assert!(!masked.text.contains(input));
    assert!(masked.report.ner_used);
    let repeated = vault
        .mask_with_ner(
            &Detector(Ok(Vec::new())),
            &masked.text,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(repeated.text, masked.text);
    let token_end = u32::try_from(masked.text.len())?;
    let overbroad = vault
        .mask_with_ner(
            &Detector(Ok(vec![pii_span(0, token_end)?])),
            &masked.text,
            Duration::from_secs(1),
        )
        .await?;
    assert_eq!(overbroad.text, masked.text);
    Ok(())
}

#[test]
fn issued_marker_scan_is_bounded_with_many_entries_and_unrelated_text()
-> Result<(), Box<dyn std::error::Error>> {
    let limits = PiiLimits {
        max_matches_per_input: 256,
        ..PiiLimits::default()
    };
    let vault = PiiVault::new(
        polaris_sdk::ids::SessionId::from_u128(11),
        polaris_sdk::ids::FrameId::from_u128(12),
        EntropyCfg::default(),
        limits,
    )?;
    let mut marker_text = String::new();
    for index in 0..96 {
        let masked = vault.mask(&format!("person{index}@example.com"))?;
        marker_text.push_str(&masked.text);
        marker_text.push('|');
    }
    let mut input = String::new();
    input.try_reserve(250_000 + marker_text.len())?;
    input.push_str(&"x".repeat(250_000));
    input.push_str(&marker_text);
    let span_end = u32::try_from(input.len())?;
    let overbroad = vault.mask_with_ner_spans(&input, &[pii_span(0, span_end)?])?;
    assert_eq!(overbroad.text, input);

    let excessive_markers = "<PII1:".repeat(257);
    assert!(matches!(
        vault.mask_with_ner_spans(&excessive_markers, &[]),
        Err(PiiError::Capacity)
    ));
    Ok(())
}
