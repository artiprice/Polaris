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
| Kernel execution | `kernel-batch-execution-tests.txt`: 4 passed, 0 failed | Explicit fake batch transport: stale/replayed approval (including restart), failing/zero tests, remote source tampering, secret content, preexisting-name collision and destroy failure all block. Pass publication follows confirmed cleanup. Does not establish M2. |
| Conditional sync | `cargo test -p polaris-storage-sync --test sync_apply`: 7 passed, 0 failed | Fake transport implements conditional commits; intervening remote edits survive replace/delete, and cancelled deletion is never reported as performed. Real transport/fidelity/performance remains pending. |
| VCS | `vcs-audit-current.txt`: 6 passed, 0 failed | Real gix/git status/diff, CRLF undo/conflict preservation, worktree lifecycle, persistent crash recovery and multi-action rollback. Full requirement audit remains open. |
| Guest sync commit | `guest-cas-current.txt`: 9 Linux tests passed; `guest-snapshot-final.txt`: 3 passed | Private initialization, conditional file/link commit and immutable paged snapshots. New batch helper and production root confinement require their own current verification. |
| CLI and doctor | `kernel-cli-targeted-current.txt`: 2 doctor contracts passed; `doctor-report-current.json`: all 26 authoritative inventory items visible | Actual machine report retains missing/unknown prerequisites; subsequent probe corrections need a fresh report. No secret payloads belong in probe output. |
| Durable approval wire contract | `blast-alert-wire-store-tests2.txt`: SDK/protobuf/eventlog suites and doctests passed | Additive protobuf tag 22, schema 3 alert minimum, legacy schemas remain readable. Broader consumer regression and Kani recheck pending. |
| Lease lookup | `lease-lookup-cached-hash-release.txt`: 10,389,319 lookups/s, 200 ns p99 at 1,024 holders; `leases-indexed-contracts.txt`: allocation contract passed | Later cached-hash change needs allocation/correctness rerun; formal/fuzz/model gates remain open. |
| WSL lifecycle | `wsl-platform-final-verified.txt`: 10 passed | Real cancellation, timeout descendant ownership, KeepAlive drop, bounded output and exact bytes. |
| Automata | `automata-performance-release-final-memory.txt`: 10/100/1,000-rule engine memory and throughput table passed; `security-automata-facade-final3.txt`: stream/facade contracts passed | Engine memory is not isolated dense-DFA state count. Formal/fuzz/bypass/fallback audit remains open. |
| Burst driver | `sandbox-burst-contracts2.txt`: 4 fake-gateway contracts passed, live test explicitly ignored | Concurrency, cleanup retries and collision ownership; does not establish real 20 × 5 stress. |

## Required work still open

- P2-X-1: rebuild and verify the final image containing both the real Rust toolchain and
  current guest `sync-cas`; record its actual local image ID, base digest and blueprint.
  A preliminary image built, but it is not final evidence for the current guest source.
- OpenShell live adapter: byte transport was repaired. Intermediate image now reaches real Cargo;
  `openshell-live-quota-intermediate-retry.txt` passed with real Docker CPU/memory limits. Final
  batch-helper image, deletion absence and full stress evidence remain required.
- P2-X-2: execute the actual doctor report, verify every inventory entry's probe coverage,
  credential presence without payloads, concrete onboarding actions, and machine setup limits.
- Process/watch/PTY: explicit 1,000 spawn/stop, depth-5, 100k event and 100 MiB stream runs passed
  (`process-stress-*.txt`); final source/evidence audit and end-to-end checkout/sync remain open.
- Sync: full remote boundedness, deny rules, case/link/mode/EOL fidelity, 10k-file performance,
  cancellation/retry cleanup and actual conditional helper integration.
- VCS: every PLAN requirement, persistent crash recovery, rollback conflicts, git configuration
  and execution safety, Windows path/case constraints and broader adversarial tests.
- Guards/automata/leases: complete their explicit PLAN tests, performance, allocation and
  applicable fuzz/proof requirements. Earlier 100k blast/classifier release measurements passed;
  retained logs must be included in the final requirement audit.
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
