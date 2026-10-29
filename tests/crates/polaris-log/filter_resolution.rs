//! Filter Resolution (descriptor: specs/descriptors/log_filter_resolution.json).

use std::ffi::OsStr;

use polaris_log::{LogError, resolve_filter};

#[test]
fn explicit_level_wins_over_env() -> Result<(), LogError> {
    let f = resolve_filter("warn", Some(OsStr::new("trace")))?;
    let text = f.to_string();
    assert!(text.contains("warn"), "{text}");
    assert!(!text.contains("trace"), "{text}");
    Ok(())
}

#[test]
fn empty_level_uses_env() -> Result<(), LogError> {
    let f = resolve_filter("", Some(OsStr::new("polaris=debug")))?;
    assert!(f.to_string().contains("polaris=debug"));
    let f = resolve_filter("   ", Some(OsStr::new("error")))?;
    assert!(f.to_string().contains("error"));
    Ok(())
}

#[test]
fn nothing_set_defaults_to_info() -> Result<(), LogError> {
    let f = resolve_filter("", None)?;
    assert!(f.to_string().contains("info"));
    let f = resolve_filter("", Some(OsStr::new("")))?;
    assert!(f.to_string().contains("info"));
    Ok(())
}

#[test]
fn bad_level_is_error() {
    assert!(matches!(
        resolve_filter("polaris=loud", None),
        Err(LogError::BadFilter(_))
    ));
}

#[test]
fn bad_env_is_error_when_it_is_used() {
    assert!(matches!(
        resolve_filter("", Some(OsStr::new("polaris=loud"))),
        Err(LogError::BadFilter(_))
    ));
    // An explicit valid level never looks at the (broken) environment value.
    assert!(resolve_filter("info", Some(OsStr::new("polaris=loud"))).is_ok());
}

#[cfg(windows)]
#[test]
fn non_unicode_env_is_error() {
    use std::os::windows::ffi::OsStringExt;
    let bad = std::ffi::OsString::from_wide(&[0x0069, 0xD800]);
    assert!(matches!(
        resolve_filter("", Some(bad.as_os_str())),
        Err(LogError::BadFilter(_))
    ));
}
