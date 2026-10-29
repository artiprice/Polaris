use polaris_code_intel::ByteSpan;
use polaris_security_rules::{DiffAuditRequest, RuleEngine};
use std::error::Error;
use std::path::Path;

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn explicit_parent_component_in_joined_open_path_is_reported() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"open(os.path.join(base, '..', filename))\n";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/read.py"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    let finding = report
        .findings()
        .iter()
        .find(|finding| finding.rule().as_str() == "python-parent-path-component")
        .ok_or("missing parent-path finding")?;
    assert_eq!(
        finding.cwe().map(polaris_security_rules::Cwe::get),
        Some(22)
    );

    let benign = b"open(os.path.join(base, 'assets', filename))\n";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request =
        DiffAuditRequest::new(Path::new("src/read.py"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "python-parent-path-component")
    );
    Ok(())
}

#[test]
fn repeated_adjacent_free_in_one_compound_statement_is_reported() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"void release(int* ptr) { free(ptr); free(ptr); }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/release.cpp"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(
        report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-repeated-free")
    );

    let benign = b"void release(int* ptr) { free(ptr); ptr = nullptr; }";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request =
        DiffAuditRequest::new(Path::new("src/release.cpp"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-repeated-free")
    );
    Ok(())
}

#[test]
fn repeated_adjacent_delete_in_one_compound_statement_is_reported() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"void release(int* ptr) { delete ptr; delete ptr; }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/release.cpp"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(
        report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-repeated-delete")
    );

    let benign = b"void release(int* ptr) { delete ptr; ptr = nullptr; }";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request =
        DiffAuditRequest::new(Path::new("src/release.cpp"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-repeated-delete")
    );
    Ok(())
}

#[test]
fn freed_pointer_use_in_same_compound_statement_is_reported_as_potential() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"void f(Item *ptr) { { free(ptr); ptr->value; } }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/use.cpp"), b"", source, &hunks);
    let findings = engine.audit_diff(&request)?.findings().to_vec();
    let finding = findings
        .iter()
        .find(|finding| finding.rule().as_str() == "cpp-free-then-dereference")
        .ok_or("missing potential use-after-free indicator")?;
    assert_eq!(
        finding.cwe().map(polaris_security_rules::Cwe::get),
        Some(416)
    );
    assert!(finding.disputed());
    assert!(finding.message().to_ascii_lowercase().contains("potential"));
    Ok(())
}

#[test]
fn reassigned_pointer_after_free_is_not_reported_as_same_expression_uaf() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"void f(Item *ptr) { { free(ptr); ptr = allocate(); ptr->value; } }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/reassigned.cpp"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(
        !report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-free-then-dereference")
    );
    Ok(())
}

#[test]
fn other_pointer_after_free_is_not_reported_as_same_expression_uaf() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"void f(Item *ptr, Item *other) { { free(ptr); other->value; } }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/other-pointer.cpp"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(
        !report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cpp-free-then-dereference")
    );
    Ok(())
}

#[test]
fn symlink_check_then_open_of_same_path_is_reported() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"fn read(path: &std::path::Path) { if std::fs::symlink_metadata(path).is_ok() { std::fs::File::open(path); } }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/read.rs"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    let finding = report
        .findings()
        .iter()
        .find(|finding| finding.rule().as_str() == "rust-symlink-check-open")
        .ok_or("missing symlink race finding")?;
    assert_eq!(
        finding.cwe().map(polaris_security_rules::Cwe::get),
        Some(59)
    );

    let benign = b"fn read(path: &std::path::Path) { let _file = std::fs::File::open(path); }";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request =
        DiffAuditRequest::new(Path::new("src/read.rs"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "rust-symlink-check-open")
    );
    Ok(())
}

#[test]
fn direct_cuda_thread_index_access_is_an_informational_review_hint() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"__global__ void kernel(int* values) { int value = values[threadIdx.x]; }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/kernel.cu"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(report.findings().iter().any(|finding| {
        finding.rule().as_str() == "cuda-thread-index-array-access"
            && finding.severity() == polaris_security_rules::Severity::Info
    }));

    let bounded = b"__global__ void kernel() { __shared__ int values[256]; if (threadIdx.x < 256) { values[threadIdx.x] = 0; } }";
    let bounded_hunks = [ByteSpan::new(0, bounded.len())];
    let bounded_request =
        DiffAuditRequest::new(Path::new("src/kernel.cu"), b"", bounded, &bounded_hunks);
    let bounded_report = engine.audit_diff(&bounded_request)?;
    assert!(
        bounded_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cuda-thread-index-array-access")
    );
    let benign = b"__global__ void kernel() { int value = 1; }";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request =
        DiffAuditRequest::new(Path::new("src/kernel.cu"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "cuda-thread-index-array-access")
    );
    Ok(())
}

#[test]
fn rust_subprocess_arguments_are_reviewed_for_taint() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let source = b"fn run(input: &str) { std::process::Command::new(\"sh\").arg(input); }";
    let hunks = [ByteSpan::new(0, source.len())];
    let request = DiffAuditRequest::new(Path::new("src/run.rs"), b"", source, &hunks);
    let report = engine.audit_diff(&request)?;
    assert!(
        report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "rust-process-command-arg")
    );

    let benign = b"fn run() { println!(\"safe\"); }";
    let benign_hunks = [ByteSpan::new(0, benign.len())];
    let benign_request = DiffAuditRequest::new(Path::new("src/run.rs"), b"", benign, &benign_hunks);
    let benign_report = engine.audit_diff(&benign_request)?;
    assert!(
        !benign_report
            .findings()
            .iter()
            .any(|finding| finding.rule().as_str() == "rust-process-command-arg")
    );
    Ok(())
}

#[test]
fn python_shell_execution_has_a_benign_negative_case() -> TestResult {
    let engine = RuleEngine::load_embedded()?;
    let benign = b"print('ordinary output')\n";
    let hunks = [ByteSpan::new(0, benign.len())];
    let request = DiffAuditRequest::new(Path::new("src/run.py"), b"", benign, &hunks);
    assert_eq!(engine.audit_diff(&request)?.findings().len(), 0);
    Ok(())
}
