# Polaris — Phase status

> **Document:** `@prime/PHASES_STATUS.md` · **Snapshot:** 2026-10-22 (P1 logic completion verification) · **Branch:** `main`
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
| PASS | 44 |
| FAIL | 3 |
| IN PROGRESS | 45 |
| BLOCKED-ON-ACCESS | 15 |
| NOT STARTED | 57 |
| **total** | 164 |

**P1 non-UI logic: PASS. UI remains deferred by the user.** Implementation, native workspace checks, current-graph mandatory fuzz and document consistency checks are complete. The detailed verdict and every limitation live in [P1_LOGIC_STATUS.md](P1_LOGIC_STATUS.md), with durable raw excerpts under [evidence/p1/verification.txt](evidence/p1/verification.txt).

P1 now composes actual CAS/redb/SQLite, a signed directive verifier, bounded receipts/replay, native process ownership, workspace read tools and a real headless CLI. Windows hard-kill and Linux journal-throughput failures were fixed against failing regressions; earlier outputs remain retained. Tokio 1.54.0 was yanked during verification and was replaced with 1.54.1 in all resolution graphs. No advisory exemption was introduced.

**Full M1/P1 UI acceptance remains open:** TUI, terminal composer/completion and dump snapshots are excluded by the user's direction. Facades remain IN PROGRESS where later-phase children or the full execution bundle are open; their P1 exports and local profile are checked separately. P2–P9 source compiles in the workspace, but this does not mark those feature phases complete.

P0-SKEL entry is met. P0 overall still has manual ConPTY testing, account requests, the Slint licence decision and dependency vetting open. PID-only orphan identities remain fail-closed for operator review. Full model/sandbox/S1 assembly, remote transports, live credentials and UI retain their own phase gates.

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
| P0-ENV-7 `deny.toml`, cargo-vet, Slint licence decision | IN PROGRESS | §E11 historical audit snapshot; §E29 current `cargo deny check` all ok, `cargo vet --locked` FAIL: 241 unvetted dependencies in the expanded graph. Existing exemptions are not audits; no new exemptions added. Slint licence choice remains open (team, ADR-014). |
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
| P1-X-1 composition-root wiring v0 | PASS (§E30: actual LocalBackends, signed directive dispatch, read tools, durable stores, session typestate and shutdown) |
| P1-X-2 TUI shell | NOT STARTED (user-deferred UI) |
| P1-X-3 rustls aws-lc-rs provider in `main` | PASS (§E30: real main and subprocess provider smoke) |
| P2-X-1 Polaris sandbox image + blueprint | IN PROGRESS — final guest image and live fidelity gate pending; [P2 audit](P2_LOGIC_STATUS.md) |
| P2-X-2 `/onboarding` + `polaris doctor` | IN PROGRESS — headless inventory probes implemented; current machine report and coverage audit pending |
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
Raw evidence for the core continuations is in §E21–§E29; prior crate evidence remains in §E14–§E17.
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
| `crate:polaris-cli` | `polaris-cli` | P1 | IN PROGRESS | P1 non-UI main/run/doctor/RTK, aws-lc provider, logging/signals and real local kernel smoke pass (§E30). TUI/dump/terminal UI remain user-deferred; full CLI gate remains open. |
| `crate:polaris-sdk` | `polaris-sdk` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: validated contracts and enum forwarding; SDK Miri supported tests and Kani 3/3. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-proto` | `polaris-proto` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded generated protocol, SDK conversions and frozen descriptor contract; native Windows suite, Linux warning-denied check/docs and proto_decode fuzz. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-config` | `polaris-config` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded layered config, trust/provenance/secret refs and real Windows sharing-lock retry. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-log` | `polaris-log` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: redacted bounded rings/queue, zero-allocation enqueue, JSON, UTC daily-size rotation and application flush hooks. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-telemetry` | `polaris-telemetry` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: cost/budget/histogram/snapshot/replay contracts and 1M-fold measurement; no invented provider cache data. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-code-intel` | `polaris-code-intel` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: five grammar ABIs, depth/size/time bounds, deterministic map, metrics/merge; Kani full-u32 budget, 3 fuzz targets and 100k-file measurement. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-storage-cas` | `polaris-storage-cas` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded atomic/integrity storage; universal formatter Kani, short-ref fuzz, actual ENOSPC and release latency/hash workloads. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-storage-eventlog` | `polaris-storage-eventlog` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded async owner/group commit, lazy replay, real killed-writer and ENOSPC recovery plus 1M-event workload. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-storage-registry` | `polaris-storage-registry` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded SQLite owner/queries, SQL-before-LIMIT session isolation, Unicode snippets, 2-process/crash recovery, fuzz and 1M-message measurements. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-storage-cache` | `polaris-storage-cache` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: scoped L1 and actual redb L2, cancellation/pressure/Loom coverage and 8M-get measurement; semantic L3 is P5. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-storage` | `polaris-storage` | P1 | IN PROGRESS | P1 exports/layout doctest and CAS/eventlog/registry/cache pass (§E30). Later vector/sync child gates stay open. |
| `crate:polaris-kernel-bus` | `polaris-kernel-bus` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: durable-prefix/FIFO/lag/close tests; zero allocation, Loom, 13 supported Miri tests, Kani 2/2, bus_ops fuzz and million-event native floods. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-kernel-directives` | `polaris-kernel-directives` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded grammar/render/persistence tests; Miri and 2 cursor proofs; mandatory fuzz and 1MiB throughput corpus. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: task/native ownership, bounded intensity/DLQ/backoff, health timeouts and clean/crash classification; 200 native kills with exact restart count and zero roots. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: ordered cancellation-safe teardown and durable journal; release checkpoint batching, emergency process guard, safe orphan handling; 1070-resource workloads on Windows/WSL. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: P1 bounded read/pure tools, actual CAS expand/session recall, stable schemas/catalog and diff_apply fuzz; workspace read TOCTOU limit documented. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded JSON/text streams, real recorded cargo fixtures, actual CAS preservation, subprocess ledger contention and release throughput workloads. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-tools` | `polaris-tools` | P1 | IN PROGRESS | P1 builtin/RTK exports and generated catalog consistency pass (§E30). Math child is P3; full child gate stays open. |
| `crate:polaris-kernel` | `polaris-kernel` | P1 | IN PROGRESS | P1 LocalBackends, signed directive dispatcher, actual read tools/stores/platform, durable session typestate and cancellation-safe shutdown pass (§E30). Full model/sandbox/S1 execution bundle and later child gates retain P2/P3/P4 scope. |
| `crate:polaris-ui-model` | `polaris-ui-model` | P1 | IN PROGRESS | 111 Windows tests incl. 1,000-case properties, 5 initial snapshots reviewed, Unicode chip regression saved; Linux check; ui_completion fuzz 207,648 runs/61 s (§E21). UI deferred by user: remaining settings/provenance, full HUD, metadata arity, Quick Chips, fixtures/Miri and TUI integration. |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui` | `polaris-ui` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: 1M events with 1/8/64 readers, stalled subscriber, churn, bounded command accounting and RSS; Windows and WSL native suites. Final consistency/fuzz checkpoint recorded there. |

### P2

<!-- generated: P2 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | P2 | IN PROGRESS | main session, test-first; evidence pending |
| `crate:polaris-security-automata` | `polaris-security-automata` | P2 | IN PROGRESS | 5 descriptors; red then green: 27 tests (19-rule default pack with a positive+negative per rule, scan == regex-crate reference proptest, user-regex guard, integer fixed-point entropy, redactor idempotence, dhat zero-alloc scan/any_match/redact_into); 3 planted bugs caught (one after tightening a too-loose assertion) (§E18). Open: fuzz targets, Miri, load/scalability tables, bypass corpus, chunk-boundary policy |
| `crate:polaris-platform-wsl` | `polaris-platform-wsl` | P2 | IN PROGRESS | Real WSL discovery, bounded argv execution, doctor, path/config contracts implemented. Targeted Windows tests pass; full P2 gate remains open. |
| `crate:polaris-platform-process` | `polaris-platform-process` | P2 | IN PROGRESS | P1 native ownership pulled forward: Windows/Linux tree, shutdown, worker restart and Drop tests PASS; @prime/tmp/p1/p1-native-{windows,wsl}-current.txt. P2 PTY/watch features remain open. |
| `crate:polaris-platform` | `polaris-platform` | P2 | IN PROGRESS | Facade exposes process and WSL children; native process implementation pulled forward for P1 supervision. |
| `crate:polaris-vcs` | `polaris-vcs` | P2 | IN PROGRESS | Real gix status, bounded git fallback, persistent CAS undo and worktree lifecycle implemented; complete recovery and test audit pending. |
| `crate:polaris-openshell-client` | `polaris-openshell-client` | P2 | IN PROGRESS | Pinned real SDK adapter, explicit policy CID binding, mTLS, bounded transfer/exec and cancellation guards implemented; live acceptance pending. |
| `crate:polaris-openshell-policy` | `polaris-openshell-policy` | P2 | IN PROGRESS | Actual upstream pinned schema, policy validation, role/tier documents and provider YAML implemented; targeted contracts pass, live policy matrix pending. |
| `crate:polaris-openshell-nemoclaw` | `polaris-openshell-nemoclaw` | P2 | IN PROGRESS | Native versioned blueprint validation, canonical digest, actual backend apply and protocol handshake implemented; image/live evidence pending. |
| `crate:polaris-openshell` | `polaris-openshell` | P2 | IN PROGRESS | Actual client/policy/blueprint facades compile; child live gates remain open. |
| `crate:polaris-security` | `polaris-security` | P2 | IN PROGRESS | Facade reexports implemented children; child completion tracked separately. |
| `crate:polaris-runtime-leases` | `polaris-runtime-leases` | P2 | IN PROGRESS | P2 authority tests: 6 passed, including immediate overlap, AST conflict, FIFO cancellation, TTL wake-up, OCC/churn and 64-frame/20-file contention. Full phase proof/allocation audit pending; see @prime/P2_LOGIC_STATUS.md. |
| `crate:polaris-runtime-guards` | `polaris-runtime-guards` | P2 | IN PROGRESS | P2 proof regressions: 7 passed, including all Cargo groups, stderr binding, hidden earlier failures and malformed/overflow counts. Blast/AST and live/performance gates remain open; see @prime/P2_LOGIC_STATUS.md. |
| `crate:polaris-runtime` | `polaris-runtime` | P2 | IN PROGRESS | Runtime child facade exports written; guards and DAG logic UNVERIFIED, full frame/context/lease/research assembly remains pending. |
| `crate:polaris-spike-sandbox` | `polaris-spike-sandbox` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P3

<!-- generated: P3 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-ratelimit` | `polaris-ratelimit` | P3 | IN PROGRESS | Production RPM/TPM reservation, priority admission and usage reconciliation in progress; UNVERIFIED, validation deferred by user. |
| `crate:polaris-security-capability` | `polaris-security-capability` | P3 | IN PROGRESS | §E28–§E29: real Ed25519/Biscuit root issuance, signed monotone attenuation, scoped rights/metrics/expiry/hash checks, revocation hook and bounded evaluation. 20 contracts + 2 compile-fail tests, Windows/Linux, 95,847 fuzz executions, isolated expiry Kani proof and path Miri. Published-source vendor patch verified; dependency audit debt and full mutation-pipeline integration remain open. |
| `crate:polaris-security-secrets` | `polaris-security-secrets` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud-token-factory` | `polaris-nebius-cloud-token-factory` | P3 | IN PROGRESS | Production reqwest chat/SSE adapter, tool replay, model listing and embedding transport written; UNVERIFIED. Credentials/live API, throughput and validation remain pending. |
| `crate:polaris-nebius-cloud-sandboxes` | `polaris-nebius-cloud-sandboxes` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-nebius-cloud` | `polaris-nebius-cloud` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-models` | `polaris-models` | P3 | IN PROGRESS | Production model catalog, effort policy and cancellable local slot lifecycle in progress; UNVERIFIED, validation deferred by user. |
| `crate:polaris-tools-math` | `polaris-tools-math` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-frames` | `polaris-runtime-frames` | P3 | IN PROGRESS | Production frame typestate, bounded quotas, attenuation and streamed step execution in progress; UNVERIFIED, full root assembly remains pending. |
| `crate:polaris-runtime-context` | `polaris-runtime-context` | P3 | IN PROGRESS | Production deterministic prompt assembly and byte-exact CAS context folding in progress; UNVERIFIED, validation deferred. |
| `crate:polaris-kernel-operators` | `polaris-kernel-operators` | P3 | IN PROGRESS | Production typed operator combinators, cancellation, bounded retries and stage-order contracts in progress; UNVERIFIED, full composition remains open. |
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | P3 | IN PROGRESS | Production hybrid mutation verification and evidence-backed approvals in progress; UNVERIFIED, validation deferred by user. Existing wire-contract tests preserved (§E25). |
| `crate:polaris-spike-ratelimit` | `polaris-spike-ratelimit` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P4

<!-- generated: P4 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-worker` | `polaris-worker` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-engine` | `polaris-system-one-engine` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-client` | `polaris-system-one-client` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-pii` | `polaris-security-pii` | P4 | IN PROGRESS | Production reversible bounded DFA/entropy privacy vault in progress; UNVERIFIED. NER span API and real sealed persistence integration remain pending. |
| `crate:polaris-security-rules` | `polaris-security-rules` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-system-one-schemas` | `polaris-system-one-schemas` | P4 | IN PROGRESS | Production schema catalog, thresholds, calibration and restrictive overrides in progress; new implementation UNVERIFIED, validation deferred by user. |
| `crate:polaris-system-one-router` | `polaris-system-one-router` | P4 | IN PROGRESS | Production routing, effort restrictions, tool/file ranking and sandbox recommendations written; UNVERIFIED, tests and validation deferred by user. |
| `crate:polaris-system-one-verifier` | `polaris-system-one-verifier` | P4 | IN PROGRESS | Production deterministic safety, preflight/postflight and merge arbitration in progress; UNVERIFIED, validation deferred by user. |
| `crate:polaris-system-one-watchdog` | `polaris-system-one-watchdog` | P4 | IN PROGRESS | Production bounded loop detection, advisory role/context guards and semantic drift inspection in progress; UNVERIFIED, validation deferred by user. |
| `crate:polaris-system-one` | `polaris-system-one` | P4 | IN PROGRESS | Kernel-side facade exports production schemas/router/verifier/watchdog; UNVERIFIED, inference worker assembly and calibration remain pending. |
| `crate:polaris-spike-s1` | `polaris-spike-s1` | P4 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P5

<!-- generated: P5 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-vector` | `polaris-storage-vector` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ingest` | `polaris-ingest` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-search-providers` | `polaris-search-providers` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-search-sift` | `polaris-search-sift` | P5 | IN PROGRESS | Production provenance-preserving ranking/filtering written with bounded S1 calls and fail-open fallback; UNVERIFIED, source race and validation remain open. |
| `crate:polaris-search` | `polaris-search` | P5 | IN PROGRESS | Child module facade exports written; sifter is UNVERIFIED, live providers and search racing remain pending. |
| `crate:polaris-nvidia-docs` | `polaris-nvidia-docs` | P5 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P6

<!-- generated: P6 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-nebius-cloud-data-lab` | `polaris-nebius-cloud-data-lab` | P6 | IN PROGRESS | Production local consent-gated trajectory masking/export in progress; UNVERIFIED. Remote API and fine-tuning remain pending API/access. |
| `crate:polaris-nebius-cloud-ai-cloud` | `polaris-nebius-cloud-ai-cloud` | P6 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P7

<!-- generated: P7 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-guest` | `polaris-guest` | P7 | IN PROGRESS | P2 protocol-version and locked conditional sync-cas helper implemented; Linux filesystem tests 6 passed plus host/Linux checks. Final image/live sync gate pending. Full structured exec/browser remains P7. |
| `crate:polaris-ext-mcp` | `polaris-ext-mcp` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ext-skills` | `polaris-ext-skills` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ext` | `polaris-ext` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-browser` | `polaris-browser` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-hacker` | `polaris-hacker` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-runtime-dag` | `polaris-runtime-dag` | P7 | IN PROGRESS | Production bounded DAG scheduler, named pools, speculative races and cancellation written; UNVERIFIED, validation deferred by user. |
| `crate:polaris-runtime-research` | `polaris-runtime-research` | P7 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P8

<!-- generated: P8 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-devenv-nix` | `polaris-devenv-nix` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-devenv-docker` | `polaris-devenv-docker` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-devenv-cicd` | `polaris-devenv-cicd` | P8 | IN PROGRESS | Production typed CI pipelines, matrix reporting and sandbox engine in progress; UNVERIFIED, Dagger and validation deferred. |
| `crate:polaris-devenv` | `polaris-devenv` | P8 | IN PROGRESS | Child module facade exports written; CI implementation UNVERIFIED, Nix/Docker adapters remain pending. |
| `crate:polaris-system-one-nano` | `polaris-system-one-nano` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P9

<!-- generated: P9 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-desktop` | `polaris-desktop` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-web` | `polaris-web` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-daemon` | `polaris-daemon` | P9 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-rpc` | `polaris-rpc` | P9 | IN PROGRESS | Production request authentication and tonic service adapters in progress; UNVERIFIED. Named-pipe/mTLS listener assembly and validation remain open. |
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

### E28 — real signed capability boundary (not the full hybrid verifier)

VERIFIED: `polaris-security-capability` now issues real Ed25519 Biscuit roots, validates bounded
serialized chains against an expected issuer, appends signed scope checks without the root private
key, and authorizes trusted operation facts with default-deny Datalog. Vocabulary version 1 covers
path/symbol, actual churn, injected current time, unsafe flag, exact diff hash and requested right;
grants and issuing-frame provenance are authority facts. Tokens are bearer credentials: the frame
fact records provenance and does not claim non-transferable holder authentication.

Generated blocks contain no ambient mutation facts/rules. Biscuit 6's `trusting authority` also
includes the current block and authorizer; there is no authorizer-only keyword. Descendant facts
cannot satisfy ancestor checks, and the authorizer policy ignores descendant grants/version facts.
Real adversarial appended blocks test these restrictions. Datalog errors/limits never approve.
Requested wider child scopes remain intersected with all ancestor checks rather than being falsely
reported as an immediately validated subset.

The descriptor and first contracts preceded implementation and failed on absent APIs. Additional
cryptographic/fact-explosion/version/compile-fail cases were initial-green characterization. A
device-name spelling exposed a separate normalization regression before its fix:

```text
$ cargo test -p polaris-security-capability --no-fail-fast (initial RED; excerpt)
error[E0432]: unresolved imports `polaris_security_capability::Authority`, `polaris_security_capability::Authorization`, `polaris_security_capability::CapabilityError`, ...
$ cargo test -p polaris-security-capability --test capability_contract resource_paths_reject_aliases_and_escapes -- --exact (RED)
thread 'resource_paths_reject_aliases_and_escapes' (15160) panicked at crates\polaris-security\crates\capability\tests\capability_contract.rs:198:9:
"src/CON .txt"
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 16 filtered out; finished in 0.00s
$ cargo test --workspace --no-fail-fast (final capability group)
running 20 tests
test token_facts_cannot_spoof_ambient_facts_or_grants ... ok
test datalog_fact_explosion_exhausts_budget_without_granting ... ok
test prop_attenuation_is_monotone ... ok
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.61s
```

Twenty integration contracts cover wrong issuer/proof tampering, exact/prefix/symbol scopes, all
rights, inclusive churn/exclusive expiry, pre-issuance clock rollback, hash mismatch/missing hash,
unsafe changes, widening attempts, three fixed attenuation levels, 256 generated multi-level chains,
256 arbitrary-token cases, inherited revocation, full 32-block reparse/authorization and size limits.
Private key/token fields are compile-fail doctests. Parsing is bounded to 64 KiB and 32 blocks before
signature verification; Datalog is bounded to 4096 facts, 64 iterations and 50 ms. The valid signed
fact-explosion fixture actually exhausts the evaluator rather than merely failing an unrelated check.
Library/OS allocation and native scheduling are not claimed to have a hard wall-clock deadline.

```text
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target cargo test -p polaris-security-capability --no-fail-fast'
test result: ok. 20 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.06s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.09s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && bash @prime/tools/setup/kani-guarded.sh polaris-security-capability'
Checking harness kani_proofs::expiry_never_wraps_or_extends_lifetime...
VERIFICATION:- SUCCESSFUL
Verification Time: 0.34348804s
Complete - 1 successfully verified harnesses, 0 failures, 1 total.
kani polaris-security-capability exit=0 wall_s=13
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target-miri MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p polaris-security-capability --test capability_contract resource_paths_reject_aliases_and_escapes -- --exact'
running 1 test
test resource_paths_reject_aliases_and_escapes ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 19 filtered out; finished in 1.66s
$ wsl -d Ubuntu-24.04 -- bash -lc 'source ~/.cargo/env && CARGO_TARGET_DIR=~/.cache/polaris-target-fuzz cargo +nightly fuzz run --fuzz-dir tests/fuzz --target x86_64-unknown-linux-gnu biscuit_token_bytes -- -max_total_time=60 -max_len=65568'
#95847 DONE cov: 4974 ft: 13140 corp: 385/946Kb lim: 5734 exec/s: 1545 rss: 436Mb
Done 95847 runs in 62 second(s)
```

The Linux test excerpt predates the last property-only expansion from one random child to generated
chains/256 cases; the final Windows workspace group includes that expansion. Kani proves only
checked expiry arithmetic over all i64 issue times/u128 microsecond lifetimes. Miri covers only
the selected lexical path test; cryptography/Datalog/OS entropy are not Miri-proven. The final fuzz
run used the macro-free vendored library before only formatting/proof-helper/property changes; a
prior macro-enabled run completed 85,903 executions. Generated 1/4/16-block corpus seeds include
valid real signatures, rather than testing only instant parser errors. Corpus/artifacts are ignored;
no real credentials or private keys are committed.

Current-source local latency sample (1000 calls per row, dev profile, synthetic scopes, other work
running; not release or production throughput):

```text
$ cargo run -p polaris-security-capability --example capability_latency
synthetic fixture; samples=1000; debug_assertions=true
blocks=1 bytes=555 parse_signatures=false p99_ns=122800
blocks=1 bytes=555 parse_signatures=true p99_ns=318600
blocks=4 bytes=1533 parse_signatures=false p99_ns=241200
blocks=4 bytes=1533 parse_signatures=true p99_ns=736400
blocks=16 bytes=5445 parse_signatures=false p99_ns=649400
blocks=16 bytes=5445 parse_signatures=true p99_ns=3691000
```

These results do not support the chatlog's approximate 5 µs claim. The example reports cached-token
authorization separately from parsing/signature verification and writes only synthetic fuzz seeds.
Logs under `@prime/tmp/p1/`: `capability-{red,device-red,current,linux-current,kani-current,miri,fuzz-current,latency-current}.txt`.

Open: full mutation-fact construction (trusted filesystem containment, actual AST/churn/unsafe/hash),
session issuer distribution, hybrid metric/S1 stages and approval witness/dispatcher wiring, dependency
audits (§E29), broader malicious corpus, release-profile/workload calibration. Lexical path checks
reject traversal/absolute/drive/UNC/ADS/device aliases and normalize separators/Windows ASCII case;
they do not resolve symlinks, Windows short names or Unicode case aliases. Trusted filesystem owners
must supply resolved in-workspace targets. No actual execution or ApprovalWitness is exposed here.
The full phase/goal remains active; UI stays deferred.

### E29 — pinned dependency feature repair and current gates

VERIFIED: activating the planned Biscuit dependency exposed its default macro dependency's
[RUSTSEC-2026-0173](https://rustsec.org/advisories/RUSTSEC-2026-0173.html) unmaintained advisory.
Disabling unused macros initially failed to compile due to
[upstream issue 305](https://github.com/eclipse-biscuit/biscuit-rust/issues/305): five unconditional
`ToAnyParam` imports referenced a macro-gated trait. The published 6.0.0 crate is now vendored with
exactly seven missing feature guards (those imports, a macro/test-only PublicKey import and a
PEM-only helper). No cryptographic/runtime logic was changed. Existing workspace formatting stays
enabled, including sorted imports and narrowly verified generated prost-attribute/empty-struct
formatting. No lint suppression, advisory ignore or audit exemption was added.

`vendor/biscuit-auth/POLARIS-PATCH.md` records provenance, patch scope and removal conditions.
Published archive SHA-256 is `d5884fc86b3e21f5649ef4326e17ef729b3096e6502deaf13db7b7fb05bb992b`.
The checksum manifest itself is pinned in `@prime/tools/check-vendored-biscuit.py`; it reverses only
the declared guards/formatting and checks every published file's exact hash and the file set.
An isolated temporary-copy test appended a cryptographic-source comment and was rejected. This
mechanical provenance check is not an independent security audit of the library.

```text
$ cargo deny check (before feature repair)
error[unmaintained]: proc-macro-error2 is unmaintained
ID: RUSTSEC-2026-0173
advisories FAILED, bans ok, licenses ok, sources ok
$ cargo test -p polaris-security-capability --no-fail-fast (macros disabled, before vendor fix)
error[E0432]: unresolved import `super::ToAnyParam`
error: could not compile `biscuit-auth` (lib) due to 5 previous errors
$ python -I @prime/tools/check-vendored-biscuit.py
check-vendored-biscuit: 94 published files; 7 feature guards + prost attribute formatting -> OK
$ python -I @prime/tools/check-vendored-biscuit.py <isolated modified vendor fixture>
check-vendored-biscuit: FAILED: unexpected upstream change: src/crypto/ed25519.rs
$ cargo deny check (current)
advisories ok, bans ok, licenses ok, sources ok
$ cargo vet --locked (current)
Vetting Failed!
241 unvetted dependencies:
  biscuit-auth:6.0.0 missing ["safe-to-deploy"]
```

`audit-as-crates-io = true` deliberately retains audit enforcement for the vendored dependency,
following [cargo-vet's policy](https://mozilla.github.io/cargo-vet/config.html#audit-as-crates-io).
The expanded graph has 241 unaudited entries; historical §E11's green snapshot is not current proof.
No blanket exemption/trust declaration was introduced. This audit gate is FAIL, not waived or PASS.

```text
$ cargo test --workspace --no-fail-fast
(exit 0; actual Cargo result-line aggregate: 919 passed, 0 failed, 0 ignored; 306 result groups)
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets
    Finished `dev` profile [unoptimized] target(s) in 5.91s
$ RUSTDOCFLAGS=-Dwarnings cargo doc -p polaris-security-capability --no-deps
    Finished `dev` profile [unoptimized] target(s) in 13.74s
   Generated C:\mDev\Polaris\target\doc\polaris_security_capability\index.html
$ cargo xtask check-members
check-members: 93 packages -> OK
$ cargo xtask check-layers
check-layers: 93 packages, 250 edges -> OK
$ cargo xtask check-witness
check-witness: 0 violation(s) -> OK
$ cargo fmt --all --check
(exit 0; no output)
$ cargo generate-lockfile --offline --manifest-path @prime/tools/dep-probe/Cargo.toml
     Locking 1698 packages to highest Rust 1.99 compatible versions
$ python -I @prime/tools/workspace.py render-docs --check
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: up to date (10 generated blocks)
$ python -I @prime/tools/workspace.py check-docs
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
```

Counts include zero-test skeletons and do not prove phase completion. Root Cargo dependency/excludes
and the standalone dependency probe derive portably from `workspace.toml` (explicit UTF-8 comparison
matches the generated root exactly). The fuzz workspace uses the same dependency source through
the capability crate's inherited workspace dependency. Its manifest/lock, README and proof script
were updated; no UI feature work was added. The normal capability dependency graph has zero ring
packages. Raw local logs: `capability-{workspace-tests-current,workspace-check-current,doc,deny-current,vet-current,dep-probe,tree}.txt`.

## E30 — P1 logic / current-graph completion checkpoint (2026-10-22)

See [P1_LOGIC_STATUS.md](P1_LOGIC_STATUS.md) for the implemented feature matrix, exact local-log
paths, measured workloads, failure fixes and scope boundaries. Durable raw command/output excerpts:
[evidence/p1/verification.txt](evidence/p1/verification.txt).

Windows `cargo test --workspace --no-fail-fast`: exit 0, 1009 passed / 0 failed / 14 ignored
workload cases across 345 Cargo result groups. All 14 ignored workload cases were also run explicitly
under their documented release/disk-full conditions (not counted as executed by the fast suite).
Linux warning-denied workspace check/docs, formatting and 93-package leg: PASS. Updated dependency
deny: `advisories ok, bans ok, licenses ok, sources ok`. These counts include existing UI-model tests
and zero-test future skeletons; they do not imply UI or later-phase feature completion.

All six mandatory WSL ASan targets passed for 61–65 seconds each on the updated graph. Member/layer/witness checks report 93 packages / 258 edges / zero witness violations; generated documents, 136 descriptor JSON files and root/fuzz formatting are checked. The linked P1 verdict records exact counts and limits.
Historical §E29's unverified-source description and older dependency-deny result are superseded for
this P1 checkpoint; unresolved P0 audits/access/licensing are retained.
