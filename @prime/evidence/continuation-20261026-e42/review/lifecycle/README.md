# `polaris-models` lifecycle evidence

Captured command arguments, working directory, exit codes, and merged command output are stored in the adjacent `.argv.txt` and `.log` files. Commands ran from `C:\mDev\Polaris` in PowerShell with `login=false`.

- `focused-tests`: both owned contract integration targets; 13 tests passed.
- `strict-clippy`: both owned contract targets with `-D warnings`; exit code 0.
- `json-descriptors`: both model contract descriptors parsed with `ConvertFrom-Json`; exit code 0.
- `rustfmt-check`: formatting check for the owned Rust files; exit code 0.

The `ModelLoader` docs in `crates/polaris-models/src/lifecycle.rs` now state the resource ownership contract: callers may only borrow backend use through a lease, loader APIs must not expose a handle that can escape that lease, timed-out futures may be dropped, and owned `M` resources must provide a safe `Drop` fallback. The generic API cannot enforce backend-specific handle behavior.
