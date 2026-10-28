Math crate verification — 2026-10-27

Windows commands and output are captured verbatim in windows-verification.txt.

WSL environment inspection (Ubuntu-24.04):
COMMAND: rustup show active-toolchain && rustc --version && cargo --version && cargo miri --version; cargo kani --version; cargo +nightly miri --version; cargo +nightly kani --version
OUTPUT:
1.99.0-x86_64-unknown-linux-gnu (overridden by '/mnt/c/mDev/Polaris/rust-toolchain.toml')
rustc 1.99.0 (b940084d7 2026-09-28)
cargo 1.99.0 (5f94df478 2026-08-27)
error: the 'miri' component which provides the command 'cargo-miri' is not available for the '1.99.0-x86_64-unknown-linux-gnu' toolchain
Kani Rust Verifier 0.68.0 (cargo plugin)
CBMC 6.11.0
miri 0.1.0 (07e526db9d 2026-10-19)
Kani Rust Verifier 0.68.0 (cargo plugin)
CBMC 6.11.0

Linux verification command requested:
CARGO_BUILD_JOBS=1 cargo test --offline --locked -p polaris-tools-math

At capture time this command was queued behind the shared target-directory lock. The active cargo process was `cargo test --locked -p polaris-kernel --test model_lifecycle -- --nocapture`; two math test processes were waiting for the same target lock. A prior WSL run passed before the final depth-boundary edit, so that run is not evidence for the final source state. Linux verification of the final source remains pending until the queued command completes.

PLAN §13 remaining acceptance gaps noted in this focused crate audit:
- `fuzz_calc_expr` and `fuzz_gpu_table_toml` harnesses/runs were not performed here.
- Kani occupancy arithmetic harness was not run (Kani is installed; harness selection/verification not established).
- GPU per-SM values for sm_75 and sm_90 remain explicitly UNVERIFIED by PLAN; this task did not assert or source new hardware rows.
- No benchmark/performance claim was made.
