//! P7-X-2: `/hack` on the seeded-bug repository in `tests/fixtures/hack/seeded-service` finds
//! every planted bug (races, blocking calls in async code, CUDA barrier misuse) and nothing in
//! the file that does the same work correctly.

use std::path::{Path, PathBuf};

use polaris_kernel::directives::directive::NvidiaMode;
use polaris_ui_session::commands::LocalCommand;
use polaris_ui_session::hack::{hack, hack_lines};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../../tests/fixtures")
}

/// Copy a sample repository into a temporary workspace, dropping the `.fixture` suffix.
fn materialize(name: &str) -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
    let target = tempfile::tempdir()?;
    let root = fixtures().join(name);
    let mut stack = vec![root.clone()];
    while let Some(directory) = stack.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(&root)?.to_string_lossy().into_owned();
            let relative = relative.strip_suffix(".fixture").unwrap_or(&relative);
            let destination = target.path().join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, destination)?;
        }
    }
    Ok(target)
}

/// `(path, line, class)` of each `[[seed]]` in `SEEDED.toml`.
/// A planted bug: path, line, class.
type Seed = (String, usize, String);

fn seeds(workspace: &Path) -> Result<Vec<Seed>, Box<dyn std::error::Error>> {
    let text = std::fs::read_to_string(workspace.join("SEEDED.toml"))?;
    let mut seeds = Vec::new();
    for block in text.split("[[seed]]").skip(1) {
        let field = |name: &str| {
            block
                .lines()
                .find_map(|line| line.trim().strip_prefix(&format!("{name} = ")))
                .map(|value| value.trim().trim_matches('"').to_owned())
        };
        seeds.push((
            field("path").ok_or("seed path")?,
            field("line").ok_or("seed line")?.parse()?,
            field("class").ok_or("seed class")?,
        ));
    }
    Ok(seeds)
}

#[test]
fn hack_finds_every_seeded_bug_and_nothing_in_the_clean_file() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    let seeded = seeds(workspace.path())?;
    assert_eq!(seeded.len(), 5);
    let report = hack(workspace.path())?;
    let found: Vec<(&str, usize, &str)> = report
        .findings
        .iter()
        .map(|finding| (finding.path.as_str(), finding.line, finding.category))
        .collect();
    for (path, line, class) in &seeded {
        let category = match class.as_str() {
            "cuda-barrier" => "cuda",
            other => other,
        };
        assert!(
            found.contains(&(path.as_str(), *line, category)),
            "seeded {class} at {path}:{line} not found in {found:#?}"
        );
    }
    assert!(
        !found.iter().any(|(path, _, _)| *path == "src/clean.rs"),
        "the correct versions are not findings: {found:#?}"
    );
    assert_eq!(found.len(), seeded.len(), "no extra findings: {found:#?}");
    assert_eq!(
        report.analyzed,
        [("Rust", 4), ("CUDA", 1)],
        "four Rust sources and one CUDA source"
    );

    let lines = hack_lines(&report);
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("src/ids.rs:9 race [race-atomic-load-store] `NEXT_ID`")),
        "{lines:#?}"
    );
    assert!(
        lines.iter().any(|line| line.starts_with(
            "src/account.rs:22 race [race-split-lock] `self.balance` is locked again"
        )),
        "{lines:#?}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line
                .starts_with("kernels/normalize.cu:8 cuda [barrier-in-divergent-branch]")),
        "{lines:#?}"
    );
    assert!(lines.last().is_some_and(|line| line.contains("not proof")));
    Ok(())
}

#[test]
fn the_summary_counts_findings_by_category() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    let lines = hack_lines(&hack(workspace.path())?);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Analyzed 4 Rust, 1 CUDA: 5 findings (2 blocking-in-async, 1 cuda, 2 race)."),
        "{lines:#?}"
    );
    Ok(())
}

#[test]
fn a_repository_without_seeded_bugs_reports_none() -> TestResult {
    let workspace = materialize("nvidia/plain-rust")?;
    let lines = hack_lines(&hack(workspace.path())?);
    assert_eq!(
        lines.first().map(String::as_str),
        Some("Analyzed 1 Rust: 0 findings.")
    );
    Ok(())
}

#[tokio::test]
async fn hack_runs_as_a_local_command() -> TestResult {
    let workspace = materialize("hack/seeded-service")?;
    let lines = LocalCommand::Hack
        .run_with(NvidiaMode::Off, workspace.path())
        .await?;
    assert!(
        lines
            .first()
            .is_some_and(|line| line.ends_with("5 findings (2 blocking-in-async, 1 cuda, 2 race).")),
        "$nvidia(off) does not switch off /hack: {lines:#?}"
    );
    assert_eq!(LocalCommand::Hack.title(), "Hack audit");
    Ok(())
}
