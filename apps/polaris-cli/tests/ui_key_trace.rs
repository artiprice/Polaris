//! Actual file/diagnostic privacy checks with isolated canaries.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris_cli::key_trace::KeyTrace;
use polaris_ui_cli::input::NormalizedInput;
#[test]
fn trace_never_overwrites_existing_file() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace");
    std::fs::write(&path, "keep")?;
    assert!(
        KeyTrace::create(&path).is_err(),
        "trace overwrote an existing file"
    );
    assert_eq!(std::fs::read_to_string(path)?, "keep");
    Ok(())
}
#[test]
fn trace_excludes_draft_paste_and_printable_keys() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace");
    let mut trace = KeyTrace::create(&path)?;
    trace.record_input(&NormalizedInput::Paste(
        "PRIVATE-DRAFT-KEY-CANARY".to_owned(),
    ))?;
    trace.record_input(&NormalizedInput::Key(KeyEvent::new(
        KeyCode::Char('密'),
        KeyModifiers::NONE,
    )))?;
    trace.finish()?;
    let text = std::fs::read_to_string(path)?;
    assert!(text.contains("paste"), "category missing");
    assert!(
        !text.contains("CANARY") && !text.contains('密'),
        "private content leaked"
    );
    Ok(())
}
#[test]
fn trace_enforces_the_byte_limit_without_partial_followup() -> Result<(), Box<dyn std::error::Error>>
{
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace");
    let mut trace = KeyTrace::create(&path)?;
    let event = NormalizedInput::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    for _ in 0..104_857 {
        trace.record_input(&event)?;
    }
    let expected_bytes = 1_048_570_u64;
    assert_eq!(std::fs::metadata(&path)?.len(), expected_bytes);
    assert!(
        trace.record_input(&event).is_err(),
        "trace exceeded its 1 MiB budget"
    );
    assert_eq!(std::fs::metadata(&path)?.len(), expected_bytes);
    trace.finish()?;
    Ok(())
}

#[cfg(unix)]
#[test]
fn trace_refuses_symbolic_link_target() -> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("trace");
    let target = dir.path().join("target");
    std::fs::write(&target, "keep")?;
    std::os::unix::fs::symlink(&target, &path)?;
    assert!(KeyTrace::create(&path).is_err());
    assert_eq!(std::fs::read_to_string(target)?, "keep");
    Ok(())
}
