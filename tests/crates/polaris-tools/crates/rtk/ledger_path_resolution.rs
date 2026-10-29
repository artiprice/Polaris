//! Ledger path resolution (descriptor: specs/descriptors/rtk_ledger_path_resolution.json).

use std::error::Error;
use std::ffi::OsStr;
use std::fs;
use std::path::Path;

use polaris_tools_rtk::{LedgerFile, find_workspace_root};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn environment_variable_wins() -> TestResult {
    let dir = tempfile::tempdir()?;
    let env = dir.path().join("from-env.md");
    let explicit = dir.path().join("explicit.md");
    let file = LedgerFile::resolve_with(
        Some(env.as_os_str()),
        Some(explicit.as_path()),
        dir.path(),
        Some(dir.path()),
    );
    assert_eq!(file.path(), env.as_path());
    // An empty variable is "unset".
    let file = LedgerFile::resolve_with(
        Some(OsStr::new("")),
        Some(explicit.as_path()),
        dir.path(),
        Some(dir.path()),
    );
    assert_eq!(file.path(), explicit.as_path());
    Ok(())
}

#[test]
fn explicit_path_beats_default() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join(".git"))?;
    let explicit = dir.path().join("custom").join("ledger.md");
    let file = LedgerFile::resolve_with(None, Some(explicit.as_path()), dir.path(), None);
    assert_eq!(file.path(), explicit.as_path());
    Ok(())
}

#[test]
fn default_is_polaris_issues_md_at_workspace_root() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join(".git"))?;
    let nested = dir.path().join("crates").join("a").join("src");
    fs::create_dir_all(&nested)?;
    let file = LedgerFile::resolve_with(None, None, &nested, None);
    assert_eq!(file.path(), dir.path().join(".polaris").join("issues.md"));
    assert_eq!(
        find_workspace_root(&nested, None).as_deref(),
        Some(dir.path())
    );
    Ok(())
}

#[test]
fn polaris_directory_also_marks_the_root() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join(".polaris"))?;
    let nested = dir.path().join("deep").join("er");
    fs::create_dir_all(&nested)?;
    assert_eq!(
        find_workspace_root(&nested, None).as_deref(),
        Some(dir.path())
    );
    Ok(())
}

#[test]
fn no_marker_falls_back_to_start_directory() -> TestResult {
    let dir = tempfile::tempdir()?;
    let nested = dir.path().join("a").join("b");
    fs::create_dir_all(&nested)?;
    // The ceiling keeps the search inside the temp tree (a marker in a parent of the temp
    // folder, such as a home-level `.polaris`, must not influence this test).
    assert_eq!(find_workspace_root(&nested, Some(dir.path())), None);
    let file = LedgerFile::resolve_with(None, None, &nested, Some(dir.path()));
    assert_eq!(file.path(), nested.join(".polaris").join("issues.md"));
    Ok(())
}

fn write_legacy(root: &Path, text: &str) -> std::io::Result<()> {
    fs::write(root.join("ISSUES.md"), text)
}

#[test]
fn legacy_issues_md_is_migrated_once() -> TestResult {
    let dir = tempfile::tempdir()?;
    write_legacy(dir.path(), "- a legacy note\n")?;
    let file = LedgerFile::new(dir.path().join(".polaris").join("issues.md"));
    assert!(
        file.migrate_legacy(dir.path())?,
        "first call moves the file"
    );
    assert_eq!(fs::read_to_string(file.path())?, "- a legacy note\n");
    assert!(!dir.path().join("ISSUES.md").exists());
    assert!(!file.migrate_legacy(dir.path())?, "second call is a no-op");
    Ok(())
}

#[test]
fn existing_ledger_blocks_migration() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join(".polaris").join("issues.md"));
    fs::create_dir(dir.path().join(".polaris"))?;
    fs::write(file.path(), "current\n")?;
    write_legacy(dir.path(), "legacy\n")?;
    assert!(!file.migrate_legacy(dir.path())?);
    assert_eq!(fs::read_to_string(file.path())?, "current\n");
    assert_eq!(
        fs::read_to_string(dir.path().join("ISSUES.md"))?,
        "legacy\n"
    );
    Ok(())
}

#[test]
fn missing_ledger_loads_empty() -> TestResult {
    let dir = tempfile::tempdir()?;
    let file = LedgerFile::new(dir.path().join("nope").join("issues.md"));
    let ledger = file.load()?;
    assert_eq!(ledger.render(), "");
    assert!(ledger.lines().is_empty());
    Ok(())
}

#[test]
fn default_resolution_stops_at_the_home_directory() -> TestResult {
    // Real environment on purpose: a temp dir under the user's home must not resolve to the global
    // `~/.polaris` store. Meaningful wherever `~/.polaris` exists (it does on Polaris machines).
    let dir = tempfile::tempdir()?;
    let start = dir.path().join("no-marker-project");
    fs::create_dir_all(&start)?;
    let file = LedgerFile::resolve(None, &start);
    match std::env::var_os("POLARIS_ISSUES_FILE").filter(|v| !v.is_empty()) {
        Some(env) => assert_eq!(file.path(), Path::new(&env)),
        None => {
            let home_store = std::env::home_dir().map(|h| h.join(".polaris"));
            assert!(
                home_store.is_none_or(|store| !file.path().starts_with(store)),
                "ledger resolved into the global store: {}",
                file.path().display()
            );
            assert_eq!(
                file.path(),
                dunce::canonicalize(&start)?
                    .join(".polaris")
                    .join("issues.md")
            );
        }
    }
    Ok(())
}
