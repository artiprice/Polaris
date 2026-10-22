use std::collections::BTreeSet;
use std::time::Instant;

use polaris_code_intel::{ByteSpan, Skeleton, SkeletonItem, SymbolKind, SymbolRef};
use polaris_runtime_guards::{
    AstChange, AstImpactLimits, BlastInput, BlastLimits, CallSiteFacts, Confidence, FileFact,
    GuardError, ReferenceConfidence, Severity, ShellKind, SymbolReferenceView, ViewError,
    WorkspacePath, WorkspaceView, ast_blast_radius, blast_radius,
};

#[derive(Clone)]
struct FakeWorkspace {
    files: Vec<FileFact>,
    dirty: BTreeSet<WorkspacePath>,
}

impl WorkspaceView for FakeWorkspace {
    fn list_matching(
        &self,
        pattern: &polaris_runtime_guards::GlobPattern,
    ) -> Result<Vec<FileFact>, ViewError> {
        Ok(self
            .files
            .iter()
            .filter(|fact| pattern.matches(&fact.path))
            .cloned()
            .collect())
    }

    fn dirty_paths(&self) -> Result<BTreeSet<WorkspacePath>, ViewError> {
        Ok(self.dirty.clone())
    }
}

fn limits() -> BlastLimits {
    BlastLimits::new(2, 10, 20, 100, true).expect("valid test limits")
}

fn fact(path: &str, bytes: u64, uncommitted: bool) -> FileFact {
    FileFact {
        path: WorkspacePath::new(path).expect("valid test path"),
        bytes,
        uncommitted,
    }
}

fn inspect(
    command: &str,
    shell: ShellKind,
    view: &FakeWorkspace,
) -> polaris_runtime_guards::BlastReport {
    let cwd = WorkspacePath::root();
    blast_radius(
        view,
        &BlastInput {
            command,
            cwd: &cwd,
            shell,
            limits: &limits(),
            cargo_target_dir: None,
        },
    )
    .expect("blast analysis should complete")
}

#[test]
fn posix_recursive_delete_counts_bytes_and_uncommitted_paths_once() {
    let view = FakeWorkspace {
        files: vec![
            fact("build/a.o", 12, false),
            fact("build/b.o", 8, true),
            fact("build/b.o", 8, true),
            fact("source.rs", 31, true),
        ],
        dirty: [WorkspacePath::new("build/b.o").expect("path")]
            .into_iter()
            .collect(),
    };
    let report = inspect("rm -rf build", ShellKind::Posix, &view);
    assert_eq!(report.confidence, Confidence::Exact);
    assert_eq!(report.severity, Severity::Critical);
    assert_eq!(report.total_bytes, 20);
    assert_eq!(report.deleted.len(), 2);
    assert_eq!(report.uncommitted_hit.len(), 1);
}

#[test]
fn supported_shell_aliases_are_not_interpreted_as_another_shell() {
    let view = FakeWorkspace {
        files: vec![fact("build/item", 1, false)],
        dirty: BTreeSet::new(),
    };
    for (command, shell) in [
        ("rm -rf build", ShellKind::Cmd),
        ("rm -rf build", ShellKind::PowerShell),
        ("del /s /q build", ShellKind::PowerShell),
        ("move from to", ShellKind::PowerShell),
    ] {
        let report = inspect(command, shell, &view);
        assert_eq!(
            report.confidence,
            Confidence::Unknown,
            "{command} in {shell:?}"
        );
        assert_eq!(report.severity, Severity::Critical);
    }
    let cmd = inspect("del /s /q build", ShellKind::Cmd, &view);
    assert_eq!(cmd.confidence, Confidence::Exact);
    assert_eq!(cmd.deleted.len(), 1);
}

#[test]
fn shell_composition_expansion_and_escape_fail_closed() {
    let view = FakeWorkspace {
        files: vec![fact("safe/file", 7, false)],
        dirty: BTreeSet::new(),
    };
    for (command, shell) in [
        ("rm -rf safe; echo done", ShellKind::Posix),
        ("rm -rf $TARGET", ShellKind::Posix),
        ("Remove-Item $env:TEMP -Recurse", ShellKind::PowerShell),
        ("del %TEMP% /s", ShellKind::Cmd),
        ("rm -rf ../../outside", ShellKind::Posix),
        ("echo x >> safe/file", ShellKind::Posix),
    ] {
        let report = inspect(command, shell, &view);
        assert_eq!(report.confidence, Confidence::Unknown, "{command}");
        assert_eq!(report.severity, Severity::Critical);
        assert!(report.deleted.is_empty());
        assert!(report.overwritten.is_empty());
    }
}

#[test]
fn dirty_deleted_path_missing_from_tree_is_still_reported() {
    let view = FakeWorkspace {
        files: Vec::new(),
        dirty: [WorkspacePath::new("build/removed.rs").expect("path")]
            .into_iter()
            .collect(),
    };
    let report = inspect("rm -rf build", ShellKind::Posix, &view);
    assert_eq!(report.deleted.len(), 1);
    assert_eq!(report.total_bytes, 0);
    assert_eq!(report.uncommitted_hit.len(), 1);
    assert_eq!(report.severity, Severity::Critical);
}

#[test]
fn provider_duplicate_paths_do_not_inflate_bytes() {
    let view = FakeWorkspace {
        files: vec![fact("target/a", 9, false), fact("target/a", 5, false)],
        dirty: BTreeSet::new(),
    };
    let report = inspect("rm -rf target", ShellKind::Posix, &view);
    assert_eq!(report.total_bytes, 9);
    assert_eq!(report.deleted.len(), 1);
}

#[test]
fn arbitrary_command_bytes_never_panic_and_unknown_stays_critical() {
    let view = FakeWorkspace {
        files: Vec::new(),
        dirty: BTreeSet::new(),
    };
    for seed in 0_u32..10_000 {
        let bytes: Vec<u8> = (0..((seed % 97) + 1))
            .map(|offset| seed.wrapping_mul(31).wrapping_add(offset) as u8)
            .collect();
        let command = String::from_utf8_lossy(&bytes);
        let report = inspect(&command, ShellKind::Posix, &view);
        if report.confidence == Confidence::Unknown {
            assert_eq!(report.severity, Severity::Critical);
        }
    }
}

#[test]
fn blast_accounting_scales_to_a_hundred_thousand_file_view() {
    let view = FakeWorkspace {
        files: (0..100_000)
            .map(|index| fact(&format!("target/{index:06}.o"), 1, false))
            .collect(),
        dirty: BTreeSet::new(),
    };
    let target = WorkspacePath::new("target").expect("target path");
    let policy = BlastLimits::new(1, 100_000, 1, u64::MAX, false).expect("policy");
    let start = Instant::now();
    let report = blast_radius(
        &view,
        &BlastInput {
            command: "cargo clean",
            cwd: &WorkspacePath::root(),
            shell: ShellKind::Posix,
            limits: &policy,
            cargo_target_dir: Some(&target),
        },
    )
    .expect("blast estimate");
    let elapsed = start.elapsed();
    assert_eq!(report.deleted.len(), 100_000);
    assert_eq!(report.total_bytes, 100_000);
    println!("blast_100k_files_elapsed_ms={}", elapsed.as_millis());
}

#[test]
fn blast_classifier_handles_a_hundred_thousand_commands() {
    let view = FakeWorkspace {
        files: Vec::new(),
        dirty: BTreeSet::new(),
    };
    let commands = [
        "rm -rf target",
        "Remove-Item -Path build -Recurse",
        "git reset --hard",
        "echo output > generated.txt",
        "rm -rf $TARGET",
        "cmd /c del file",
    ];
    let started = Instant::now();
    for index in 0..100_000_usize {
        let command = commands[index % commands.len()];
        let shell = match index % 3 {
            0 => ShellKind::Posix,
            1 => ShellKind::PowerShell,
            _ => ShellKind::Cmd,
        };
        let report = inspect(command, shell, &view);
        if report.confidence == Confidence::Unknown {
            assert_eq!(report.severity, Severity::Critical);
        }
    }
    let elapsed = started.elapsed();
    println!(
        "blast_classifier_commands=100000;elapsed_ms={};commands_per_second={}",
        elapsed.as_millis(),
        100_000_u128.saturating_mul(1_000) / elapsed.as_millis().max(1)
    );
}

struct FixedReferences(Option<CallSiteFacts>);

impl SymbolReferenceView for FixedReferences {
    fn call_sites(&self, _symbol: &SymbolRef) -> Result<Option<CallSiteFacts>, ViewError> {
        Ok(self.0)
    }
}

fn skeleton(signature: &str, body_label: &str) -> Skeleton {
    Skeleton {
        items: vec![SkeletonItem {
            kind: SymbolKind::Function,
            name: "render".to_owned(),
            signature: signature.to_owned(),
            span: ByteSpan::new(0, body_label.len()),
            depth: 0,
        }],
    }
}

#[test]
fn ast_signature_changes_report_syntactic_calls_and_test_sites() {
    let facts = CallSiteFacts {
        call_sites: 8,
        files: 4,
        crates: 2,
        test_call_sites: 3,
        confidence: ReferenceConfidence::SyntacticEstimate,
    };
    let report = ast_blast_radius(
        &skeleton("fn render(x: i32)", "old body"),
        &skeleton("fn render(x: i64)", "new body"),
        &FixedReferences(Some(facts)),
        AstImpactLimits::new(2).expect("limits"),
    )
    .expect("AST blast estimate");
    assert!(report.references_complete);
    let impact = report.impacts.first().expect("one changed item");
    assert_eq!(impact.change, AstChange::SignatureChanged);
    assert_eq!(impact.call_sites, Some(facts));
}

#[test]
fn ast_body_only_changes_have_no_impact() {
    let report = ast_blast_radius(
        &skeleton("fn render(x: i32)", "old body"),
        &skeleton("fn render(x: i32)", "different body"),
        &FixedReferences(None),
        AstImpactLimits::new(1).expect("limits"),
    )
    .expect("AST blast estimate");
    assert!(report.impacts.is_empty());
    assert!(report.references_complete);
}

#[test]
fn ast_missing_or_unknown_reference_facts_are_never_zero_callers() {
    for facts in [
        None,
        Some(CallSiteFacts {
            call_sites: 0,
            files: 0,
            crates: 0,
            test_call_sites: 0,
            confidence: ReferenceConfidence::Unknown,
        }),
    ] {
        let report = ast_blast_radius(
            &skeleton("fn render(x: i32)", "old"),
            &skeleton("fn render(x: i64)", "new"),
            &FixedReferences(facts),
            AstImpactLimits::new(1).expect("limits"),
        )
        .expect("AST blast estimate");
        assert!(!report.references_complete);
        assert_eq!(
            report.impacts.first().and_then(|impact| impact.call_sites),
            facts
        );
    }
}

#[test]
fn ast_rejects_impossible_reference_counts_and_provider_failures() {
    let old = skeleton("fn render(x: i32)", "old");
    let new = skeleton("fn render(x: i64)", "new");
    let impossible = CallSiteFacts {
        call_sites: 2,
        files: 3,
        crates: 1,
        test_call_sites: 1,
        confidence: ReferenceConfidence::SyntacticEstimate,
    };
    assert_eq!(
        ast_blast_radius(
            &old,
            &new,
            &FixedReferences(Some(impossible)),
            AstImpactLimits::new(1).expect("limits"),
        )
        .err(),
        Some(GuardError::Workspace(ViewError::InvalidFacts))
    );

    struct OfflineReferences;
    impl SymbolReferenceView for OfflineReferences {
        fn call_sites(&self, _symbol: &SymbolRef) -> Result<Option<CallSiteFacts>, ViewError> {
            Err(ViewError::Unavailable)
        }
    }
    assert_eq!(
        ast_blast_radius(
            &old,
            &new,
            &OfflineReferences,
            AstImpactLimits::new(1).expect("limits"),
        )
        .err(),
        Some(GuardError::Workspace(ViewError::Unavailable))
    );
}

#[test]
fn ast_symbol_limit_bounds_adversarial_diffs() {
    let old = Skeleton::default();
    let new = Skeleton {
        items: (0..3)
            .map(|index| SkeletonItem {
                kind: SymbolKind::Function,
                name: format!("function_{index}"),
                signature: format!("fn function_{index}()"),
                span: ByteSpan::new(index, index + 1),
                depth: 0,
            })
            .collect(),
    };
    let result = ast_blast_radius(
        &old,
        &new,
        &FixedReferences(None),
        AstImpactLimits::new(2).expect("limits"),
    );
    assert_eq!(result.err(), Some(GuardError::TooManySymbols { limit: 2 }));
}
