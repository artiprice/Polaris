# E41 worker lifecycle, cloud contracts and CI evidence

Scoped current-tree checkpoint. Complete phase acceptance and production release remain open.

| Command group | Exit | Raw output |
|---|---:|---|
| `vendored-docx` | 0 | [vendored-docx.txt](vendored-docx.txt) |
| `vendored-docx-tests` | 0 | [vendored-docx-tests.txt](vendored-docx-tests.txt) |
| `vendored-biscuit` | 0 | [vendored-biscuit.txt](vendored-biscuit.txt) |
| `zip-graph` | 0 | [zip-graph.txt](zip-graph.txt) |
| `summary` | 0 | [summary.txt](summary.txt) |
| `members` | 0 | [members.txt](members.txt) |
| `layers` | 0 | [layers.txt](layers.txt) |
| `witness` | 0 | [witness.txt](witness.txt) |
| `workspace-check` | 0 | [workspace-check.txt](workspace-check.txt) |
| `scoped-tests` | 0 | [scoped-tests.txt](scoped-tests.txt) |
| `sdk-full-tests` | 101 | [sdk-full-tests.txt](sdk-full-tests.txt) |
| `real-model` | 0 | [real-model.txt](real-model.txt) |
| `real-worker-process` | 0 | [real-worker-process.txt](real-worker-process.txt) |
| `scoped-clippy` | 0 | [scoped-clippy.txt](scoped-clippy.txt) |
| `supervisor-production-clippy` | 0 | [supervisor-production-clippy.txt](supervisor-production-clippy.txt) |
| `supervisor-bootstrap-clippy` | 0 | [supervisor-bootstrap-clippy.txt](supervisor-bootstrap-clippy.txt) |
| `kernel-production-clippy` | 0 | [kernel-production-clippy.txt](kernel-production-clippy.txt) |
| `sdk-production-clippy` | 0 | [sdk-production-clippy.txt](sdk-production-clippy.txt) |
| `scoped-format` | 0 | [scoped-format.txt](scoped-format.txt) |
| `supervised-real-worker` | 0 | [supervised-real-worker.txt](supervised-real-worker.txt) |
| `advisories` | 1 | [advisories.txt](advisories.txt) |
| `vet` | 4294967295 | [vet.txt](vet.txt) |

[commands.json](commands.json) preserves exact arguments, timestamps and exits. The scoped suite
reports **254 passed, 0 failed, 8 ignored**. The three installed-artifact checks were separately
invoked and each passed one case: direct inference, authenticated child IPC, and the supervised
child inference path. These are individual integration cases. The SDK full suite retains **54 passed, 2 failed**;
the two schema-boundary failures are unchanged. The advisory and cargo-vet checks are failures and
remain so in this record. Other recorded commands must exit successfully before the helper writes
this checkpoint. Current advisory output records six unmaintained-crate findings; cargo-vet records
804 unvetted dependencies.

Scoped all-targets Clippy covers seven packages. Separate checks cover the supervisor production
library and bootstrap tests, kernel production library and SDK production library; these are scoped
checks, not full-workspace Clippy. The two supervisor behavioral REDs (`retirement-order-red` and
`bounded-stop-red`) precede six passing supervisor tests. The API compile RED is compile evidence,
not a behavioral regression. CI review baseline comparisons were performed after implementation;
they are not historical test-first results.

The worker path now has SDK optional-trait bootstrap, one-shot stdin credentials with explicit
cleanup truth, bounded per-generation supervisor initialization/retirement, and S1WorkerLifecycle
readiness/generation fencing through actual worker integration. Generation retirement is recorded
before slow cleanup and guarded at most once. A timeout does not establish native-work preemption or
resource release; an uncertain kill remains uncertain. The one-input real-model check is not parity
or calibration. Bearer authentication does not prove mutual server authentication or endpoint ACL
ownership.

P6 AI Cloud implements checked reservations, terminal TTL handling, strict mTLS/IP ingress checks
and owner-label selection; the facade exports the available API. Provider transport and live cloud
requests are absent. P8 CI owns cancellation cleanup and late cleanup after ambiguous create results,
with bounded execution caps. Runtime abort or a create that never resolves can leave a bounded owned
cleanup task retained without proving remote deletion. Output redaction follows the output cap; this
is not a preemptive memory limit. Live isolation, Dagger and persistent cache acceptance are absent.

P1 historical status/evidence is retained. The process P2 estimate remains 90% code / 80% verified;
its broader scope is unchanged and paused. Central dependency ownership was updated for E41 bootstrap,
client and process dependencies; the central manifest retains 167 external dependency aliases, with no new global
external alias. UI source and historical P1 verdicts are unchanged. This does not imply phase
completion.

The worker/bootstrap API is integrated as a callable kernel adapter and exercised against an actual
child process; the production application composition root still needs to select and own this adapter.
Model eviction, resource quotas, PII-NER, CUDA, OCR/nano and full parity/calibration remain unfinished.
Broad Miri/Kani/fuzz and full-workspace strict lint/release gates are not certified here.

The [first verification manifest](before-final-lifecycle-review/commands.json) and its two refreshed
raw logs are retained separately. The final manifest supersedes its workspace/scoped test records
after the retirement-order regression and equivalent runtime lint fixes. No earlier evidence is deleted.

Final consistency checks: [members](members-final.txt), [layers](layers-final.txt),
[documentation coverage](docs-final.txt), [generated blocks](docs-render-final.txt),
[percentage arithmetic and links](percentages-final.txt), and [tracked diff whitespace](diff-whitespace-final.txt)
all exited zero. [Consistency record](consistency.json) also checks the root manifest against central
metadata, validates nine JSON descriptors and rehashes 1,175 source/configuration inputs.
[Source hashes](source-hashes.json) identify the checked tree; they do not certify its correctness.

[Retained review log manifest](retained-review-logs.json) records 77 copied logs and hashes, retaining
original subdirectories to avoid attributing collided generic scratch names to the wrong component.
The supervisor [pending-stop RED](review/bounded-stop-red.txt) and
[retirement-order RED](review/retirement-order-red.txt) are compiled assertion failures, followed by
[six passing startup tests](review/retirement-order-green.txt). Process cleanup has a separate
[compiled RED](review/process-review/kill-red-stdout.log) and final native/WSL coverage, including
[12 WSL unit tests](review/process-review/wsl-fixed-stdout.log). The initial WSL failures are retained;
their correction waits for observed child exit/reaping before checking PID absence.
Cloud budget/idle regressions retain [three failing assertions](review/regression-red-stdout.log),
with [23 final policy tests](review/cloud-review/final-test-stdout.log).
The [CI review directory](review/ci-review/final-test.txt) has eight passing tests; its baseline
comparisons are post-implementation checks. API compile failures and intermediate cache/build
failures in the retained logs are not counted as behavioral failures or final acceptance.
