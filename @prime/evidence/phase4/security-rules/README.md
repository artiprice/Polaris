# P4 security-rules evidence

## Verified

- `cargo test --package polaris-security-rules --offline` passed: 31 integration tests across rule coverage, command corpus, diff properties, S1 arbitration, and pack contracts. Raw output is in `cargo-test-rules.txt`.
- `cargo clippy --package polaris-security-rules --all-targets --offline --no-deps -- -D warnings -D clippy::pedantic -D clippy::nursery` passed. Raw output is in `cargo-clippy-rules-strict.txt`.
- `cargo test --package polaris-code-intel --offline` passed, including all 4 tests in `ast_pattern_match.rs`.
- Targeted code-intel lint passed for the changed API and its test: `cargo clippy --package polaris-code-intel --lib --test ast_pattern_match --offline --no-deps -- -D warnings`; see `cargo-clippy-code-intel-ast-targeted.txt`.
- Full code-intel `--all-targets` Clippy did not pass: pre-existing `items_after_test_module` in `src/walk.rs` and `type_complexity` in `tests/subtree_fingerprints.rs`. Raw output is in `cargo-clippy-code-intel.txt`; neither file is owned by this change.

## Coverage limits

AST packs cover Python, Rust, C++, and CUDA through the existing code-intel parser and ast-grep patterns evaluated against that parsed tree. TypeScript has a parser but no configured security patterns, so coverage is explicitly incomplete. Command DFA rules cover selected POSIX/Windows destructive commands, network-to-shell patterns, and AWS/private-key/GitHub-token shapes; this is not general secret or shell analysis.

Current AST matches are bounded structural indicators. They do not establish taint, full UAF, race freedom, bounds safety, or safe symlink handling. CUDA thread-index array access and Rust subprocess argument use are informational and intentionally have no invented CWE; a realistic guarded CUDA access is recorded as a known false positive. Repeated deallocation catches one adjacent compound-statement shape. The Rust symlink pattern catches one exact check-then-open idiom.

Still open against `@prime/PLAN.md`: flow-sensitive use-after-free and taint, broad Rust unsafe/race and symlink-race patterns, CUDA shared-memory race/dataflow and barrier-divergence precision, manifest/registry-backed typosquatting and supply-chain heuristics, arbitrary YAML/command fuzz targets, a measured benign false-positive budget, load/scalability benchmarks, and provenance-backed hostile fixtures. These gaps keep the workspace status `IN PROGRESS`.

## RED/GREEN artifact note

RED failures were observed during the implementation turn before each corresponding rule/API was completed, and successful GREEN runs are captured above. The earlier raw RED command output remains in the conversation tool log; standalone RED transcript files were not materialized here, so this directory does not claim file-based RED evidence.

Additional test-first evidence for the current supply-chain additions is materialized as `red-manifest-uaf-api.txt` (unresolved public API before manifest implementation) and `red-uaf-indicator.txt` (the new UAF regression was not reported before adding the rule). These files are raw command output; GREEN results will be appended here after implementation verification.
