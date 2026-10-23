# P2 execution plane — in progress

UI is deferred by the user. The full non-UI P2 exit remains open: real sandboxed tests,
sync fidelity/performance, exact approvals and Proof-of-Done, and zero resource leaks under
the specified stress workloads must all have current evidence. This document is a working
audit, not a completion claim.

## Current checked results

### Current live checkpoint (2026-10-23)

The earlier table below is retained as historical evidence; this checkpoint supersedes its
image, M2, burst, guest, large-file and fuzz limitations. P2 is still **IN PROGRESS**.

- Final image: `sha256:6b069f5c8a26db4d5cd53847ec19dae35f6c72f2e18ea7c34b25ad3f95af9697`.
  `openshell-image-final-build.txt` records guest source SHA256
  `954cd9be4448f1e2cca9eaaeaab3557b68f8f6d72f4c1fa6d80b32246d626461`, handshake 1 and
  Rust/Cargo 1.99.0. `openshell-live-final-image.txt`: 1 actual gateway contract passed.
  Docker inspection independently observed 1,000,000,000 nano-CPUs and 536,870,912 memory bytes.
- Headless M2: `m2-final-{review,execute,proof,exec-record,independent}.json` and both captured
  stdout files retain exact review/environment/policy/tree/image binding, a real sandbox name,
  3 ordinary tests and 2 independent widened-oracle tests. The verified CAS proof has `passed:true`.
  This is the std-only acceptance fixture, not the whole Polaris workspace test matrix.
- Current terminal-event regression: `kernel-terminal-proof-current2.txt` has 5 passing contracts.
  `headless-all-final2.txt` has a successful real run plus zero-tests, failing-command and
  insufficient-test-count failures. Each execution has exactly one terminal proof event and zero
  unresolved resources. The positive review is rejected on replay across CLI/kernel restart.
  `@prime/tools/sandbox/verify-headless.ps1` reproduces these fixed-fixture contracts.
- Real final-image 20 × 5 burst on the approved 6 GiB WSL configuration:
  `sandbox-burst-real-final-6g-retry.txt` records requested/created/executed/destroyed=100,
  peak=20, failures=[], remaining_resources=0, duration 83.67s. All 89 memory samples observed
  MemTotal=6,066,372 KiB; peak used=1,904,940 KiB and peak swap used=0. Create p50=8,222,243 µs,
  p95=10,282,108 µs, p99=11,116,864 µs. These are actual create latencies, not microVM boot claims.
  `docker-scoped-resource-audit-6g.json` independently records zero containers, volumes and
  networks in the burst namespace after native cleanup. The earlier 8 GiB/intermediate run is
  retained as historical evidence. The first new sampler attempt failed before creation because
  it read a Linux path on Windows; the sampler now uses the bounded WSL adapter on Windows.
- Real 10k-file checkout/overflow: `checkout-flood-real-final2.txt` records actual queue overflow,
  mandatory full rescan, an equal final manifest and absent `.env`/`.aws`/`.git` paths. The snapshot
  traverses multiple 4,096-entry pages. Test passed in 363.79s including fixture construction;
  sync/checkout operation reported 124,872ms. The first failed run exposed the missing AWS deny
  directory; the production deny list now covers it without weakening the exact-count assertion.
- Real S0-7 under the 6 GiB cap: `sync-s0-7-live-6g-roundtrip.txt` records a 63,963,136-byte
  large file through conditional commit, 4 initial files, replacement/addition/deletion,
  exact manifests, CRLF/executable preservation and deny rules. Independently delimited
  initial apply=6,333ms and incremental apply=5,180ms. A fresh local scan, fresh remote
  snapshot, empty plan and apply complete in 109ms (remote snapshot=108ms), below the
  unchanged 5-second limit. One live test passed in 20.93s. The historical
  `sync-s0-7-live-final-current.txt` timing labels were too broad: 18,519ms and 8,004ms
  included subsequent operations and are not isolated apply timings; its 0ms measured
  only empty-plan apply. The original artifact is retained unchanged.
- Sync bounds: a file above the 24 MiB tar threshold now uses a separately staged conditional
  commit, preserving the existing 256 MiB scan limit, secret precheck and cleanup/rescan semantics.
  `sync-large-file-final-suite.txt`: 3 unit + 50 integration tests passed. The 100 MB/1k-files
  test process measured 218,656 KiB maximum RSS and 1.97s (`sync-100mb-rss-testproc-wsl.txt`);
  compiler/Cargo memory is explicitly excluded from that workload measurement.
- Leases: `leases-diagnostics-current.txt` includes actual total ownership count, path resolution
  and zero-allocation lookup contracts. ASan fuzz logs record 228,082 key runs/61s and 84,964
  operation runs/63s. A new bounded scope Kani harness is **UNVERIFIED**: its process ended without
  a final verification result in `lease-scope-kani-current.txt`. No proof claim follows from it.
- Sync ASan fuzz: `sync-ignore-fuzz-60s-gnu.txt` records 4,764 runs/63s;
  `sync-manifest-fuzz-60s-gnu.txt` records 521,474 runs/61s, both exit 0. Sanitizer-process peak
  RSS around 435–440 MiB is not a product RSS claim.
- Confined scratch quota is implemented in the OpenShell client: a fixed `/sandbox/scratch`
  tmpfs of 1–64 MiB with no host source field. Its unit contract passes. Actual ENOSPC and
  recovery passed in `scratch-enospc-live-isolated2.txt`: independently observed 1,048,576-byte
  tmpfs, an actual ENOSPC diagnostic, outside-quota write, recovered scratch capacity, and
  combined output-cap rejection. It used an isolated gateway with bind mounts disabled.
- Actual bounded OOM: `memory-pressure-live3.txt` records independently checked cgroup v2
  memory/swap maxima of 134,217,728 bytes each. A maximum 512 MiB allocator exited 137;
  `memory.events` oom_kill increased from 0 to 1, a subsequent sandbox command succeeded,
  Reaper ownership cleared, and the exact owned Docker container was absent. One live test
  passed in 32.90s. This proves per-sandbox containment; the separate 6 GiB burst is above.
- Terminal journal-release fault: `kernel-release-terminal-red.txt` reproduces a missing
  terminal proof when an actual journal-removal error follows successful sandbox deletion.
  The corrected cleanup path retains that error and publishes one blocked proof.
  `kernel-release-terminal-green.txt`: all 5 fake-transport execution contracts pass, including
  the new journal fault and exact terminal-event count. This is separate from live M2 evidence.
- Gateway kill under the 6 GiB cap: `gateway-kill-live4-6g.txt` passed in 312.97s.
  An actual private-gateway kill during a 4 × 2 burst produced 4 created resources and
  16 typed failures, with 8 journal entries retained before recovery. Restart used the same
  database and CA. Two delete timeouts remained owned until native reconciliation passed
  its 5-minute orphan grace; authoritative absence then permitted release. Final journal,
  namespace containers, volumes and networks were empty, and the private gateway stopped.
  Historical failures and their recovery logs remain retained. An older failed scope needed
  explicit removal of exactly verified owned Docker resources; its legacy journal remains
  retained and is not counted as native recovery evidence.
- Lease FIFO now blocks only conflicting requests, allowing disjoint progress. Three Loom
  models exercise the production state transitions. Current ASan logs
  `lease-fuzz-keys-60s.txt` and `lease-fuzz-ops-60s.txt` record 247,599 and 39,855 runs in
  61 seconds each, with no reported sanitizer/assertion failure; both resource-capped scopes
  exited successfully. This supersedes the earlier fuzz evidence for the changed scheduler.

Still required: complete PLAN requirement audit; registered P2 write/proposal and sandbox-only
shell tools (the current audit found these missing from the builtin registry); remaining
VM-pressure coverage; applicable formal/model/fuzz gaps;
current Windows/Linux checks and dependency/security gates. Doctor/setup passed in §E31 and
the approved 6 GiB configuration now has live burst evidence. The UI is
deferred. The user's later all-phase request also requires complete non-UI production behavior;
new P3/P7/P8 code is not evidence that those entire phases are finished.

### Earlier check results

| Area | Observed evidence | Limits |
|---|---|---|
| Proof parsing | `cargo test -p polaris-runtime-guards --test run_evidence`: 7 passed, 0 failed | Aggregates Cargo groups; binds both streams; rejects hidden earlier failures, malformed/overflow counts and duplicate terminal summaries. Broader proof/fuzz audit pending. |
| Scoped leases | `cargo test -p polaris-runtime-leases --test authority`: 6 passed, 0 failed | Covers immediate overlap, AST siblings/ancestors, expiry wake-up, cancellation, OCC identity/churn and 64 frames × 20 files. Allocation/proof gates still need audit. |
| Execution authority | `cargo test -p polaris-kernel-verifier --test execution_approval`: 2 passed, 0 failed | Exact signed call, frame, command kind and isolated issuer. Actual operator interface/live run remains separate evidence. |
| Kernel execution | `kernel-batch-execution-tests.txt`: 4 passed, 0 failed | Explicit fake batch transport: stale/replayed approval (including restart), failing/zero tests, remote source tampering, secret content, preexisting-name collision and destroy failure all block. Pass publication follows confirmed cleanup. Does not establish M2. |
| Conditional sync | `cargo test -p polaris-storage-sync --test sync_apply`: 7 passed, 0 failed | Fake transport implements conditional commits; intervening remote edits survive replace/delete, and cancelled deletion is never reported as performed. Real transport/fidelity/performance remains pending. |
| VCS | `vcs-audit-current.txt`: 6 passed, 0 failed | Real gix/git status/diff, CRLF undo/conflict preservation, worktree lifecycle, persistent crash recovery and multi-action rollback. Full requirement audit remains open. |
| Guest sync commit | `guest-cas-current.txt`: 9 Linux tests passed; `guest-snapshot-final.txt`: 3 passed | Private initialization, conditional file/link commit and immutable paged snapshots. New batch helper and production root confinement require their own current verification. |
| CLI and doctor | `evidence/p2/doctor-cli-tests.txt`: 3 passed; `evidence/p2/process-probe-tests.txt`: 4 passed; `evidence/p2/doctor-headless-current.json`: 26/26 inventory items and onboarding actions, ready=true, required_missing=0 | P2-X-2 passes on the current machine. Credential references are checked without payloads; 7 optional later-phase items remain unresolved. An initial OpenShell BadSignature was followed by two authenticated status passes; cause is unknown and retained in evidence. |
| Durable approval wire contract | `blast-alert-wire-store-tests2.txt`: SDK/protobuf/eventlog suites and doctests passed | Additive protobuf tag 22, schema 3 alert minimum, legacy schemas remain readable. Broader consumer regression and Kani recheck pending. |
| Lease lookup | `lease-lookup-cached-hash-release.txt`: 10,389,319 lookups/s, 200 ns p99 at 1,024 holders; `leases-indexed-contracts.txt`: allocation contract passed | Later cached-hash change needs allocation/correctness rerun; formal/fuzz/model gates remain open. |
| WSL lifecycle | `wsl-platform-final-verified.txt`: 10 passed | Real cancellation, timeout descendant ownership, KeepAlive drop, bounded output and exact bytes. |
| Automata | `automata-performance-release-final-memory.txt`: 10/100/1,000-rule engine memory and throughput table passed; `security-automata-facade-final3.txt`: stream/facade contracts passed | Engine memory is not isolated dense-DFA state count. Formal/fuzz/bypass/fallback audit remains open. |
| Burst driver | `sandbox-burst-contracts2.txt`: 4 fake-gateway contracts passed, live test explicitly ignored | Concurrency, cleanup retries and collision ownership; does not establish real 20 × 5 stress. |

## Historical open-work list (superseded by the current checkpoint)

The following list records the earlier audit state. It is not the current outstanding-work
verdict; completed image, M2, burst, fault and sync evidence is recorded above.

- P2-X-1: rebuild and verify the final image containing both the real Rust toolchain and
  current guest `sync-cas`; record its actual local image ID, base digest and blueprint.
  A preliminary image built, but it is not final evidence for the current guest source.
- OpenShell live adapter: byte transport was repaired. Intermediate image now reaches real Cargo;
  `openshell-live-quota-intermediate-retry.txt` passed with real Docker CPU/memory limits. Final
  batch-helper image, deletion absence and full stress evidence remain required.
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
