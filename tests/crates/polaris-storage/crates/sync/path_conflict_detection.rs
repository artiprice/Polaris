//! Path Conflict Detection (descriptor: specs/descriptors/sync_path_conflict_detection.json).
//! Hostile names exist only in memory: they are generated here, never written to disk.

use std::error::Error;

use polaris_sdk::ids::Cid;
use polaris_storage_sync::{
    EntryKind, FileEntry, Manifest, PathConflict, RelPath, detect_conflicts,
};

type TestResult = Result<(), Box<dyn Error>>;

fn manifest(paths: &[&str]) -> Result<Manifest, Box<dyn Error>> {
    let mut m = Manifest::default();
    for p in paths {
        let entry = FileEntry {
            size: 0,
            hash: Cid::from_bytes([0; 32]),
            executable: false,
            kind: EntryKind::File,
        };
        m.entries.insert(RelPath::new(p)?, entry);
    }
    Ok(m)
}

fn rel(p: &str) -> Result<RelPath, Box<dyn Error>> {
    Ok(RelPath::new(p)?)
}

#[test]
fn case_only_pair_flagged() -> TestResult {
    let c = detect_conflicts(&manifest(&[
        "docs/Readme.md",
        "docs/README.md",
        "docs/other.md",
    ])?);
    assert_eq!(
        c,
        vec![PathConflict::CaseOnly {
            a: rel("docs/README.md")?,
            b: rel("docs/Readme.md")?
        }]
    );
    Ok(())
}

#[test]
fn reserved_name_flagged() -> TestResult {
    let c = detect_conflicts(&manifest(&[
        "CON",
        "logs/nul.txt",
        "x/com1.tar.gz",
        "console.rs",
    ])?);
    assert_eq!(
        c,
        vec![
            PathConflict::ReservedName(rel("CON")?),
            PathConflict::ReservedName(rel("logs/nul.txt")?),
            PathConflict::ReservedName(rel("x/com1.tar.gz")?),
        ]
    );
    Ok(())
}

#[test]
fn trailing_dot_flagged() -> TestResult {
    let c = detect_conflicts(&manifest(&["notes.", "dir /f", "ok.txt"])?);
    assert_eq!(
        c,
        vec![
            PathConflict::TrailingDotOrSpace(rel("dir /f")?),
            PathConflict::TrailingDotOrSpace(rel("notes.")?)
        ]
    );
    Ok(())
}

#[test]
fn alternate_stream_flagged() -> TestResult {
    let c = detect_conflicts(&manifest(&["file.txt:hidden"])?);
    assert_eq!(
        c,
        vec![PathConflict::AlternateStream(rel("file.txt:hidden")?)]
    );
    Ok(())
}

#[test]
fn windows_invalid_char_flagged() -> TestResult {
    let c = detect_conflicts(&manifest(&["what?.md", "a|b", "tab\there"])?);
    assert_eq!(c.len(), 3);
    assert!(c.iter().all(|x| matches!(x, PathConflict::InvalidChar(_))));
    Ok(())
}

#[test]
fn too_long_component_flagged() -> TestResult {
    let long = "x".repeat(256);
    let c = detect_conflicts(&manifest(&[&long, &"y".repeat(255)])?);
    assert_eq!(c, vec![PathConflict::TooLong(rel(&long)?)]);
    Ok(())
}

#[test]
fn too_long_total_path_is_flagged_once() -> TestResult {
    let path = vec!["segment"; 38].join("/");
    assert!(path.len() > 260);
    let conflicts = detect_conflicts(&manifest(&[&path])?);
    assert_eq!(conflicts, vec![PathConflict::TooLong(rel(&path)?)]);
    Ok(())
}

#[test]
fn clean_paths_no_conflicts() -> TestResult {
    assert!(
        detect_conflicts(&manifest(&[
            "src/main.rs",
            ".github/ci.yml",
            "dossier/été.md",
            "a-b_c.d"
        ])?)
        .is_empty()
    );
    Ok(())
}
