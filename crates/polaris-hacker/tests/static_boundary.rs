use polaris_code_intel::Lang;
use polaris_hacker::{BoundaryGen, BoundaryLimits, SourceFile, static_scan};
use polaris_security_automata::DfaScanner;

fn file(path: &str, source: &str) -> SourceFile {
    SourceFile {
        path: path.to_owned(),
        lang: Lang::Rust,
        bytes: source.as_bytes().to_vec(),
    }
}

#[test]
fn finds_seeded_async_blocking_call_and_ignores_sync_negative()
-> Result<(), Box<dyn std::error::Error>> {
    let sources = [
        file(
            "src/bug.rs",
            "async fn seeded_bug() { std::thread::sleep(std::time::Duration::ZERO); }",
        ),
        file(
            "src/sync.rs",
            "fn ordinary() { std::thread::sleep(std::time::Duration::ZERO); }",
        ),
        file(
            "src/comment.rs",
            "// async fn fake() { std::thread::sleep(x); }\nfn safe() {}",
        ),
    ];
    let rules = DfaScanner::default_pack()?;
    let report = static_scan(&sources, &rules)?;
    assert_eq!(report.findings.len(), 1);
    let Some(finding) = report.findings.first() else {
        return Err("expected one seeded finding".into());
    };
    assert_eq!(finding.path, "src/bug.rs");
    assert_eq!(finding.rule, "async-blocking-thread-sleep");
    assert!(finding.end > finding.start);
    assert!(finding.evidence.contains("std::thread::sleep"));
    Ok(())
}

#[test]
fn boundary_corpus_is_seeded_and_bounded() {
    let limits = BoundaryLimits {
        max_cases: 24,
        max_case_bytes: 64,
    };
    let first = BoundaryGen::generate(7, limits);
    assert_eq!(first, BoundaryGen::generate(7, limits));
    assert!(first.len() <= limits.max_cases);
    assert!(
        first
            .iter()
            .all(|input| input.len() <= limits.max_case_bytes)
    );
    assert!(first.iter().any(|input| input == b"0"));
    assert!(first.iter().any(|input| input == b"-1"));
    assert!(
        first
            .iter()
            .any(|input| input.len() == limits.max_case_bytes)
    );
}

#[test]
fn ast_and_dfa_findings_keep_locations_without_copying_secret_bytes()
-> Result<(), Box<dyn std::error::Error>> {
    use polaris_hacker::{Confidence, FindingClass};

    let source = "#[test]\nfn weak_check() { assert!(true); }\nfn risky() { unsafe { let token = \"inline-credential-abcdefghijklmnopqrstuvwx\"; core::hint::black_box(0); } }\nfn secret() { let token = \"abcdefghijklmnopqrstuvwx\"; }\n";
    let sources = [file("src/evidence.rs", source)];
    let rules = DfaScanner::default_pack()?;
    let report = static_scan(&sources, &rules)?;
    assert!(report.findings.iter().any(|finding| {
        finding.class == FindingClass::TautologicalAssertion
            && finding.confidence == Confidence::DeterministicObservation
            && finding.evidence.starts_with("ast-tautology:")
            && source.get(finding.start..finding.end).is_some()
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.class == FindingClass::UnsafeConstruct
            && finding.evidence.starts_with("ast-unsafe:")
            && source.get(finding.start..finding.end).is_some()
    }));
    assert!(report.findings.iter().any(|finding| {
        finding.class == FindingClass::RuleMatch
            && finding.rule == "secret_assignment"
            && !finding.evidence.contains("abcdefghijklmnopqrstuvwx")
            && finding.start < finding.end
    }));
    assert!(report.findings.iter().all(|finding| {
        !finding
            .evidence
            .contains("inline-credential-abcdefghijklmnopqrstuvwx")
            && !finding.evidence.contains("abcdefghijklmnopqrstuvwx")
    }));
    Ok(())
}

#[test]
fn rejects_file_count_and_byte_budgets_before_parsing() -> Result<(), Box<dyn std::error::Error>> {
    let rules = DfaScanner::default_pack()?;
    let too_many = (0..129)
        .map(|index| file(&format!("src/{index}.rs"), ""))
        .collect::<Vec<_>>();
    assert!(static_scan(&too_many, &rules).is_err());

    let oversized = SourceFile {
        path: "src/large.rs".to_owned(),
        lang: Lang::Rust,
        bytes: vec![b' '; 4 * 1024 * 1024 + 1],
    };
    assert!(static_scan(&[oversized], &rules).is_err());

    let oversized_aggregate = (0..9)
        .map(|index| SourceFile {
            path: format!("src/part-{index}.rs"),
            lang: Lang::Rust,
            bytes: vec![b' '; 4 * 1024 * 1024],
        })
        .collect::<Vec<_>>();
    assert!(static_scan(&oversized_aggregate, &rules).is_err());

    let many_findings = format!("fn risky() {{ {} }}", "unsafe {{}} ".repeat(8193));
    assert!(static_scan(&[file("src/many.rs", &many_findings)], &rules).is_err());
    Ok(())
}
