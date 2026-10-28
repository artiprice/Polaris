# P4 security-rules follow-up evidence

## Scope completed

- Added bounded manifest review for caller-supplied package baselines. Supported inputs: Cargo.toml dependency sections (including package aliases), npm package.json dependency sections (including npm aliases), Python pyproject.toml project/Poetry dependencies, and requirements.txt names.
- Missing baselines return `BaselineUnavailable` and scan no dependencies. No registry data or CWE is invented. Findings are informational and identify nearest caller-supplied package names.
- Name distance uses bounded restricted Damerau-Levenshtein (adjacent transpositions count as one), with deterministic tie reporting. Python distribution-name case and separator runs are normalized; manifest bytes, path units, names, package counts, and comparison work are bounded.
- Added a disputed structural CWE-416 indicator for `free(expr)` followed by `expr->field` in one C++ block. Tests cover the positive plus reassignment and different-pointer negatives. This is not flow-sensitive lifetime analysis.

## Verification

`cargo test --package polaris-security-rules --offline --locked` passed. Raw output: `cargo-test-rules-final.txt`. It includes the preserved 31-rule-pack baseline tests, 10 focused rule-coverage tests, and 10 manifest tests: 44 integration tests total.

`cargo clippy --package polaris-security-rules --all-targets --offline --locked --no-deps -- -D warnings -D clippy::pedantic -D clippy::nursery -D unsafe_code -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::indexing_slicing -D clippy::string_slice -D clippy::arithmetic_side_effects -D clippy::as_conversions -D clippy::cast_possible_truncation -D clippy::cast_possible_wrap -D clippy::cast_precision_loss -D clippy::cast_sign_loss` passed. Raw output: `cargo-clippy-rules-strict-final.txt`.

Owned Rust files were formatted with `rustfmt --edition 2024`. Workspace-wide `cargo fmt --all -- --check` remains unverified/failed because it reports formatting differences across unrelated packages; only the rules-owned Rust files were formatted.

## Remaining limits

The name heuristic is not registry-backed and proves no malicious intent. Python requirements parsing is intentionally a bounded name extractor, not a complete pip parser. The C++ finding recognizes one narrow AST shape; it does not prove reachability, ownership, or lifetime. Broader taint, flow-sensitive UAF, race, symlink-race, CUDA precision, fuzzing, false-positive-rate measurement, and scalability work remain outside this follow-up. Root-owned PLAN/status files were not edited.
