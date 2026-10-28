# Code Intel checked-arithmetic hardening

VERIFIED scope: retained 38 `clippy::arithmetic_side_effects` diagnostics from `@prime/evidence/phase3-20261027/19-boundaries-safety-current.txt`, limited to the Code Intel library's production sources. The newly added tests exercise checked Grid size/address behavior and compare generated unary-tree distances with an independent Levenshtein implementation.

The test-first state was **compile RED**: `cargo test -p polaris-code-intel arithmetic_regressions --lib` reported unresolved `GridError`, non-fallible `Grid::new/get/set`, and the not-yet-fallible tree-distance return contract. It was not a behavioral RED because the proposed test API did not yet compile. A first oracle iteration incorrectly modeled multiple roots as one sequence and produced a behavioral mismatch (`left=[0]`, `right=[0,0]`, actual `Ok(0)` vs oracle `Ok(1)`). That test model was corrected to generate unary trees; the independent property then passed.

Final raw command output is preserved in `cargo-test.txt`, `cargo-clippy-lib.txt`, and `cargo-clippy-all-targets.txt`.

VERIFIED commands:

```powershell
cargo test -p polaris-code-intel
cargo clippy -p polaris-code-intel --lib -- -D clippy::arithmetic_side_effects -D clippy::indexing_slicing -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic
```

The package test command passed all active unit, integration, and doc tests; the existing release-mode scalability workload remained ignored by its test annotation. The strict library Clippy command passed. A broader `cargo clippy --all-targets` attempt was **not clean**: it reached unrelated pre-existing violations in `examples/scratch.rs` (three `unwrap()` calls) and existing test targets (`subtree_fingerprints.rs`, `mutation_metrics.rs`, `repo_map_ranking.rs`, `symbol_merge.rs`) for indexing/modulo/negation/increment arithmetic. Those files are outside the retained 38 library-source diagnostics; no assertions or unrelated files were changed to make that broader command pass.

The public `metrics`/`metrics_with` success signatures remain unchanged. `CodeIntelError` gains the additive `ArithmeticInvariant` error for impossible checked calculations; `Grid` now uses fallible allocation and returns typed failures for invalid coordinates. These new failures cannot be turned into a fabricated zero distance.
