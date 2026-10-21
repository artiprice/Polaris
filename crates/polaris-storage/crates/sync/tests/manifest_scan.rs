//! Manifest Scan (descriptor: specs/descriptors/sync_manifest_scan.json).
//! Trees are generated at test time (no committed fixtures).

use std::error::Error;
use std::fs;
use std::path::Path;

use polaris_storage_sync::{HashMode, IgnoreRules, RelPath, scan};

type TestResult = Result<(), Box<dyn Error>>;

fn tree(root: &Path) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(root.join("src/nested"))?;
    fs::write(root.join("src/main.rs"), "fn main() {}\n")?;
    fs::write(
        root.join("src/nested/crlf.txt"),
        b"line one\r\nline two\r\n",
    )?;
    fs::write(root.join("blob.bin"), [0u8, 255, 13, 10, 0, 7])?;
    fs::write(root.join("run.sh"), "#!/bin/sh\necho hi\n")?;
    fs::write(root.join(".env"), "TOKEN=not-real")?;
    Ok(())
}

#[test]
fn scan_is_deterministic() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    let a = scan(tmp.path(), &rules, None, HashMode::Fast)?;
    let b = scan(tmp.path(), &rules, None, HashMode::Fast)?;
    assert_eq!(a.manifest, b.manifest);
    let paths: Vec<&str> = a.manifest.entries.keys().map(RelPath::as_str).collect();
    assert_eq!(
        paths,
        ["blob.bin", "run.sh", "src/main.rs", "src/nested/crlf.txt"]
    );
    Ok(())
}

#[test]
fn unchanged_file_not_rehashed_with_warm_cache() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    let cold = scan(tmp.path(), &rules, None, HashMode::Fast)?;
    assert_eq!((cold.stats.hashed, cold.stats.reused), (4, 0));
    fs::write(
        tmp.path().join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )?;
    let warm = scan(tmp.path(), &rules, Some(&cold.cache), HashMode::Fast)?;
    assert_eq!((warm.stats.hashed, warm.stats.reused), (1, 3));
    assert_ne!(warm.manifest, cold.manifest);
    Ok(())
}

#[test]
fn changed_content_same_mtime_detected_in_paranoid_mode() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let rules = IgnoreRules::new(tmp.path(), None)?;
    let first = scan(tmp.path(), &rules, None, HashMode::Fast)?;
    let path = tmp.path().join("src/main.rs");
    let mtime = fs::metadata(&path)?.modified()?;
    fs::write(&path, "fn main(){ }\n")?; // same 13 bytes as the original
    fs::File::options()
        .write(true)
        .open(&path)?
        .set_modified(mtime)?;
    let fast = scan(tmp.path(), &rules, Some(&first.cache), HashMode::Fast)?;
    assert_eq!(
        fast.manifest, first.manifest,
        "fast mode trusts size+mtime (documented limit)"
    );
    let paranoid = scan(tmp.path(), &rules, Some(&first.cache), HashMode::Paranoid)?;
    assert_ne!(paranoid.manifest, first.manifest, "paranoid mode rehashes");
    Ok(())
}

#[test]
fn denied_files_absent_from_manifest() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let rules = IgnoreRules::new(tmp.path(), Some("!.env\n"))?;
    let out = scan(tmp.path(), &rules, None, HashMode::Fast)?;
    let env = RelPath::new(".env")?;
    assert!(!out.manifest.entries.contains_key(&env));
    assert_eq!(out.denied, vec![env]);
    Ok(())
}

#[test]
fn crlf_and_binary_bytes_hashed_exactly() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let out = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?;
    for (rel, raw) in [
        ("src/nested/crlf.txt", &b"line one\r\nline two\r\n"[..]),
        ("blob.bin", &[0u8, 255, 13, 10, 0, 7][..]),
    ] {
        let entry = out
            .manifest
            .entries
            .get(&RelPath::new(rel)?)
            .ok_or("missing")?;
        assert_eq!(entry.hash.as_bytes(), blake3::hash(raw).as_bytes(), "{rel}");
        assert_eq!(entry.size, u64::try_from(raw.len())?);
    }
    Ok(())
}

#[test]
fn shebang_marks_executable() -> TestResult {
    let tmp = tempfile::tempdir()?;
    tree(tmp.path())?;
    let out = scan(
        tmp.path(),
        &IgnoreRules::new(tmp.path(), None)?,
        None,
        HashMode::Fast,
    )?;
    let exec = |p: &str| -> Result<bool, Box<dyn Error>> {
        Ok(out
            .manifest
            .entries
            .get(&RelPath::new(p)?)
            .ok_or("missing")?
            .executable)
    };
    assert!(exec("run.sh")?);
    assert!(!exec("src/main.rs")?);
    Ok(())
}

#[cfg(windows)]
#[test]
fn windows_junction_not_followed() -> TestResult {
    use polaris_storage_sync::EntryKind;
    let tmp = tempfile::tempdir()?;
    let outside = tmp.path().join("outside");
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&outside)?;
    fs::create_dir_all(&ws)?;
    fs::write(outside.join("secret.txt"), "private")?;
    fs::write(ws.join("ok.txt"), "fine")?;
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(ws.join("link"))
        .arg(&outside)
        .output()?;
    assert!(status.status.success(), "mklink /J failed: {status:?}");
    let out = scan(&ws, &IgnoreRules::new(&ws, None)?, None, HashMode::Fast)?;
    assert!(
        out.manifest
            .entries
            .keys()
            .all(|k| !k.as_str().contains("secret")),
        "junction was followed"
    );
    assert_eq!(out.escaped, vec![RelPath::new("link")?]);
    assert!(
        out.manifest
            .entries
            .values()
            .all(|e| matches!(e.kind, EntryKind::File))
    );
    Ok(())
}
