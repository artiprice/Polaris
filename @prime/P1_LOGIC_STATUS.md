# P1 logic completion record

Snapshot: 2026-10-22. Scope: offline/keyless P1 logic, with UI deferred by the user.
**Verdict: PASS — P1 non-UI logic is complete and the required scoped checks passed.**

The full M1 milestone still requires the TUI, composer, completion UI and dump snapshots. This
record does not mark M1 or the entire original P1 UI gate PASS. P0 account requests, dependency
audits and the Slint licence decision retain their own statuses.

## Implemented scope

| Surface | Checked behavior |
|---|---|
| SDK / protocol | Validated IDs and contracts, sealed command verification state, enum forwarding, version negotiation, bounded protobuf conversions and frozen descriptor semantics |
| Configuration | Layer precedence and provenance, repository trust filtering, secret references, bounded files, BOM/CRLF, Windows locked-file retry |
| Logging | Bounded rings and drain queue, redaction before enqueue, zero-allocation warmed enqueue, JSON, UTC daily plus size rotation, flush/drop and application signal/panic hooks |
| Telemetry | Checked/saturating accounting, unknown prices remain unpriced, histograms, budgets, explicit snapshot publication and ordered replay duplicate/conflict handling |
| Code intelligence | Five grammar ABI tests, bounded/cancellable parsing, iterative hostile-depth traversal, deterministic PageRank and byte-budget fitting, skeletons, metrics, fingerprints and conflict-reporting merge |
| CAS | Atomic versioned storage, integrity/decompression/encoded-read limits, bounded streaming hash, short-reference collision handling, retry and GC |
| Journal | Atomic gap-free append, bounded async owner and group commit, cancellation-independent drain/join, lazy paged immutable-prefix replay, checkpoints and lock recovery |
| Registry | Bounded owner queue and queries, WAL, migrations, redacted history, quoted FTS, session filtering before LIMIT, Unicode snippet positions and rebuildable projections |
| Cache | Scoped BLAKE3 keys, weighted L1, actual redb L2, cancellation-safe single-flight, purge and pressure handling; semantic L3 belongs to P5 |
| Bus / directives | Bounded commands, durable append before fan-out, typed full/closed/lag, replay recovery, bounded parsing, canonical rendering and session-scoped persistence |
| Supervisor | Owned task/process supervision, restart policies/intensity, backoff, circuits, bounded DLQ, actual caller-injected health probes with timeout, native clean/failure/crash classification and joined stop |
| Native ownership / reaper | Retained process handles, explicit environment allowlist, Job Object hard-kill backstop, process guards, ordered bounded teardown, crash journal, safe orphan-path sweep and unresolved-identity reporting |
| Builtin / RTK tools | Bounded workspace inspection, pure patch application, actual CAS expansion and session-scoped recall, stable schemas/catalog, bounded diagnostic streams, preserved raw CAS input and atomic multi-process ledger reconciliation |
| Kernel / headless CLI | Actual local stores/platform/tools/verifier bundle, durable session typestate, signed directive admission, bounded receipts and replay, shutdown cancellation/panic safety, TLS provider in real main, JSONL run, doctor and RTK command |

P1 uses `LocalBackends`. Full model/sandbox/S1 execution bundles begin in P2/P3/P4; the local profile
rejects unavailable execution commands. Proposal construction never implies approval or a workspace
write. Facade exports compile; later-phase child completion is tracked separately from these P1 exports.
`docs/tool-catalog.md` comes from registered schemas and is checked against them by a native test.
[evidence/p1/source.sha256](evidence/p1/source.sha256) freezes the checked Rust/proto/descriptor/build-manifest contents.

## Required verification

The durable command/output excerpt is [evidence/p1/verification.txt](evidence/p1/verification.txt).
Full local logs are under `@prime/tmp/p1/`; successful reruns do not delete earlier failures.

| Check | Evidence / conditions |
|---|---|
| Windows workspace suite | `cargo test --workspace --no-fail-fast`: exit 0, 1009 passed / 0 failed / 14 ignored workload cases; all 14 were run separately; `workspace-tests-final4.txt` |
| Linux check leg | `verify-linux-leg.sh`: all-target workspace check and docs with warnings denied, fmt, vendored-source integrity and 93 packages; `linux-leg-final3.txt` |
| Bus / log allocation checks | Native dhat test binaries in the workspace suite; warmed enqueue/fan-out paths allocate zero blocks |
| Mandatory WSL fuzz | Six ASan targets passed on Tokio 1.54.1: directives 22,683 / RTK 34,218 / short refs 2,684,504 / eventlog 142,542 / proto 127,880 / diff 718,966 runs (61–65 s each); `fuzz-post-tokio-summary.txt` |
| Additional fuzz | `bus_ops`: 378,173 runs; registry query/migration: 63,872 / 12,297; code-intel parse/skeleton/merge: 37,823 / 39,387 / 40,764 (each at least 60 s) |
| Kani | SDK 3, directive cursor 2, bus arithmetic/admission 2, repo-map full-u32 byte-budget 1, every-CID CAS formatter 1; per-crate proof logs retained |
| Miri | SDK and directives supported pure tests; 13 bus behavior tests in `bus-miri-supported-final.txt`. SDK insta snapshot requires unsupported socketpair; Loom stack switching/getrlimit and dhat allocator instrumentation run natively |
| Real failure paths | CAS/eventlog ENOSPC on disposable WSL tmpfs, killed journal/SQLite writers, invalid capabilities, startup rollback, failed projections, cancelled shutdown waiters, native owner hard-kill and hanging health transport |
| Repository consistency | Member/layer/witness checks, descriptor JSON, generated document checks, root/fuzz formatting and dependency deny report |

Tokio 1.54.0 was yanked during this work. The workspace, fuzz and resolution-probe locks now resolve
1.54.1. The original deny failure and the passing updated result are retained; no advisory exemption
was added. `cargo vet` remains a separate P0 audit gate, not a claimed P1 audit sign-off.

## Observed workloads

These are synthetic local measurements, with the full assertions preserved in source. They are not
universal latency guarantees. Release builds were used for throughput and storage scaling runs.

| Workload | Observed result | Full local log |
|---|---|---|
| CAS, 10k small blobs | Quiescent put p99 4.947 ms; earlier concurrent run 6.172 ms missed the 5 ms target | `cas-put-quiescent-release.txt`, `cas-load-current.txt` |
| CAS streaming hash, 1 GiB | Bounded parallel hashing 3.107 GB/s; same digest as the sequential implementation | `cas-stream-rayon-release.txt` |
| Eventlog, 1M records | 269,488,128 bytes; 100k replay 85.7 ms; append p99 Eventual 29.5 us / Immediate 2.679 ms | `eventlog-load-current.txt` |
| Registry, 1M messages | 449.62 s indexing; 300,860,072 bytes; final query p99 9.111 ms. Earlier mixed-load 100k/200k points were 61.5/78.6 ms | `registry-million-current.txt` |
| Cache L1, 8 x 1M gets | 830 ms / 103 ns per operation | `cache-current-load-release.txt` |
| Code-intel map, 1k/10k/100k files | Map 2/24/287 ms; 100k graph build 1,165 ms; RSS 179,777,536 bytes | `codeintel-scalability-current.txt` |
| RTK, 100 MiB streams | JSON 320.67 MiB/s; text 240.82 MiB/s | `rtk-current-load-release.txt` |
| Directives, 256 KiB / 512 KiB / 1 MiB | 106.4 / 100.3 / 102.4 MB/s; full output counts checked | `directives-load-final2.txt` |
| Telemetry, 1M folds | 79.2572 ms | `telemetry-load-final.txt` |
| Bus, 1M events and 1/8/64 subscribers | Every subscriber receives all records; native release latency/RSS output retained | `bus-load-final-linux.txt` |
| Native worker kills | Windows: 200 kills / 199 restarts / one DLQ / zero surviving roots; Linux same counts | `lifecycle-stress-windows-final.txt`, `lifecycle-stress-linux-fixed.txt` |
| Reaper, 1000 dirs + 50 fake sandboxes + 20 native processes | Windows 2.582 s; Linux 867 ms; all resources resolved and journal clean | same lifecycle workload logs |

The first Linux mass teardown hit the 10-second deadline because it fsynced the complete journal
after every resource. Shutdown now checkpoints each 32 successful releases and always performs the
final flush on a blocking owner. An interruption between checkpoints can retain extra already-released
locators, but cannot forget a live resource. Explicit register/release operations retain atomic writes.

The first Windows owner hard-kill test left descendants alive: the raw Tokio kill-on-drop flag did
not enable the wrapper's Job Object limit. Registering `process_wrap::KillOnDrop` fixed the real test.

## Scope and verification limits

- UI and full M1 remain deferred by the user. No new UI completion is claimed.
- Full execution, S1 worker protocol/named-pipe assembly, semantic L3, remote APIs and keyed runs
  retain their later phase gates. Existing broader source compiling does not make those phases PASS.
- PID-only recovery identities are insufficient for safe automatic killing; they remain journaled
  and block startup for operator review. Linux process groups are not claimed to survive an abrupt
  owner kill with the Windows Job Object guarantee.
- Workspace reads reject lexical escapes, symlinks/reparse points and secret paths and bound opened
  reads. Canonicalize/open retains a TOCTOU window; these tools are not a native filesystem sandbox.
- C FFI paths are covered natively/fuzzed, not interpreted by Miri. Full-parser Kani exceeded the
  memory guard; the bounded cursor and budget arithmetic proofs do not claim a universal parser proof.
- Extended P10 benchmark/portability/live-service work remains separate. Observed timing outliers,
  unaccepted snapshot diagnostics and property regression seeds remain retained.
- Automatic approval rejected removing `forbid(unsafe_code)` from the later-phase sift/router crates,
  citing a weakened security boundary. Those safeguards remain; PLAN D10 has this documented
  consistency exception. No check was loosened to obtain a passing result.
