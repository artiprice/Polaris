# Polaris — Phase status

> **Document:** `@prime/PHASES_STATUS.md` · **Snapshot:** 2026-10-21 (P1 core-runtime implementation) · **Branch:** `main`
> (the team's periodic commits picked up part of the P0 work; this session committed nothing) ·
> **Reference PC:** Windows 11 LTSC 2024, Ryzen 7 2700, 16 GB RAM, GTX 1650 4 GB, rustc/cargo 1.99.0,
> WSL2 Ubuntu 24.04.5.
>
> Rules: `PASS` requires the command, its raw output and the conditions (Evidence log §E, or a spike's
> `RESULT.md` + raw file). `BLOCKED-ON-ACCESS` = needs a key/account that does not exist yet (D9). A doc-only
> skeleton is **not** an implementation. Hand-edited except `<!-- generated: … -->` blocks: crate rows come from
> the `status`/`evidence` keys in `@prime/tools/workspace.toml`; the summary counts every status row in this file.
> Refresh with `python -I @prime/tools/workspace.py render-docs`.

## Summary

<!-- generated: status summary -->
| Status | Rows |
|---|---|
| PASS | 27 |
| FAIL | 3 |
| IN PROGRESS | 22 |
| BLOCKED-ON-ACCESS | 15 |
| NOT STARTED | 97 |
| **total** | 164 |

**Current phase: P0 → P1.** P0 exit gate (PHASES.md P0) is **not fully met**: S0-10 needs a manual key test
(human at the keyboard), P0-ACC-1..4 need the team's account requests, and the Slint licence choice
(ADR-014, part of P0-ENV-7) is open. P1's entry condition (all P0-SKEL PASS) is met, so keyless P1 work
proceeds in parallel. **Current user direction: defer UI and implement the core logic.** Supervisor task
supervision and reaper teardown now have Windows/Linux evidence (§E22–§E23), and durable kernel sessions
have real-storage recovery evidence (§E25) on the schema-2 lifecycle contract (§E24). These crates are not fully PASS
because native worker/process and larger verification tasks remain. The generated crate rows are the
status index; implementation does not imply the entire crate/phase gate is complete.

Next actions: (1) **team** — request keys/accounts (P0-ACC-1..4); (2) **each member** — run
`@prime/tools/setup/setup-windows.ps1` (PHASES.md §0.1) and the S0-10 probe in their terminals; (3) **team** —
Slint licence decision; (4) core runtime: verified command dispatch/operator pipeline, native worker/platform
integration, concrete backend choices, and complete startup/shutdown assembly. UI work is deferred.
The prior proto witness-gate violation was fixed by moving the unchanged wire-contract test to the
allowed verifier boundary (§E25); `check-witness` is green and its implementation is unchanged.

## P0 — tasks

| Task | Status | Evidence / notes |
|---|---|---|
| P0-SKEL-1 `.gitignore` anchored, Cargo.lock tracked | PASS | §E1; spike outputs (`bin/`, `out*/`, `research/*-src/`) added to the ignore list this session |
| P0-SKEL-2 `.gitattributes` | PASS | file present (LF default; CRLF for .ps1/.bat/.cmd) |
| P0-SKEL-3 `workspace.toml` (93 packages, 156 ext deps, 26 member-setup items) | PASS | §E2 |
| P0-SKEL-4 `workspace.py` tooling | PASS | §E2–§E8, §E13 (`gen` skips `implemented` crates; `render-docs`) |
| P0-SKEL-5 root `Cargo.toml` + `rust-toolchain.toml` generated | PASS | §E3 (`cargo metadata`: 93 packages) |
| P0-SKEL-6 doc-only skeleton compiles | PASS | §E4, re-run §E13 |
| P0-SKEL-7 member + layer checks | PASS | §E5 (Python) and §E9 (Rust xtask) |
| P0-SKEL-8 version audit + resolution probe | PASS | §E6 (156/156 OK; lockfile resolved — 1,699 packages; see probe history) |
| P0-SKEL-9 PLAN.md / PHASES.md / PHASES_STATUS.md; inventory 100 % covered | PASS | §E13 (`check-docs`, `render-docs --check`, `chatlog-inventory --check`) |
| P0-ACC-1 Token Factory API key | BLOCKED-ON-ACCESS | team action: request key |
| P0-ACC-2 Token Factory Sandboxes (Contree) access | BLOCKED-ON-ACCESS | team action |
| P0-ACC-3 Nebius AI Cloud quota | BLOCKED-ON-ACCESS | team action |
| P0-ACC-4 Tavily API key | BLOCKED-ON-ACCESS | team action |
| P0-ENV-1 Ubuntu 24.04 in WSL2 | PASS | §E10, §E12: Ubuntu 24.04.5, kernel 6.6.87.2, user `dev` (sudo, kvm, docker), systemd; `/dev/kvm` present (S0-2); no `.wslconfig` needed |
| P0-ENV-2 Docker Engine + OpenShell v0.1.4 gateway in WSL2 | PASS | §E10 `verify-openshell.sh` (create 1.5 s, exec 112 ms, exit code, byte-exact upload, delete); driver pin in `setup-wsl.sh`; idle shutdown (W14) needs the keepalive |
| P0-ENV-3 tooling + setup scripts | PASS | §E12 `setup-windows.ps1 -Check` (all required present), install mode idempotent; WSL2 toolchain line in §E10. **Fresh-machine install paths not exercised** (everything was already present) |
| P0-ENV-4 `.cargo/config.toml`, longpaths, PROTOC | PASS | §E12/§E13: `CMAKE_POLICY_VERSION_MINIMUM=3.5` + `xtask` alias committed; `core.longpaths=true`; protoc on PATH; heavy deps build (openshell-sdk S0-1, LanceDB S0-9, ort S0-4); short target-dir not needed |
| P0-ENV-5 Linux check leg in WSL2 | PASS | §E10 `verify-linux-leg.sh` (check + doc `-Dwarnings`, fmt, 93 packages) + xtask tests on Linux |
| P0-ENV-6 local OpenAI-compatible server | PASS (endpoint) | §E10: Ollama 0.40.6 + `nemotron-mini`, SSE streaming OK (TTFT 2.3 s warm, 32.8 s cold). **Tool calling flaky: tool_calls in 1 of 4 runs** (the one success was in the previous session; its transcript is not in §E — the two runs in §E10 both show `tool_calls ABSENT`) — risk R22 for P3-X-1, evaluate a stronger small model when disk allows |
| P0-ENV-7 `deny.toml`, cargo-vet, Slint licence decision | IN PROGRESS | §E11: `cargo deny check` all ok; `cargo vet` OK with 5 audit imports and **29 exemptions (not audits)**; Slint licence choice open (team, ADR-014) |
| P0-XT-1 Rust xtask port of checks | PASS | §E9 (test-first: red → green; real tree OK; negative test FAILs as expected; Windows + WSL2) |

## P0 — spikes

| Spike | Status | Result / fallback chosen (details: `@prime/spikes/<dir>/RESULT.md`) |
|---|---|---|
| S0-1 openshell-sdk on windows-msvc + WSL2 gateway | PASS | builds without protoc; mTLS channel built by Polaris + `OpenShellClient::from_parts`; dial `localhost`; gateway request limit 1 MiB; ~3 s cold start → connect retry |
| S0-2 nested KVM + vm driver | PASS | `/dev/kvm` present; vm driver boots Ubuntu (guest kernel 6.12.76) in 27.5 s; exec 216 ms; one driver per gateway |
| S0-3 NemoClaw custom blueprint | FAIL | → **native blueprint applier** (`polaris-openshell-nemoclaw`); agents are in-tree only; blueprint pins OpenShell 0.0.116 |
| S0-4 GLiNER2.5-Decide ONNX parity + latency | PASS (conditional) | 260/260 = 100 % top-1 (fp32, max Δp 4e-6) **only with the trailing-period rule** (93.85 % without); Rust fp32 p50 363 ms / p95 481 ms (1 task, 4 threads), 2.9× faster than Python; RSS peak 2.35 GiB (= 2.52 GB, borderline vs 2.5 GB); fp16 worse on CPU; CUDA EP not measured |
| S0-5 Token Factory capabilities | BLOCKED-ON-ACCESS | needs P0-ACC-1 |
| S0-6 Needle 3 on Windows/WSL2 | PASS | Windows engine works: tool call p50 317 ms, embeddings 3,072-d p50 50 ms (Python/C only; `--serve` has no embed endpoint) |
| S0-7 workspace sync performance/fidelity | PASS | 10k files: no-op 1.1 s, initial 61 MiB in 10.4 s, 0 mismatches, CRLF + exec bit kept, deny-list holds; case-fidelity **not tested** |
| S0-8 iceoryx2 on Windows | PASS (condition) | RTT p50 2.0 µs / p99 4.7 µs; peer kill handled, dead-node cleanup OK; **security condition**: default root `C:\Temp\iceoryx2` is writable by all authenticated users → per-user `set_root_path` required |
| S0-9 LanceDB build on Windows | FAIL | build cost (36 min at -j4, 4.8 GB target, 249 MB exe; 0.40.0 needs `remote` feature + protoc) → **usearch + SQLite** default (verified: 1 min 41 s build, 4.3 MB, kNN p50 0.475 ms); LanceDB opt-in feature |
| S0-10 crossterm Shift+Enter on ConPTY | IN PROGRESS | probe built; **manual key test pending** (see RESULT.md for the steps) |
| S0-11 OpenShell mxc driver | FAIL | → **WSL2 tiers only** (no Windows gateway asset in v0.1.4; MXC "coming soon") |
| S0-12 guest build + handshake | PASS | guest built in WSL2, baked into image, handshake + mismatch refusal OK; headless Chromium screenshot OK (`--no-zygote`); Xvfb fails under default policy; `/opt` not readable/executable in sandboxes |
| S0-13 Contree MCP auth + branching | BLOCKED-ON-ACCESS | needs P0-ACC-2 |

## Cross-cutting phase tasks

| Task | Status |
|---|---|
| P1-X-1 composition-root wiring v0 | IN PROGRESS (§E25: durable session/store wiring; full backends and dispatcher remain open) |
| P1-X-2 TUI shell | NOT STARTED |
| P1-X-3 rustls aws-lc-rs provider in `main` | NOT STARTED |
| P2-X-1 Polaris sandbox image + blueprint | NOT STARTED |
| P2-X-2 `/onboarding` + `polaris doctor` | NOT STARTED |
| P3-X-1 walking skeleton (local endpoint) | NOT STARTED |
| P3-X-2 same loop on Nemotron | BLOCKED-ON-ACCESS |
| P3-X-3 Contree sidecar + E3 run | BLOCKED-ON-ACCESS |
| P3-X-4 live re-recording of SSE fixtures | BLOCKED-ON-ACCESS |
| P4-X-1 ONNX export script + goldens | NOT STARTED |
| P4-X-2 threshold calibration v1 | NOT STARTED |
| P4-X-3 MVP scripted demo | NOT STARTED |
| P5-X-1 SearXNG container + provider | NOT STARTED |
| P5-X-2 Tavily provider live | BLOCKED-ON-ACCESS |
| P5-X-3 NVIDIA docs sync | NOT STARTED |
| P5-X-4 embeddings via Token Factory | BLOCKED-ON-ACCESS |
| P6-X-1 SWE env resolver + branching | BLOCKED-ON-ACCESS |
| P6-X-2 remote OpenShell gateway (E4) | BLOCKED-ON-ACCESS |
| P6-X-3 Data Lab export + fine-tune trigger | BLOCKED-ON-ACCESS |
| P7-X-1 `/research` long run | NOT STARTED |
| P7-X-2 `/hack` on seeded-bug repos | NOT STARTED |
| P7-X-3 microVM browser loop | NOT STARTED |
| P9-X-1 static welcome page | NOT STARTED |
| P9-X-2 named-pipe transport + mTLS remote | NOT STARTED |
| P10-1 full test matrix | NOT STARTED |
| P10-2 security review | NOT STARTED |
| P10-3 performance budgets | NOT STARTED |
| P10-4 `[profile.dist]` submission build | NOT STARTED |
| P10-5 demo script + video | NOT STARTED |
| P10-6 Linux/macOS adaptation start | NOT STARTED |

## Milestones

| Milestone | Status |
|---|---|
| M0 skeleton compiles, layers enforced, versions audited, probe green | PASS (§E3–§E6, §E9) |
| M1 TUI shell with sessions + directives | NOT STARTED |
| M2 first sandboxed `cargo test` from the TUI | NOT STARTED |
| M3 walking skeleton (local endpoint) | NOT STARTED |
| M4 same loop on Nemotron + Contree | BLOCKED-ON-ACCESS |
| M5 MVP demo | NOT STARTED |
| M6 submission build + video | NOT STARTED |

## Incidents (this session)

- The previous session ended abruptly. Two files were left **zero-filled** by the torn write: the git index
  (`.git/index`, 34,333 bytes of zeros — `fatal: index file corrupt`) and an rlib in the shared spike target dir
  (`E0786 invalid metadata files for crate equivalent`). The index was rebuilt from HEAD with `git reset`
  (working tree untouched; the broken file is kept as `.git/index.corrupt-20261020`); the spike target dir was
  deleted and rebuilt.
- Around 23:42 the user cleared `%LOCALAPPDATA%\Temp`, which held the session scratchpad. No repository data was lost; evidence had already been embedded here, and the one draft (§E14) was rewritten from the session context. Temporary files now go to the git-ignored `@prime/tmp/`.
- The team's periodic commits recorded a 5 MB spike binary (`@prime/spikes/s0-1-openshell-sdk/bin/…exe`, commit
  6535a56) and a nested clone as a **gitlink without `.gitmodules`** (`@prime/spikes/s0-4-gliner/research/gliner2-rs-src`,
  commit 2b6b1ff) — the latter breaks teammates' checkouts. Both paths are now ignored; untracking them
  (`git rm --cached`) is left to the team.

## Crate tasks (generated)

Status and evidence per crate come from `status`/`evidence` in `@prime/tools/workspace.toml`. Some
IN PROGRESS crates have implemented behavior and passing suites; others still have only test-first
descriptors/tests. Read each evidence cell rather than treating either state as a completed crate.
Raw evidence for the core continuations is in §E21–§E27; prior crate evidence remains in §E14–§E17.
Earlier metadata also references an §E18 entry for log/telemetry/cache/automata/RTK that is absent
from this snapshot. Those prior claims were not re-verified in this continuation.

### P0

<!-- generated: P0 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-tests` | `polaris-tests` | P0 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:xtask` | `xtask` | P0 | PASS | P0-XT-1: `check-members` + `check-layers` in Rust, test-first (11 tests, red then green; Windows + WSL2), real repo OK (§E9) |

### P1

<!-- generated: P1 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-cli` | `polaris-cli` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-sdk` | `polaris-sdk` | P1 | PASS | Prior SDK contracts §E14; schema-2 session lifecycle extension §E24 adds 5 tests/properties, version-preserving envelopes and rejection of mislabeled new events. Existing v1 golden retained; current SDK Miri passed (existing insta snapshot skip) and Kani 3/3 harnesses verified. Witness gate unchanged. |
| `crate:polaris-proto` | `polaris-proto` | P1 | IN PROGRESS | delegated (subagent), test-first; evidence pending |
| `crate:polaris-config` | `polaris-config` | P1 | PASS | 5 descriptors; 29 integration tests + 1 doctest, 27 observed red before the code (long-path and highest-layer proptest were green on first run: characterization; golden snapshot reviewed by eye) (merge, provenance, trust filter, secret refs, discovery, env, session/D11, BOM/CRLF, Windows locked-file retry, long path, insta golden provenance); 2 planted bugs caught; fuzz config_toml_layer 64,562 runs clean; empty-machine load p50 0.29 ms (§E16) |
| `crate:polaris-log` | `polaris-log` | P1 | IN PROGRESS | 9 descriptors (subagent; re-verified): 63 tests (ring sink, drain queue with drop counter, redaction layer, JSON/text line format with injection escaping, filter resolution, size rotation, init lifecycle in a child process, 2 dhat zero-alloc tests: ring push and log enqueue); 3 planted bugs caught (§E18). Open: daily+size rotation, panic/Ctrl+C flush with the reaper, fuzz, 1M-record load bench |
| `crate:polaris-telemetry` | `polaris-telemetry` | P1 | IN PROGRESS | 6 descriptors (subagent; re-verified): 48 tests (usage fold, mergeable aggregates, integer micro-USD cost, budget monitor incl. exact-limit boundaries, arc-swap snapshots with 4 concurrent readers, HUD); 3 planted bugs caught after a proptest was strengthened (§E18). Open: bench for 1M events/s, TTFT/context-load once events carry them, replay idempotence |
| `crate:polaris-code-intel` | `polaris-code-intel` | P1 | IN PROGRESS | §E26: observed RED adversarial fingerprint cost and adjacent-declaration false conflict corrected without weakening assertions. Tracked depth matches native cursor across five grammars; 20k fingerprint digest preserved. Current Windows/Linux suites pass; full PLAN category gates remain open. |
| `crate:polaris-storage-cas` | `polaris-storage-cas` | P1 | IN PROGRESS | 5 descriptors; red then green: 31 tests incl. real Windows share-mode-0 retry and dhat zero-alloc; fuzz cas_blob_decode 3.96M runs clean (§E14). Open: Kani fold_ref, shortref fuzz, load/stress |
| `crate:polaris-storage-eventlog` | `polaris-storage-eventlog` | P1 | IN PROGRESS | 5 descriptors; red then green: 24 tests (append/read, replay, checkpoints, lock-table recovery, L2 exact cache, codec); JSON codec (postcard cannot decode the tagged Event, verified); fuzz eventlog_decode 309,107 runs clean (§E15). Open: load/stress, invariants + research tables (later phases), lazy replay iterator |
| `crate:polaris-storage-registry` | `polaris-storage-registry` | P1 | IN PROGRESS | 6 descriptors (subagent; re-verified): 47 tests incl. FTS5 trigram on bundled SQLite 3.53.2, masked-only indexing scan of db/-wal/-shm, migration rollback; 2 planted bugs caught (§E17). Open: fuzz (fts query, migration open), load/scalability, two-process stress |
| `crate:polaris-storage-cache` | `polaris-storage-cache` | P1 | IN PROGRESS | 4 descriptors; red then green: 25 tests (scope-isolated blake3 keys, L1->L2 promotion, write-through, TTL with fake clock, single-flight: 16 callers run once, leader cancel, errors not cached, Send future); 2 planted bugs caught (§E18). Open: L3 semantic tier (P5), loom model, load/stress, real-L2 integration test in polaris-tests |
| `crate:polaris-storage` | `polaris-storage` | P1 | IN PROGRESS | Storage child modules re-exported for the kernel composition root (§E25); child crates still have open tasks, so the facade gate is not PASS. |
| `crate:polaris-kernel-bus` | `polaris-kernel-bus` | P1 | IN PROGRESS | 4 descriptors; red then green: 13 tests incl. dhat zero-alloc publish/consume; fan-out-before-append mutation caught by 6 tests; Miri 13 tests clean (§E15). Open: loom (tokio not instrumentable), fuzz_bus_ops, load/stress |
| `crate:polaris-kernel-directives` | `polaris-kernel-directives` | P1 | IN PROGRESS | 6 descriptors; red then green: 39 tests incl. proptest (5,000-case roundtrip) and 2 caught mutations; Miri 39 tests clean; Kani 2/2 cursor harnesses; fuzz directives_grammar 137k runs clean (§E14). Open: load test, fixture corpus |
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | P1 | IN PROGRESS | 40 tests Windows + Linux; 2,000-case properties on Windows; task panic isolation/restart/backoff, bounded DLQ, circuit probes, durable bus events, joined shutdown (§E22). Open: native workers/health probes, Kani, load/stress. Two test contracts corrected with explicit user approval. |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | P1 | IN PROGRESS | 41 tests Windows / 40 Linux; 1,000-case outcome property on Windows; cancellation, ordered bounded shutdown, escaped atomic journal, orphan review, real temp-dir cleanup incl. Windows held file (§E23). Open: real process trees/kill-on-parent-death, start-time identity, backend orphan reconciliation, large stress. |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | P1 | IN PROGRESS | 10 descriptors (subagent; re-verified): 100 tests incl. a real cargo build round trip (JSON and text open the same issue id), real share-mode-0 lock retry; planted bugs caught (11 failures); home-ceiling bug found and fixed test-first; fuzz rtk_stream 112,824 runs clean (§E18). Open: throughput benches, rustc-version fixture corpus, cross-process stress, CLI entry |
| `crate:polaris-tools` | `polaris-tools` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel` | `polaris-kernel` | P1 | IN PROGRESS | Durable session context/typestate using real CAS/redb/SQLite, ordered replay and projection failure recovery (§E25). Full verified command orchestrator, backend choices, native workers, signals and full startup/shutdown assembly remain open. |
| `crate:polaris-ui-model` | `polaris-ui-model` | P1 | IN PROGRESS | 111 Windows tests incl. 1,000-case properties, 5 initial snapshots reviewed, Unicode chip regression saved; Linux check; ui_completion fuzz 207,648 runs/61 s (§E21). UI deferred by user: remaining settings/provenance, full HUD, metadata arity, Quick Chips, fixtures/Miri and TUI integration. |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui` | `polaris-ui` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P2

<!-- generated: P2 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | P2 | IN PROGRESS | main session, test-first; evidence pending |
| `crate:polaris-security-automata` | `polaris-security-automata` | P2 | IN PROGRESS | 5 descriptors; red then green: 27 tests (19-rule default pack with a positive+negative per rule, scan == regex-crate reference proptest, user-regex guard, integer fixed-point entropy, redactor idempotence, dhat zero-alloc scan/any_match/redact_into); 3 planted bugs caught (one after tightening a too-loose assertion) (§E18). Open: fuzz targets, Miri, load/scalability tables, bypass corpus, chunk-boundary policy |
| `crate:polaris-platform-wsl` | `polaris-platform-wsl` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-platform-process` | `polaris-platform-process` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-platform` | `polaris-platform` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-vcs` | `polaris-vcs` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-openshell-client` | `polaris-openshell-client` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-openshell-policy` | `polaris-openshell-policy` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-openshell-nemoclaw` | `polaris-openshell-nemoclaw` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-openshell` | `polaris-openshell` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security` | `polaris-security` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-leases` | `polaris-runtime-leases` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-guards` | `polaris-runtime-guards` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime` | `polaris-runtime` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-sandbox` | `polaris-spike-sandbox` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P3

<!-- generated: P3 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-ratelimit` | `polaris-ratelimit` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-capability` | `polaris-security-capability` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-secrets` | `polaris-security-secrets` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud-token-factory` | `polaris-nebius-cloud-token-factory` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud-sandboxes` | `polaris-nebius-cloud-sandboxes` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud` | `polaris-nebius-cloud` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-models` | `polaris-models` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-tools-math` | `polaris-tools-math` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-frames` | `polaris-runtime-frames` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-context` | `polaris-runtime-context` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-operators` | `polaris-kernel-operators` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | P3 | IN PROGRESS | Manifest protected for the unchanged verified-command wire contract test relocated into the allowed witness boundary (§E25). Production verifier remains a documentation skeleton; authorization/AST/S1 implementation not started. |
| `crate:polaris-spike-ratelimit` | `polaris-spike-ratelimit` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P4

<!-- generated: P4 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-worker` | `polaris-worker` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-engine` | `polaris-system-one-engine` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-client` | `polaris-system-one-client` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-pii` | `polaris-security-pii` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-rules` | `polaris-security-rules` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-schemas` | `polaris-system-one-schemas` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-router` | `polaris-system-one-router` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-verifier` | `polaris-system-one-verifier` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-watchdog` | `polaris-system-one-watchdog` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one` | `polaris-system-one` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-s1` | `polaris-spike-s1` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P5

<!-- generated: P5 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-vector` | `polaris-storage-vector` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ingest` | `polaris-ingest` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-search-providers` | `polaris-search-providers` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-search-sift` | `polaris-search-sift` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-search` | `polaris-search` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nvidia-docs` | `polaris-nvidia-docs` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P6

<!-- generated: P6 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-nebius-cloud-data-lab` | `polaris-nebius-cloud-data-lab` | P6 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud-ai-cloud` | `polaris-nebius-cloud-ai-cloud` | P6 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P7

<!-- generated: P7 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-guest` | `polaris-guest` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ext-mcp` | `polaris-ext-mcp` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ext-skills` | `polaris-ext-skills` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ext` | `polaris-ext` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-browser` | `polaris-browser` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-hacker` | `polaris-hacker` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-dag` | `polaris-runtime-dag` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-research` | `polaris-runtime-research` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P8

<!-- generated: P8 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-devenv-nix` | `polaris-devenv-nix` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-devenv-docker` | `polaris-devenv-docker` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-devenv-cicd` | `polaris-devenv-cicd` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-devenv` | `polaris-devenv` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-nano` | `polaris-system-one-nano` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P9

<!-- generated: P9 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-desktop` | `polaris-desktop` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-web` | `polaris-web` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-daemon` | `polaris-daemon` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-rpc` | `polaris-rpc` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui-gui` | `polaris-ui-gui` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui-web` | `polaris-ui-web` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-web` | `polaris-spike-web` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

## Probe history (dependency resolution)

1. First `workspace.py probe` run **FAILED**: `oar-ocr-core v0.10.0` requires `ort =2.0.0-rc.13`, conflicting
   with the initial pin `ort 2.0.0-rc.15` (`ort-sys` declares `links`, one version per graph).
   Fix: pin `ort = "=2.0.0-rc.13"` (has `load-dynamic` and `api-23`; verified on crates.io).
2. Second run: resolved — "Locking 1691 packages to highest Rust 1.99 compatible versions".
3. Duplicate review: own `rten 0.27` duplicated `ocrs`'s `rten 0.26` → aligned to `0.26.0`; third run resolved.
   Remaining duplicates are third-party (PLAN §10). `cargo tree -i ring` is non-empty and explained (ADR-005).
4. Added `usearch 2.26.5` (S0-9 fallback) and `ast-grep-config 0.51.0` (rule packs): resolved — "Locking 1696 packages".
5. Added `dhat 0.3.3` (zero-alloc tests, ADR-003): resolved — "Locking 1699 packages".

## §E Evidence log (raw output, this snapshot)

Conditions: Windows 11 LTSC 2024 (10.0.26100), rustc 1.99.0 (b940084d7 2026-09-28), cargo 1.99.0
(5f94df478 2026-08-27), Python 3.14.5, Git Bash. Paths printed by `git check-ignore -v` for the first
command are **negation matches** (`!` patterns), i.e. *not ignored*; paths without a line are not matched
at all (also not ignored).

```text
### E1 — .gitignore behaviour
$ git check-ignore -v crates/a/src/bin/x.rs crates/a/src/sessions/x.rs crates/polaris-storage/crates/cache/src/lib.rs tests/fixtures/a.log Cargo.lock
.gitignore:71:!/tests/fixtures/**/*.log	tests/fixtures/a.log
.gitignore:146:!/Cargo.lock	Cargo.lock
[exit 0]
$ git check-ignore -v @prime/chatlog.json @prime/initial.md @prime/prompt.md .agents/skills/README.md @prime/tools/dep-probe/Cargo.toml
.gitignore:43:**/chatlog.json	@prime/chatlog.json
.gitignore:45:**/initial.md	@prime/initial.md
.gitignore:29:*/prompt.md	@prime/prompt.md
.gitignore:51:/.agents/	.agents/skills/README.md
.gitignore:35:/@prime/tools/dep-probe/	@prime/tools/dep-probe/Cargo.toml
[exit 0]
$ git status --porcelain --ignored -- apps crates tests xtask @prime/tools Cargo.toml Cargo.lock rust-toolchain.toml .gitattributes .gitignore | grep "^!!"
!! @prime/tools/dep-probe/
### E2 — crate table
$ python -I @prime/tools/workspace.py summary
packages=93 top-level-crates=27 nested=53 external-deps=156
kinds: {'app': 6, 'facade': 13, 'lib': 67, 'spike': 5, 'test': 1, 'xtask': 1}
layers: {0: 1, 1: 18, 2: 28, 3: 17, 4: 9, 5: 2, 6: 5, 7: 6, 8: 7}
phases: {'P0': 2, 'P1': 24, 'P2': 15, 'P3': 13, 'P4': 11, 'P5': 6, 'P6': 2, 'P7': 8, 'P8': 5, 'P9': 7}
[exit 0]
### E3 — manifests
$ cargo metadata --format-version 1 --no-deps  (package count)
packages: 93 workspace_members: 93
### E4 — build gates
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized] target(s) in 0.19s
[exit 0]
$ RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
    Finished `dev` profile [unoptimized] target(s) in 0.44s
   Generated C:\mDev\Polaris\target\doc\polaris_browser\index.html and 92 other files
[exit 0]
$ cargo fmt --all --check
[exit 0]
### E5 — structure
$ python -I @prime/tools/workspace.py check-members
check-members: 93 members matched, table has 93 -> OK
[exit 0]
$ python -I @prime/tools/workspace.py check-layers
check-layers: 93 packages, 252 edges -> OK
[exit 0]
$ cargo tree -p polaris-kernel -e normal --prefix none | grep -cE "polaris-(ingest|system-one-engine|system-one-nano) "
0
### E6 — versions & resolution
$ python -I @prime/tools/workspace.py audit-versions | tail -1
audit-versions: 156 dependencies checked -> OK
[exit 0]
$ python -I @prime/tools/workspace.py probe  (Locking line)
     Locking 1699 packages to highest Rust 1.99 compatible versions
[exit 0]
### E7 — toolchain
$ rustc --version; cargo --version
rustc 1.99.0 (b940084d7 2026-09-28)
cargo 1.99.0 (5f94df478 2026-08-27)
### E8 — document consistency
$ python -I @prime/tools/workspace.py check-docs
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes -> OK
[exit 0]
$ python -I @prime/tools/workspace.py chatlog-inventory --check @prime/PLAN.md | tail -1
all 396 items present in @prime/PLAN.md
[exit 0]
### E9 — P0-XT-1 xtask (test-first; descriptors specs/descriptors/xtask_check_*.json)
$ cargo test -p xtask --no-fail-fast --test check_members      (stub implementation → red)
test glob_dir_without_manifest_is_reported ... FAILED
test all_table_packages_present_is_ok ... ok
test table_package_missing_on_disk_is_reported ... FAILED
test stray_file_at_glob_level_is_reported ... FAILED
test member_not_in_table_is_reported ... FAILED
test result: FAILED. 1 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out
$ cargo test -p xtask                                           (stub → red, check_layers)
test result: FAILED. 2 passed; 4 failed; 0 ignored; 0 measured; 0 filtered out
$ cargo test -p xtask --no-fail-fast                            (implementation → green)
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (check_layers)
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out   (check_members)
$ cargo xtask check-members; cargo xtask check-layers
check-members: 93 packages -> OK
check-layers: 93 packages, 252 edges -> OK
$ (negative test on the real tree: crates/STRAY.txt + empty crates/polaris-ghost/) target\debug\xtask.exe check-members
FAIL StrayFile("crates/STRAY.txt")
FAIL MissingManifest("crates/polaris-ghost")
check-members: 93 packages -> FAILED
[exit 1]   (through `cargo xtask`, cargo itself rejects the manifest-less member first: exit 101
            "failed to load manifest for workspace member … referenced via crates/*")
### E10 — environment verification (WSL2, OpenShell, Linux leg, local LLM), 2026-10-20 ~22:05
### verify-openshell.sh 2026-10-20T22:05:55
== openshell openshell 0.1.4 / gateway: active (ready after 97 ms)
Server Status
  Status: Connected
  Authentication: Authenticated (mTLS transport)
  Version: 0.1.4
create: ok (1474 ms)
phase: Ready
Linux 6.6.87.2-microsoft-standard-WSL2
uid=1000(ubuntu) gid=1000(ubuntu) groups=1000(ubuntu)
PRETTY_NAME="Ubuntu 24.04.5 LTS"
landlock_abi=n/a
pwd=/sandbox
exec: 112 ms
exit-code propagation: ok (7)
upload byte-exact: ok (3287856bc0a394cc1a63ac81500d3a888909f883512f308ae90dfda56d78e7a1)
delete: ok
RESULT: PASS
exit=0
### verify-linux-leg.sh
toolchain: rustc 1.99.0 (b940084d7 2026-09-28) | nightly: rustc 1.101.0-nightly (07e526db9 2026-10-19) | nextest: cargo-nextest 0.9.148 (cd1d6d546 2026-10-07) | fuzz: cargo-fuzz 0.13.2 | kani: Kani Rust Verifier 0.68.0 (cargo plugin)
    Checking xtask v0.1.0-alpha (/mnt/c/mDev/Polaris/xtask)
    Finished `dev` profile [unoptimized] target(s) in 14.38s
   Generated /home/dev/.cache/polaris-target/doc/polaris_browser/index.html and 92 other files
fmt: ok
packages: 93
RESULT: PASS
exit=0
### xtask on Linux (WSL2)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
check-members: 93 packages -> OK
check-layers: 93 packages, 252 edges -> OK
### verify-local-llm.py
models: ['nemotron-mini:latest']
stream: chunks=15 ttft_ms=32824 total_ms=33205 text="Sure, I'm ready! How can I assist you today?"
tool_call: ms=3448 calls=[] content=' '
tool_call: model returned no tool_calls (model capability, not a client failure)
RESULT: PASS
exit=0
### verify-local-llm.py --require-tools (warm, after script fix)
models: ['nemotron-mini:latest']
stream: chunks=6 ttft_ms=2266 total_ms=2504 text="I'm ready!"
tool_call: ms=3004 calls=[] content=' '
tool_call: model returned no tool_calls (model capability, not a client failure)
RESULT: endpoint PASS; tool_calls ABSENT
exit=1
### E11 — supply chain
$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
$ cargo vet import mozilla|google|bytecode-alliance|zcash|isrg   → 25 unvetted remain (imports cover older versions)
$ cargo vet regenerate exemptions; cargo vet
Vetting Succeeded (5 fully audited, 1 partially audited, 29 exempted)
### E12 — per-PC setup script (PHASES.md §0.1)
$ powershell -NoProfile -ExecutionPolicy Bypass -File @prime\tools\setup\setup-windows.ps1 -Check
== 1/8 Windows prerequisites
  [ok     ] windows-build          build 26100 (WSL2 needs >= 19045)
  [ok     ] LongPathsEnabled       1
  [ok     ] msvc-build-tools       VS 18.11.12306.390 (VC tools x64)
== 2/8 winget packages
  [ok     ] git                    git version 2.52.0.windows.1
  [ok     ] rustup                 rustup 1.29.1 (d95a37b6a 2026-08-13)
  [ok     ] python                 Python 3.14.5
  [ok     ] cmake                  cmake version 4.3.0-rc3
  [ok     ] nasm                   NASM version 3.01 compiled on Dec  3 2025
  [ok     ] ninja                  1.13.2
  [ok     ] protoc                 libprotoc 36.0
== 3/8 Rust 1.99.0
  [ok     ] rust-1.99.0            rustc 1.99.0 (b940084d7 2026-09-28)
== 4/8 cargo tools
  [ok     ] cargo-binstall         1.26.0
  [ok     ] cargo-nextest          cargo-nextest 0.9.148 (cd1d6d546 2026-10-07) (want 0.9.148)
  [ok     ] cargo-mutants          cargo-mutants 27.1.0 (want 27.1.0)
  [ok     ] cargo-leptos           cargo-leptos 0.3.11 (want 0.3.11)
  [ok     ] cargo-deny             cargo-deny 0.20.2 (want 0.20.2)
  [ok     ] cargo-vet              cargo-vet 0.10.2 (want 0.10.2)
  [ok     ] cargo-audit            cargo-audit-audit 0.22.2 (want 0.22.2)
== 5/8 uv 0.13.3 (python -m uv)
  [ok     ] uv                     uv 0.13.3 (502f93ca6 2026-10-19 x86_64-pc-windows-msvc)
== 6/8 repository-local git config
  [ok     ] git core.longpaths     true
== 7/8 Ollama + nemotron-mini (P3 local endpoint)
  [ok     ] ollama                 ollama version is 0.40.6
  [ok     ] nemotron-mini          pulled
== 8/8 WSL2 Ubuntu-24.04 + OpenShell (via setup-wsl.sh)
  [ok     ] Ubuntu-24.04           installed
  [ok     ] wsl user dev           default user with sudo (see script comment)
  [ok     ] openshell (wsl)        openshell 0.1.4
  [ok     ] openshell-gateway      active
  [info   ] wsl keepalive          optional: re-run with -RegisterKeepalive, or keep a wsl session open
RESULT: all required components present
exit=0
$ (install mode, same machine) … RESULT: all required components present   [exit 0, 20 s; runs setup-wsl.sh inside WSL2:
  "gateway: active; driver pin: 1"]
### E13 — document and build gates after the P0 updates
$ python -I @prime/tools/workspace.py check-members
check-members: 93 members matched, table has 93 -> OK
[exit 0]
$ python -I @prime/tools/workspace.py check-layers
check-layers: 93 packages, 252 edges -> OK
[exit 0]
$ python -I @prime/tools/workspace.py check-docs
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
[exit 0]
$ python -I @prime/tools/workspace.py render-docs --check
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: up to date (10 generated blocks)
[exit 0]
$ python -I @prime/tools/workspace.py chatlog-inventory --check @prime/PLAN.md | tail -1
all 396 items present in @prime/PLAN.md
[exit 0]
$ python -I @prime/tools/workspace.py audit-versions | tail -1
audit-versions: 156 dependencies checked -> OK
[exit 0]
$ python -I @prime/tools/workspace.py probe (Locking line)
     Locking 1701 packages to highest Rust 1.99 compatible versions
[exit 0]
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized] target(s) in 5.71s
[exit 0]
$ RUSTDOCFLAGS=-Dwarnings cargo doc --workspace --no-deps
    Finished `dev` profile [unoptimized] target(s) in 11.02s
   Generated C:\mDev\Polaris\target\doc\polaris_browser\index.html and 92 other files
[exit 0]
$ cargo fmt --all --check
[exit 0]
$ cargo test -p xtask (result lines)
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
[exit 0]
$ cargo xtask check-members; cargo xtask check-layers
check-members: 93 packages -> OK
check-layers: 93 packages, 252 edges -> OK
[exit 0]
$ cargo deny check
advisories ok, bans ok, licenses ok, sources ok
[exit 0]
$ cargo vet
Vetting Succeeded (5 fully audited, 1 partially audited, 29 exempted)
[exit 0]
$ git check-ignore -v (spike outputs)
.gitignore:43:/@prime/spikes/**/out*/	@prime/spikes/s0-12-guest/out/x.png
.gitignore:43:/@prime/spikes/**/out*/	@prime/spikes/s0-12-guest/out-debug/e.png
.gitignore:42:/@prime/spikes/**/bin/	@prime/spikes/s0-10-crossterm/bin/x.exe
.gitignore:31:**/target/	@prime/spikes/target/x
[exit 0]
$ git check-ignore (must NOT be ignored)
(no output above = none ignored)
[exit 1]
### E14 — P1 crates implemented test-first (polaris-sdk, polaris-kernel-directives, polaris-storage-cas) + xtask check-witness
$ cargo test -p polaris-sdk            (before implementation: red)
error[E0432]: unresolved import … (12 occurrences; every test target failed to compile)
$ cargo test -p polaris-sdk --no-fail-fast   (green; Windows)
test result: ok. 3 passed (command_typestate) · 6 passed (content_identifiers) · 6 passed (effort_parsing)
test result: ok. 4 passed (event_envelope) · 5 passed (probability_validation) · 4 passed (registry_dispatch)
Doc-tests: 3 passed + 3 "compile fail ... ok" (handler_requires_verified_command, verified_command_cannot_be_deserialized, fast_fingerprint_has_no_cid_conversion)
$ cargo test -p polaris-kernel-directives   (before implementation: red — unresolved imports in all 6 test targets)
$ cargo test -p polaris-kernel-directives --no-fail-fast   (green; Windows)
test result: ok. 4 passed (directive_persistence) · 2 passed (directive_rendering) · 13 passed (input_parser)
test result: ok. 6 passed (mention_parser) · 7 passed (slash_command_table) · 7 passed (subagent_model_ranges)
$ PROPTEST_CASES=5000 cargo test -p polaris-kernel-directives --test directive_rendering --test input_parser
test prop_render_roundtrip ... ok · prop_parse_never_panics_and_spans_are_valid ... ok · prop_text_without_markers_is_prose ... ok
$ (mutation check 1: render.rs no longer escapes '"')   → render_canonical_forms FAILED, prop_render_roundtrip FAILED
$ (mutation check 2: mention.rs keeps a trailing '.')   → mention_stops_at_closing_bracket_and_trailing_punctuation FAILED, email_is_not_a_mention FAILED
  (restored files had to be touched: a restore with the old mtime leaves cargo linking the mutated build — PLAN W21)
$ cargo test -p polaris-storage-cas         (before implementation: red — unresolved imports in 5 test targets)
$ cargo test -p polaris-storage-cas --no-fail-fast   (green; Windows)
test result: ok. 6 passed (atomic_write_with_retry, incl. windows_held_handle_is_retried: a single attempt FAILS while a
             share-mode-0 handle is held, the retry policy then succeeds) · 10 passed (blob_store_round_trip)
test result: ok. 7 passed (decompression_bound) · 6 passed (short_reference_folding) · 1 passed (zero_alloc_hot_path, dhat: 0 blocks)
$ cargo test -p xtask --test check_witness  (stub: red — other_crates_are_reported_with_line FAILED; then green: 4 passed)
$ cargo xtask check-witness → check-witness: 0 violation(s) -> OK
$ (planted ApprovalWitness::new in polaris-kernel-operators) cargo xtask check-witness
FAIL ApprovalWitness::new outside sdk/verifier at crates/polaris-kernel/crates/operators/src/witness_probe.rs:1
check-witness: 1 violation(s) -> FAILED   [exit 1]   (file removed afterwards → 0 violation(s) -> OK)
$ cargo vet regenerate exemptions; cargo vet
Vetting Succeeded (25 fully audited, 10 partially audited, 95 exempted)    (exemption entries 30 → 105: audit backlog)
$ wsl -- bash @prime/tools/setup/kani-guarded.sh polaris-kernel-directives
Checking harness kani_proofs::cursor_take_while_stays_on_char_boundaries... VERIFICATION:- SUCCESSFUL (22.1 s)
Checking harness kani_proofs::cursor_bump_stays_on_char_boundaries...       VERIFICATION:- SUCCESSFUL (25.0 s)
Complete - 2 successfully verified harnesses, 0 failures, 2 total.   kani polaris-kernel-directives exit=0 wall_s=66
$ (earlier run) cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu <target> -- -max_total_time=60
directives_grammar: #137091 DONE cov: 1069 ft: 3931 corp: 921/18Kb … Done 137091 runs in 61 second(s)   exit=0
cas_blob_decode:    #3962414 DONE cov: 131 ft: 142 corp: 12/225b … Done 3962414 runs in 61 second(s)  exit=0
Kani attempts that were stopped (memory, PLAN W24): whole-parse harness (4 bytes: CBMC 3.9 GB / 11 min; 3 bytes: 3.4 GB / 7 min,
reported FAILED after the stop — not a verified counterexample), CAS fold_ref harness (CBMC 6.1 GB, 80 MB free in WSL) → removed.
### E15 — polaris-kernel-bus + polaris-storage-eventlog (test-first) and proof run 2 (Miri incl. bus, 3 fuzz targets)
$ cargo test -p polaris-kernel-bus      (before implementation: red; grouped output, crate names elided by the grouping)
1 error: could not compile `_` (test "command_backpressure") due to 1 previous error
1 error: could not compile `_` (test "lag_recovery") due to 2 previous errors; 2 warnings emitted
1 error: could not compile `_` (test "log_then_publish") due to 1 previous error
1 error: could not compile `_` (test "zero_alloc_hot_path") due to 1 previous error
$ cargo test -p polaris-kernel-bus      (green; Windows, re-counted 2026-10-21) → passed=13 failed=0
$ (mutation: fan-out before append in bus.rs publish)
test lagged_reports_missed_and_resume_seq ... FAILED · test resume_from_store_replays_missed_events ... FAILED
test store_error_suppresses_fanout ... FAILED · test publish_appends_before_fanout ... FAILED · test seq_is_strictly_increasing ... FAILED
test hot_path_does_not_allocate_after_warmup ... FAILED          (6 tests caught it; file restored and touched — PLAN W21)
$ cargo test -p polaris-storage-eventlog   (before implementation: red; cargo stopped after these 4 targets)
1 error: could not compile `_` (test "event_append_and_read") due to 2 previous errors
1 error: could not compile `_` (test "event_codec") due to 2 previous errors
1 error: could not compile `_` (test "exact_cache_store") due to 1 previous error
1 error: could not compile `_` (test "lock_table_recovery") due to 1 previous error
$ cargo test -p polaris-storage-eventlog   (green; Windows, re-counted 2026-10-21) → passed=24 failed=0
$ (probe @prime/tmp/p1/postcard-probe) postcard decode of sdk's internally tagged Event → Err(WontImplement)   → JSON codec
$ wsl -d Ubuntu-24.04 -- bash @prime/tools/setup/verify-proofs.sh 60      (2026-10-21)
== miri: polaris-sdk                 test result: ok. 0/3/6/6/3 (1 filtered: insta snapshot)/5/5 passed   miri polaris-sdk exit=0
== miri: polaris-kernel-directives   test result: ok. 0/4/2/13/6/7/7 passed                             miri polaris-kernel-directives exit=0
== miri: polaris-kernel-bus          test result: ok. 0/5/3/4/1 passed (one target 254.02s under Miri)    miri polaris-kernel-bus exit=0
== kani: polaris-sdk                 Complete - 2 successfully verified harnesses, 0 failures, 2 total.  exit=0 wall_s=19
== kani: polaris-kernel-directives   Complete - 2 successfully verified harnesses, 0 failures, 2 total.  exit=0 wall_s=66
== fuzz: directives_grammar (60s)    Done 76720 runs in 61 second(s)     fuzz directives_grammar exit=0
== fuzz: cas_blob_decode (60s)       Done 2649491 runs in 61 second(s)   fuzz cas_blob_decode exit=0
== fuzz: eventlog_decode (60s)       Done 309107 runs in 62 second(s)    fuzz eventlog_decode exit=0
RESULT: PASS                         (extract: @prime/tmp/p1/proofs-run2.txt)
### E16 — polaris-config (test-first)
$ cargo test -p polaris-config          (before implementation: red; grouped output)
1 error: could not compile `_` (test "config_layer_merge") due to 1 previous error
1 error: could not compile `_` (test "directory_discovery") due to 1 previous error
1 error: could not compile `_` (test "repo_layer_trust_filter") due to 1 previous error
1 error: could not compile `_` (test "secret_reference_parsing") due to 1 previous error
$ cargo test -p polaris-config          (first green attempt)
test no_repo_dir_is_none ... FAILED · test home_global_dir_is_not_a_repo_dir ... FAILED
  left: Some("C:\\Users\\Admin\\.polaris")  right: None
  → the upward walk left the injected home and found the developer's real ~/.polaris (the temp dir is under home).
    Fix (design): the walk stops at home, exclusive; test extended with a .polaris above home (red, then green).
$ (added test-first) windows_locked_file_is_retried ... FAILED   (then a bounded read retry on OS errors 32/33 → ok)
$ (characterization, green on first run) long_path_repo_dir_is_found ... ok · prop_highest_layer_wins_per_key ... ok
$ cargo test -p polaris-config          (green; Windows)
test result: ok. 9 passed (config_layer_merge) · 10 passed (directory_discovery) · 1 passed (golden_provenance)
test result: ok. 5 passed (repo_layer_trust_filter) · 4 passed (secret_reference_parsing) · Doc-tests: 1 passed
$ (golden) insta: stored new snapshot …golden_provenance__golden_provenance.snap.new → reviewed line by line → accepted
$ (mutation 1: layer precedence reversed in merge.rs)
test tables_merge_scalars_replace ... FAILED · test provenance_reports_winning_layer ... FAILED · test later_layer_overrides_earlier ... FAILED
$ (mutation 2: trust filter skipped inside load) first only repo_cannot_set_endpoint FAILED → test strengthened
  (repo_cannot_downgrade_sandbox_tier now also asserts load) → 2 FAILED; restored + touched → 5 passed
$ RUSTFLAGS=-Dwarnings cargo check -p polaris-config --all-targets   → Finished, exit=0
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-config --no-deps      → Generated …\polaris_config\index.html, exit=0
$ cargo fmt -p polaris-config --check → exit=0 · cargo xtask check-layers → check-layers: 93 packages, 251 edges -> OK
$ rg (unwrap/expect/panic!/todo!/unimplemented!/unreachable!/unsafe/#[allow/#![) crates/polaris-config/src → no matches
$ wsl … cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu config_toml_layer -- -max_total_time=60
#64562	DONE   cov: 2626 ft: 6341 corp: 1135/8143b lim: 17 exec/s: 1058 rss: 292Mb
Done 64562 runs in 61 second(s)      (no crash/panic lines: grep -c "crash-|panicked" → 0)
$ (smoke, release, throwaway bin @prime/tmp/p1/config-smoke) empty machine, discover+read_layers+load, n=200
discover+read_layers+load on empty machine: first=569us p50=287us p99=340us max=569us (n=200)     (target < 5 ms)
### E17 — polaris-storage-registry (delegated subagent, test-first; re-verified by the main session)
$ cargo test -p polaris-storage-registry   (subagent red, @prime/tmp/registry/red-summary.txt)
1 error: could not compile `polaris-storage-registry` (test "database_open") due to 1 previous error
  … same line for fts_query_sanitising, history_indexing, history_search, schema_migration, session_registry_crud
1 error[E0432]: unresolved imports `polaris_storage_registry::fts_phrase`, `polaris_storage_registry::like_pattern`   (+5 more E0432 lines)
$ cargo test -p polaris-storage-registry   (re-run by the main session, Windows)
test result: ok. 11 passed (database_open) · 5 passed (fts_query_sanitising) · 6 passed (history_indexing)
test result: ok. 11 passed (history_search) · 5 passed (schema_migration) · 9 passed (session_registry_crud)
$ (subagent mutations, as reported) fts_phrase without quote doubling → embedded_quotes_are_doubled, prop_phrase_quotes_are_balanced FAILED;
  ORDER BY … ASC → hits_are_newest_first, snippet_is_a_window_around_the_match FAILED; both restored + touched → green
$ RUSTFLAGS=-Dwarnings cargo check -p polaris-storage-registry --all-targets → Finished, exit=0 · cargo doc (-Dwarnings) → exit=0
$ cargo fmt -p polaris-storage-registry --check → exit=0 · forbidden-pattern rg (incl. `dyn `) on src → no matches
$ grep '#define SQLITE_VERSION ' libsqlite3-sys-0.38.2/sqlite3/sqlite3.h → "3.53.2"   (bundled; FTS5 trigram gate test passes)
$ cargo deny check → advisories ok, bans ok, licenses ok, sources ok   (warnings only: unmatched allow entries, duplicate hashbrown/syn)
$ cargo vet → non-zero exit; tail: "cargo vet inspect libsqlite3-sys 0.38.2 … rusqlite", "cargo vet inspect sqlite-wasm-rs 0.5.5 … rusqlite", "estimated audit backlog: 1620242 lines"
$ cargo vet regenerate exemptions; cargo vet
Vetting Succeeded (30 fully audited, 11 partially audited, 105 exempted)    (exemption entries 105 → 115: audit backlog, not audits)
```

### E21 — UI model continuation (deferred by user before TUI integration)

Windows host; repository-pinned toolchain. The existing tests/descriptors preceded implementation.
New completion/classification/composer/reducer imports were observed red before implementation.
Initial snapshots were absent; four were inspected in full, accepted without replacing existing
baselines, then the paused-goal snapshot was generated, inspected and accepted. These are synthetic
event scripts, not a live UI demonstration. The user subsequently directed all UI work to be deferred.

```text
$ cargo test -p polaris-ui-model --test completion_ranking --test token_classification (red)
error[E0432]: unresolved import `polaris_ui_model::classify`
error[E0432]: unresolved import `polaris_ui_model::completion`
$ cargo test -p polaris-ui-model --test composer_buffer --test prompt_history (red)
error[E0432]: unresolved import `polaris_ui_model::composer`
$ cargo test -p polaris-ui-model --test view_reducer (red)
error[E0432]: unresolved import `polaris_ui_model::reduce`
error[E0432]: unresolved import `polaris_ui_model::view`
$ cargo test -p polaris-ui-model --test composer_buffer --test prompt_history (first implementation)
Test failed: cursor inside a chip after Image(1)
minimal failing input: ops = [Insert("\u{2de0}"), Home, Image(1)]
  Saved seed 2763d158d84d73f085f0df638c81121ab5f56012bf82c2ca5db7431dda8b896d;
  fixed insertion endpoint after chip registration, retaining the unchanged property/regression.
$ PROPTEST_CASES=1000 cargo test -p polaris-ui-model --no-fail-fast (Windows; result lines)
test result: ok. 17 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.06s
test result: ok. 21 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.08s
test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 27 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.38s
$ RUSTFLAGS=-Dwarnings cargo check -p polaris-ui-model --all-targets
    Finished `dev` profile [unoptimized] target(s) in 2.08s
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-ui-model --no-deps
    Finished `dev` profile [unoptimized] target(s) in 4.41s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target cargo check -p polaris-ui-model --all-targets'
    Finished `dev` profile [unoptimized] target(s) in 16.32s
$ wsl … CARGO_TARGET_DIR=~/.cache/polaris-target-fuzz cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu ui_completion -- -max_total_time=60
Done 207648 runs in 61 second(s)
```

Open: full settings/provenance, HUD context capacity/burn-rate/capability display, metadata arity,
Quick Chips, external UI fixture files, Miri and TUI integration. No rendered UI or M1 claim.

### E22 — core task supervisor (native worker supervision still open)

Existing four descriptors/test files were observed red against the empty library. Implements task
panic isolation (including factory construction inside the spawned task), restart policies, sliding
intensity, backon exponential jitter, bounded FIFO DLQ, durable bus announcements, event-loss counters,
single-probe circuit recovery, escalation cancellation, and explicit shutdown that joins actors and
monitors. No native worker process/health adapter is claimed. Async actor cancellation is cooperative;
an arbitrary blocking actor or indefinitely stalled synchronous event-store call has no hard termination
guarantee. Real process isolation is still needed for those cases.

Two conflicting test contracts were flagged and **explicitly approved by the user for correction**:
the telemetry test now calls `publish()` before reading the last published snapshot; backoff still
rejects early retries and allows at most one 1 ms timer tick of lateness. Restart counts, failure-event
contents and the zero-alive-tasks shutdown assertion were retained. No gate was weakened or removed.

```text
$ cargo test -p polaris-kernel-supervisor --no-fail-fast (red)
error[E0432]: unresolved imports `polaris_kernel_supervisor::RestartIntensity`, `polaris_kernel_supervisor::RestartVerdict`, `polaris_kernel_supervisor::RestartWindow`
  Other targets also failed on missing Supervisor/CircuitBreaker/DeadLetterQueue APIs.
$ cargo test -p polaris-kernel-supervisor --no-fail-fast (first implementation)
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: FAILED. 14 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
  backoff observed [100ms, 201ms, 401ms, 801ms], expected [100ms, 200ms, 400ms, 800ms].
  telemetry snapshot observed worker_restarts=0 before publication, expected 2.
$ PROPTEST_CASES=2000 cargo test -p polaris-kernel-supervisor --no-fail-fast (after approved corrections; Windows)
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
$ RUSTFLAGS=-Dwarnings cargo check -p polaris-kernel-supervisor --all-targets
    Finished `dev` profile [unoptimized] target(s) in 13.84s
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-kernel-supervisor --no-deps
    Finished `dev` profile [unoptimized] target(s) in 15.09s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target cargo test -p polaris-kernel-supervisor --no-fail-fast'
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
```

Open: native worker supervision, health timeouts, real process-tree cleanup/parent-death tests, Kani,
restart latency/load and 200-process-kill stress. The crate remains IN PROGRESS.

### E23 — core resource reaper (native orphan reconciliation still open)

Existing eight descriptors/test files preceded implementation and failed on missing APIs. Provides
cancellation, kind priority then reverse registration order, per-step and overall resource deadlines,
panic isolation for cleanup futures, retained failed resources, explicit reports, escaped v1 journal
records (fsynced temporary file + atomic replacement), prefixed-directory orphan sweep, task abort/join,
owned temporary directories, and process stop through the generic PlatformOps seam. Dynamic future
erasure is confined to the heterogeneous cold teardown registry. PID-only or backend-id orphans remain
NeedsOperator; these tests do not establish process start-time identity or live gateway reconciliation.

```text
$ cargo test -p polaris-kernel-reaper --no-fail-fast (red)
error[E0432]: unresolved imports `polaris_kernel_reaper::Locator`, `polaris_kernel_reaper::Outcome`, `polaris_kernel_reaper::Reaper`, `polaris_kernel_reaper::ResourceKind`, `polaris_kernel_reaper::ShutdownConfig`
$ PROPTEST_CASES=1000 cargo test -p polaris-kernel-reaper --no-fail-fast (Windows; result lines)
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.56s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
$ RUSTFLAGS=-Dwarnings cargo check -p polaris-kernel-reaper --all-targets
    Finished `dev` profile [unoptimized] target(s) in 3.14s
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-kernel-reaper --no-deps
    Finished `dev` profile [unoptimized] target(s) in 6.56s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target cargo test -p polaris-kernel-reaper --no-fail-fast'
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.27s
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

Open: real process-tree and parent-death integration (fake PlatformOps in the current seam tests),
start-time identities for PID recovery, backend orphan reconciliation, bounded AV-lock retry,
large cleanup stress, power-loss tests and timing of journal I/O. The resource deadline does not bound
blocking journal I/O, arbitrary synchronous code inside a Teardown future, or uninterruptible OS I/O.
One owner per journal directory is required; concurrent-owner locking is not implemented.

Workspace gates inspected during this continuation (the witness failure was preserved):

```text
$ cargo xtask check-layers
check-layers: 93 packages, 250 edges -> OK
$ cargo xtask check-witness
FAIL ApprovalWitness::new outside sdk/verifier at crates/polaris-proto/tests/command_conversion.rs:109
check-witness: 1 violation(s) -> FAILED
$ python -I @prime/tools/workspace.py check-docs
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
```

### E24 — durable lifecycle schema 2 (v1 compatibility retained)

VERIFIED: configuration/pause/resume use new fixed-size SDK event variants and protobuf tags 19–21;
tags 1–18 and the existing v1 JSON/protobuf golden files are retained. Versioned construction and
JSON/protobuf decode preserve valid old envelope versions and reject new variants mislabeled as v1,
zero/future versions, and storage-header/envelope mismatches. The codec writes the record's actual
schema, rather than upgrading only its storage header. Synthetic fixtures, local toolchains; no keys.

Descriptors/tests preceded implementation. Two extra codec compatibility tests exposed actual
failures before their fixes:

```text
$ cargo test -p polaris-sdk --test session_lifecycle (red; excerpts)
error[E0599]: no variant named `SessionConfigured` found for enum `Event`
error[E0599]: no associated function or constant named `from_versioned` found for struct `EventRecord`
$ cargo test -p polaris-storage-eventlog --test event_codec (red; excerpts)
test version_one_prefix_is_preserved_on_encode ... FAILED
  left: Some([2, 0])
 right: Some([1, 0])
test mismatched_prefix_and_envelope_are_rejected ... FAILED
test result: FAILED. 4 passed; 2 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
$ cargo test -p polaris-sdk -p polaris-proto -p polaris-storage-eventlog --no-fail-fast
test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

The green excerpt is the initial four-test SDK lifecycle group and codec group; the complete command
succeeded, including conversion/property tests and unchanged v1 goldens. A fifth schema-admission
property was added afterward and passes in the current workspace run and Miri. Full logs are locally retained at
`@prime/tmp/p1/lifecycle-{red,codec-red,green}.txt`. Current complete workspace results are in §E27.

```text
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target-miri PROPTEST_CASES=8 MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p polaris-sdk --tests --no-fail-fast -- --skip event_record_json_snapshot'
     Running tests/session_lifecycle.rs (...)
running 5 tests
test lifecycle_records_use_schema_two ... ok
test new_lifecycle_events_cannot_claim_schema_one ... ok
test prop_schema_acceptance_matches_lifecycle_minimum ... ok
test schema_one_records_remain_readable ... ok
test versioned_constructor_validates_schema ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.95s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && bash @prime/tools/setup/kani-guarded.sh polaris-sdk'
Checking harness kani_proofs::lifecycle_schema_admission_is_exact...
VERIFICATION:- SUCCESSFUL
Verification Time: 2.3920226s
Checking harness kani_proofs::probability_new_stays_in_range...
VERIFICATION:- SUCCESSFUL
Verification Time: 0.4201879s
Checking harness kani_proofs::effort_try_from_is_total...
VERIFICATION:- SUCCESSFUL
Verification Time: 0.3285442s
Complete - 3 successfully verified harnesses, 0 failures, 3 total.
kani polaris-sdk exit=0 wall_s=19
```

The full SDK Miri run passed 33 tests; its existing insta snapshot skip is retained because Miri
cannot execute that dependency's socketpair operation. No new skips. Kani proves constructor schema
admission over nondeterministic u16 versions and selected old/new lifecycle payloads; it does not
prove JSON/protobuf parsing, disk persistence, or the kernel. Logs: `lifecycle-miri.txt`,
`lifecycle-kani.txt` under `@prime/tmp/p1/`.

### E25 — local durable kernel sessions (full composition still IN PROGRESS)

VERIFIED: `KernelCtx` owns real CAS/redb/SQLite and reaper handles, sweeps safe directory orphans
before exposing session admission, refuses unresolved PID/backend resources, and unwinds opened
database handles after startup failure. `Session<S>` has exclusive mutable ownership, immutable
configuration/defaults in verified CAS, durable configure/start/pause/resume/archive transitions,
paged gap-checked replay across interleaved sessions, last-wins directive recovery, and a rebuildable
SQLite catalogue. Archived objects release their lease; crashed running sessions acquire a durable
pause before recovery is returned. Mutation admission closes before local teardown.

The first ten session contracts failed on absent APIs before implementation. Additional tests
exercise real temporary databases and injected corruption/failures (the SQLite failure is an actual
abort trigger). Those extra cases were initial-green characterization except the separately observed
timestamp and error-classification regressions below. Generated directive histories use 24 cases.
No network backend, token verification or end-user command dispatch is implied by these APIs.

```text
$ cargo test -p polaris-kernel --no-fail-fast (initial red; excerpt)
error[E0432]: unresolved imports `polaris_kernel::Archived`, `polaris_kernel::Created`, `polaris_kernel::KernelCtx`, `polaris_kernel::KernelError`, `polaris_kernel::Session`, `polaris_kernel::SessionConfig`, `polaris_kernel::storage`
$ cargo test -p polaris-kernel --test session_typestate created_configures_to_configured -- --exact (red)
assertion `left == right` failed: registry creation time comes from the durable configuration event
  left: Some(2026-10-21T09:07:20.682649Z)
 right: Some(2026-10-21T09:07:20.6806207Z)
$ cargo test -p polaris-kernel --test error_taxonomy (red)
assertion `left == right` failed: a live database owner may release its lock
  left: Fatal
 right: Transient
$ cargo test -p polaris-kernel --no-fail-fast (green; result groups)
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.76s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target cargo test -p polaris-kernel -p polaris-code-intel --no-fail-fast' (kernel groups)
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 18 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.52s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.19s
```

The timestamp fix uses the committed event timestamp at explicit microsecond precision for both
catalogue creation/update and replay. Storage error categories now delegate through composition;
`ProjectionBehind { seq, source }` explicitly reports a durable commit whose catalogue update failed.
Compile-fail doctests reject directive mutation from Paused and Archived. Other tests cover duplicate
owners, D11 default isolation, restart recovery, >256 interleaved records, lost registry rebuild,
corrupt CAS, invalid lifecycle order, startup rollback, closed mutations and unresolved orphans.
Logs: `kernel-session-{red,green,linux}.txt`, `kernel-created-at-red.txt`,
`kernel-error-kind-red.txt`, `core-linux-current.txt` in `@prime/tmp/p1/`.

The pre-existing witness gate rejected a proto wire test constructing `ApprovalWitness` outside its
allowed boundary. Its assertions/helper were moved intact into the verifier's test target; the gate
was not changed. The verifier's owned manifest has test-only proto/prost dev dependencies. This does
not implement its production verification pipeline. The storage facade now re-exports its children.
The UI model received only the event-consumer compatibility needed for durable reopen: a paused or
resumed archived row becomes open; a regression was observed red then green. No rendering/UI feature
work was added during this core continuation.

Open: verified command dispatcher/operators, Biscuit/AST/S1 policy verification, actual native workers,
process/platform integration, concrete model/sandbox backends, configuration/log/secret assembly,
signal handling, full reverse-order shutdown/drain/flush, and end-to-end load/failure verification.
The local `configure`/`apply` composition APIs perform explicit blocking I/O and require a trusted
owner; they are not a network authorization boundary. Reaper's synchronous-I/O limitations in §E23
still apply. No claim of a complete `Kernel<B>`/`KernelHandle: KernelApi` or phase exit.

### E26 — adversarial traversal and precise symbol conflicts

VERIFIED: the first whole-workspace run was allowed to finish (exit 101, two failed targets), rather
than restarted on an observation timeout. Its old parser-pool target completed in 625.79 s. A separate
temporary diagnostic isolated Rust parse (340 ms) and skeleton generation from the slow fingerprint;
that diagnostic was stopped after diagnosis. Local tree-sitter 0.27.1 source shows `cursor.depth()`
scans all ancestors. The shared preorder walk now maintains depth on visible child/parent moves;
fingerprinting uses that walk. Five-grammar native-depth comparisons also cover subtree skipping.
The 20k-deep fingerprint retains the exact old digest `6567796617403192368`.

```text
$ cargo test -p polaris-code-intel --test parser_pool deep_tree_fingerprint_has_a_bounded_walk_cost -- --exact (red)
fingerprint took 22.7772799s: FastFingerprint(6567796617403192368)
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 11 filtered out; finished in 22.82s
$ cargo test -p polaris-code-intel --test parser_pool -- --nocapture (green)
test hostile_nesting_does_not_overflow_the_stack ... ok
test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.68s
$ cargo test -p polaris-code-intel --lib tracked_depth_matches_native_cursor_on_mixed_trees
running 1 test
test walk::tests::tracked_depth_matches_native_cursor_on_mixed_trees ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
$ cargo test -p polaris-code-intel --test symbol_merge adjacent_one_sided_changes_are_not_reported_as_conflicts -- --exact (red)
  left: [SymbolRef { name: "a", kind: Function }, SymbolRef { name: "b", kind: Function }, SymbolRef { name: "c", kind: Function }]
 right: [SymbolRef { name: "a", kind: Function }]
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 19 filtered out; finished in 0.00s
$ cargo test -p polaris-code-intel --no-fail-fast (merge group)
test adjacent_one_sided_changes_are_not_reported_as_conflicts ... ok
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.03s
```

The existing merge assertion remains intact. Myers can coalesce adjacent declaration replacements
into one hunk; replacements whose kind/name/occurrence identities align are now split per declaration.
Insertions, deletions and reordered identities retain conservative handling. Regression coverage
places the actual conflict at each of three adjacent functions and swaps merge sides. Every code-intel
test group passed on Windows and Linux (93 tests/doctests per platform). These are functional/cost
regressions on synthetic inputs, not a general throughput, corpus-security or phase-exit claim.
Logs: `deep-fingerprint-red.txt`, `parser-pool-green.txt`, `walk-depth-green.txt`,
`merge-conflict-red.txt`, `codeintel-current.txt`, `core-linux-current.txt` in `@prime/tmp/p1/`.

### E27 — current workspace and consistency gates, 2026-10-21

VERIFIED: after the E24–E26 fixes, `cargo test --workspace --no-fail-fast` exited 0 on Windows.
Aggregating its actual Cargo result lines gives 897 passed, zero failed and zero ignored across 305
test binaries/doctest groups. Many skeleton packages contribute zero tests; this is not evidence
that every crate or phase is implemented. Full output: `@prime/tmp/p1/core-workspace-current.txt`.
The earlier failed workspace output is retained separately in `core-workspace-tests.txt`.

```text
$ cargo test --workspace --no-fail-fast (current; lifecycle excerpt)
running 5 tests
test versioned_constructor_validates_schema ... ok
test schema_one_records_remain_readable ... ok
test lifecycle_records_use_schema_two ... ok
test new_lifecycle_events_cannot_claim_schema_one ... ok
test prop_schema_acceptance_matches_lifecycle_minimum ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized] target(s) in 3.09s
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-kernel -p polaris-sdk -p polaris-proto -p polaris-storage -p polaris-storage-eventlog -p polaris-kernel-verifier -p polaris-code-intel --no-deps
    Finished `dev` profile [unoptimized] target(s) in 29.44s
   Generated C:\mDev\Polaris\target\doc\polaris_code_intel\index.html and 6 other files
$ cargo xtask check-members
check-members: 93 packages -> OK
$ cargo xtask check-layers
check-layers: 93 packages, 250 edges -> OK
$ cargo xtask check-witness
check-witness: 0 violation(s) -> OK
$ cargo fmt --all --check
(exit 0; no output)
$ python -I @prime/tools/workspace.py render-docs
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: refreshed 10 generated blocks
$ python -I @prime/tools/workspace.py render-docs --check
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: up to date (10 generated blocks)
$ python -I @prime/tools/workspace.py check-docs
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
```

Compiler/doc logs: `core-workspace-check-current.txt`, `core-doc-current.txt` in `@prime/tmp/p1/`.
PLAN session signatures, SDK/eventlog contracts, consumer matches, protobuf tags, descriptors,
authoritative workspace metadata and generated phase tables were updated together. No lint policy,
witness gate, golden file, or existing conflict assertion was loosened. No commits made.
P1/full-core completion remains IN PROGRESS with E25's gaps; UI work stays deferred. The full PLAN
has not been reviewed line by line in this continuation; only the relevant contract/module sections
were reviewed. Existing missing §E18 historical evidence remains flagged above.
