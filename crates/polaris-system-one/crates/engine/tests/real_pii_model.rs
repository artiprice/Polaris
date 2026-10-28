use std::path::PathBuf;

use polaris_system_one_engine::{GlinerPiiEngine, PiiEngineConfig};

/// Runs the exact local companion checkpoint and explicit ONNX Runtime library when installed.
///
/// Set `GLINER2_DEVICE=cpu`, `POLARIS_PII_MODEL_DIR` to the fp32_v2 model directory, and
/// `POLARIS_ORT_DYLIB` to the local ONNX Runtime library. This test never downloads artifacts.
#[test]
#[ignore = "requires explicit local PII companion model and ORT artifacts"]
fn real_pii_model_returns_bounded_raw_utf8_spans() -> Result<(), Box<dyn std::error::Error>> {
    let model = PathBuf::from(
        std::env::var_os("POLARIS_PII_MODEL_DIR")
            .ok_or("POLARIS_PII_MODEL_DIR is required for the real PII model check")?,
    );
    let runtime = PathBuf::from(
        std::env::var_os("POLARIS_ORT_DYLIB")
            .ok_or("POLARIS_ORT_DYLIB is required for the real PII model check")?,
    );
    let engine = GlinerPiiEngine::load(PiiEngineConfig::new(model, runtime))?;
    let text = "Contact Alice Example at alice.example@example.test.";
    let spans = engine.detect(text)?;
    assert!(
        !spans.is_empty(),
        "the planted person and contact should be detected"
    );
    for span in &spans {
        let start = usize::try_from(span.start())?;
        let end = usize::try_from(span.end())?;
        assert!(start < end);
        assert!(text.get(start..end).is_some());
        assert!(span.confidence().is_finite());
        assert!((0.0..=1.0).contains(&span.confidence()));
    }
    Ok(())
}
