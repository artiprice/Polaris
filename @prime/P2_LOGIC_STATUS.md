# P2 execution plane — in progress

UI is deferred by the user. The full non-UI P2 exit remains open: real sandboxed tests,
sync fidelity/performance, exact approvals and Proof-of-Done, and zero resource leaks under
the specified stress workloads must all have current evidence. This document is a working
audit, not a completion claim.

## Current checked results

| Area | Observed evidence | Limits |
|---|---|---|
| Proof parsing | `cargo test -p polaris-runtime-guards --test run_evidence`: 7 passed, 0 failed | Aggregates Cargo groups; binds both streams; rejects hidden earlier failures, malformed/overflow counts and duplicate terminal summaries. Broader proof/fuzz audit pending. |
| Scoped leases | `cargo test -p polaris-runtime-leases --test authority`: 6 passed, 0 failed | Covers immediate overlap, AST siblings/ancestors, expiry wake-up, cancellation, OCC identity/churn and 64 frames × 20 files. Allocation/proof gates still need audit. |
| Execution authority | `cargo test -p polaris-kernel-verifier --test execution_approval`: 2 passed, 0 failed | Exact signed call, frame, command kind and isolated issuer. Actual operator interface/live run remains separate evidence. |
| Kernel execution | `cargo test -p polaris-kernel --test sandbox_execution`: 3 passed, 0 failed | Explicit fake transport tests: stale/replayed review rejected before create, failing tests never publish completion, cancellation retains cleanup ownership. Does not establish M2. |
| Conditional sync | `cargo test -p polaris-storage-sync --test sync_apply`: 7 passed, 0 failed | Fake transport implements conditional commits; intervening remote edits survive replace/delete, and cancelled deletion is never reported as performed. Real transport/fidelity/performance remains pending. |
| VCS | `cargo test -p polaris-vcs --test vcs_smoke`: 4 passed, 0 failed | Real gix/git status/diff, CRLF undo and conflict preservation, linked worktree lifecycle. Full recovery/rollback audit remains pending. |
| Guest sync commit | `@prime/tmp/p2/sync-cas-linux-test4.txt`: 6 passed, 0 failed; host and Linux checks pass | Actual Linux files, lock, staged BLAKE3/mode verification, atomic rename, conditional deletion, links and unsafe-path refusal. Final image integration remains pending. |
| CLI and dependency wiring | `cargo check -p polaris-cli` passed before the WSL ownership rewrite; `workspace.py check-layers`: 93 packages, 262 edges OK | Headless sandbox review/approve and doctor inventory are implemented. Current machine execution and broad application tests pending. |

## Required work still open

- P2-X-1: rebuild and verify the final image containing both the real Rust toolchain and
  current guest `sync-cas`; record its actual local image ID, base digest and blueprint.
  A preliminary image built, but it is not final evidence for the current guest source.
- OpenShell live adapter: the actual test reached create/readiness, exec and upload, then
  found byte mismatch on download. This must be repaired and demonstrated, not counted green.
- P2-X-2: execute the actual doctor report, verify every inventory entry's probe coverage,
  credential presence without payloads, concrete onboarding actions, and machine setup limits.
- WSL: verify environment scrubbing and Linux descendant ownership on timeout and owner drop.
- Process/watch/PTY: run and inspect the full 1,000 spawn/stop, depth-5 tree, event-flood and
  100 MiB stream workloads; small targeted suites alone do not prove these requirements.
- Sync: full remote boundedness, deny rules, case/link/mode/EOL fidelity, 10k-file performance,
  cancellation/retry cleanup and actual conditional helper integration.
- VCS: every PLAN requirement, persistent crash recovery, rollback conflicts, git configuration
  and execution safety, Windows path/case constraints and broader adversarial tests.
- Guards/automata/leases: complete their explicit PLAN tests, performance, allocation and
  applicable fuzz/proof requirements; blast-radius 100k release measurement is pending.
- M2: a real headless kernel/CLI run with signed exact review, actual sandbox ID, source
  snapshot, ordinary and independent tests, RTK ledger, CAS output, durable proof events and
  confirmed destruction. No UI is required for this user-authorized acceptance surface.
- Sandbox spike: actual 20 concurrent × 5 create/exec/destroy cycles, quotas, 10k checkout
  flood, timing/memory evidence, and an authoritative zero-sandbox/process-leak report.
- Final current Windows/Linux builds, required test/check gates, source/doc/manifest consistency,
  and a requirement-by-requirement P2 completion audit.

Conditional commits serialize cooperating writers and retain the caller's lease. A generic
filesystem compare followed by rename cannot exclude an uncooperative OS writer; that limitation
is documented in the SDK contract rather than presented as an atomic filesystem CAS primitive.

Automatic approval review temporarily failed at a usage limit; the large kernel patch was not
executed then. Normal reviewed edits later resumed. A separate review rejected removal of a
duplicate manager-identity check; both checks were retained while the actual lease defects were fixed.
