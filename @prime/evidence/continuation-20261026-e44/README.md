# E44 DAG task interruption and scheduler evidence

VERIFIED scoped checkpoint, not complete phase or production-system acceptance.

| Command group | Exit | Raw output |
|---|---:|---|
| `members` | 0 | [members.txt](members.txt) |
| `layers` | 0 | [layers.txt](layers.txt) |
| `witness` | 0 | [witness.txt](witness.txt) |
| `workspace-check` | 0 | [workspace-check.txt](workspace-check.txt) |
| `safety-clippy` | 0 | [safety-clippy.txt](safety-clippy.txt) |
| `scoped-format` | 0 | [scoped-format.txt](scoped-format.txt) |
| `dependency-artifact-clean` | 0 | [dependency-artifact-clean.txt](dependency-artifact-clean.txt) |
| `scoped-tests` | 0 | [scoped-tests.txt](scoped-tests.txt) |
| `wsl-dag` | 0 | [wsl-dag.txt](wsl-dag.txt) |
| `render-docs` | 0 | [render-docs.txt](render-docs.txt) |
| `check-docs` | 0 | [check-docs.txt](check-docs.txt) |
| `render-docs-check` | 0 | [render-docs-check.txt](render-docs-check.txt) |

The four-package suite reports **92 passed, 0 failed, 2 ignored**.
It covers DAG, runtime facade, kernel and xtask; counts overlap earlier checkpoints and must not
be added to E43 totals. The final scoped all-target safety check denies warnings, unwrap/expect/panic,
numeric casts, indexing and unchecked arithmetic for DAG and xtask only. Full workspace all-target
compilation passes; full-workspace strict Clippy, formal proofs and complete phase gates remain open.
[commands.json](commands.json) records exact arguments/timestamps/exits and later consistency checks.
The standalone Linux DAG suite reports **12 passed, 0 failed**; this does not certify the complete
runtime on Linux. Its final capture runs a script file to avoid Windows/WSL command-string quoting.
Initial raw-capture corruption, the subsequent launcher error and the downstream missing-rlib
build failures remain under [before-final-review](before-final-review/README.md) and
[before-artifact-repair](before-artifact-repair/README.md). The dependency artifact cleanup was
confined to the workspace target directory and the three dependency packages named by the
compiler. Cargo reported that version qualifiers were ignored and removed their cached artifacts
across versions. It changed no source, dependency version, assertion or gate. The two existing pool-resize
subtractions now use checked_sub; no lint was suppressed.

## Behavior and test scope

The scheduler correlates every join with the actual Tokio task ID returned at spawn. Successful
outputs must agree with the tracked graph index, NodeId and pool. Unknown/mismatched identities
are fatal scheduler errors. Real Tokio canceled tasks produce per-node Interrupted/Aborted results,
with no invented attempt count; the panic category contains no arbitrary panic payload.
ContinueIndependent preserves independent work and marks descendants dependency-failed. FailFast
aborts and drains owned siblings. Global caller cancellation remains distinct from task interruption.
Cleanup retains completed but previously unconsumed results; scheduler-aborted pending nodes remain
Cancelled. The order_started vector is dispatch order, not a measured task-entry trace.
Retry never replays an interrupted task; only structured failures from explicitly retry-safe tasks
can consume the configured retry attempts. An interruption leaves side-effect state uncertain.

Real Tokio aborts exercise the normalization and shared result-transition helpers. Public API tests
exercise ordinary node failure policy, dependency skipping, ordering, exactly-once diamond work,
caller cancellation and dropping a started run. **Full-scheduler join interruption and actual panic
execution are UNVERIFIED.** No test intentionally invokes panic or unsafe code. The task-based abort
and drain guarantees cannot preempt native/blocking work or arbitrary detached child work created
by a backend; such work requires supervised ownership and cooperative cancellation or process isolation.

The root publicly emitted the JSON descriptor before implementation. The descriptor file and tests
were written after initial source edits; this does not satisfy the full requested test-first chronology.
The subsequent [cleanup-retention RED](review/drain-retention-red2.txt) is a compiled behavioral
failure against the extracted original abort/discard path, before that targeted behavior was changed;
the identical [GREEN](review/drain-retention-green.txt) passes after the retention fix. The earlier
drain compile failure is only compile evidence. Initial test compile and Clippy diagnostics are
retained as ordinary development failures, not pre-implementation behavioral RED. The prior
baseline package run reported zero tests; its original output was only in the agent tool transcript,
so no synthetic redirected baseline log is claimed. [Review logs](retained-review-logs.json).

E44 also replaces direct JSON indexing in an existing SBOM test with get()/as_str(), preserving its
dependency-edge assertion. No production SBOM behavior or dependency graph was changed.

## Remaining system scope

Live DAG snapshots, race/pool stress and full production agent composition are incomplete. Earlier
[E43](../continuation-20261026-e43/README.md) records bounded context archives, actual rate-limit
admission, deterministic SPDX output and the user-approved SDK schema-test correction. Its six
unmaintained findings and 804 unvetted dependencies remain failed release gates. This checkpoint
does not replace those audits or earlier real-worker/portability evidence. P1 historical acceptance
is preserved; broader P2 is paused and UI remains deferred. All mandatory open phase gates remain open.

[Final source/evidence integrity check](source-integrity.txt) confirms all 1,198 recorded hashes and the 14 preceding command records; its own invocation is appended as the fifteenth command record.
