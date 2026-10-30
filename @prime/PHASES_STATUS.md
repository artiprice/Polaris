# Polaris — Phase status

> **Document:** `@prime/PHASES_STATUS.md` · **Snapshot:** 2026-10-26 (non-UI continuation; historical P1 and later UI evidence retained) · **Branch:** `main`
> (the team's periodic commits picked up part of the P0 work; this session committed nothing) ·
> **Reference PC:** Windows 11 LTSC 2024, Ryzen 7 2700, 16 GB RAM, GTX 1650 4 GB, rustc/cargo 1.99.0,
> WSL2 Ubuntu 24.04.5.
>
> Rules: `PASS` requires the command, its raw output and the conditions (Evidence log §E, or a spike's
> `RESULT.md` + raw file). `BLOCKED-ON-ACCESS` = needs a key/account that does not exist yet (D9). `DESCOPED` = will not be done
> because the service is unavailable to the team (D10: Nebius in Ukraine). A doc-only
> skeleton is **not** an implementation. Hand-edited except `<!-- generated: … -->` blocks: crate rows come from
> the `status`/`evidence` keys in `@prime/tools/workspace.toml`; the summary counts every status row in this file.
> Refresh with `python -I @prime/tools/workspace.py render-docs`.

## Summary

<!-- generated: status summary -->
| Status | Rows |
|---|---|
| PASS | 48 |
| FAIL | 3 |
| IN PROGRESS | 91 |
| BLOCKED-ON-ACCESS | 2 |
| DESCOPED | 8 |
| NOT STARTED | 8 |
| **total** | 160 |

**Historical P1 non-UI logic: PASS (§E30). CLI UI implementation is now in progress (§E33).** Implementation, native workspace checks, current-graph mandatory fuzz and document consistency checks are complete. The detailed verdict and every limitation live in [P1_LOGIC_STATUS.md](P1_LOGIC_STATUS.md), with durable raw excerpts under [evidence/p1/verification.txt](evidence/p1/verification.txt).

P1 now composes actual CAS/redb/SQLite, a signed directive verifier, bounded receipts/replay, native process ownership, workspace read tools and a real headless CLI. Windows hard-kill and Linux journal-throughput failures were fixed against failing regressions; earlier outputs remain retained. Tokio 1.54.0 was yanked during verification and was replaced with 1.54.1 in all resolution graphs. No advisory exemption was introduced.

**Full M1/P1 UI acceptance remains open:** CLI UI implementation was subsequently authorized; tested foundation slices and remaining gates are recorded in §E33, §E35 and UI_PHASES. Facades remain IN PROGRESS where later-phase children or the full execution bundle are open; their P1 exports and local profile are checked separately. P2–P9 source compiles in the workspace, but this does not mark those feature phases complete.

P0-SKEL entry is met. P0 overall still has manual ConPTY testing, account requests and dependency vetting open (the Slint licence is decided: royalty-free, ADR-014). PID-only orphan identities remain fail-closed for operator review. Full model/sandbox/S1 assembly, remote transports, live credentials and UI retain their own phase gates.

## P0 — tasks

| Task | Status | Evidence / notes |
|---|---|---|
| P0-SKEL-1 `.gitignore` anchored, Cargo.lock tracked | PASS | §E1; spike outputs (`bin/`, `out*/`, `research/*-src/`) added to the ignore list this session |
| P0-SKEL-2 `.gitattributes` | PASS | file present (LF default; CRLF for .ps1/.bat/.cmd) |
| P0-SKEL-3 `workspace.toml` (93 packages, 164 ext deps, 26 member-setup items) | PASS | §E2 historical inventory; current counts from E39 `workspace.py summary` |
| P0-SKEL-4 `workspace.py` tooling | PASS | §E2–§E8, §E13 (`gen` skips `implemented` crates; `render-docs`) |
| P0-SKEL-5 root `Cargo.toml` + `rust-toolchain.toml` generated | PASS | §E3 (`cargo metadata`: 93 packages) |
| P0-SKEL-6 doc-only skeleton compiles | PASS | §E4, re-run §E13 |
| P0-SKEL-7 member + layer checks | PASS | §E5 (Python) and §E9 (Rust xtask) |
| P0-SKEL-8 version audit + resolution probe | PASS | Historical §E6 (156/156 OK; lockfile resolved — 1,699 packages; see probe history). Full version audit of the expanded E39 graph is not rerun. |
| P0-SKEL-9 PLAN.md / PHASES.md / PHASES_STATUS.md; inventory 100 % covered | PASS | §E13 (`check-docs`, `render-docs --check`, `chatlog-inventory --check`) |
| P0-ACC-1 Token Factory API key | DESCOPED | Nebius services are unavailable to the team (Ukraine; decision 2026-10-29, D10); models via OpenRouter / local |
| P0-ACC-2 Token Factory Sandboxes (Contree) access | DESCOPED | Nebius services are unavailable to the team (Ukraine; decision 2026-10-29, D10); local Docker sandbox instead (E56) |
| P0-ACC-3 Nebius AI Cloud quota | DESCOPED | Nebius services are unavailable to the team (Ukraine; decision 2026-10-29, D10) |
| P0-ACC-4 Tavily API key | BLOCKED-ON-ACCESS | team action |
| P0-ENV-1 Ubuntu 24.04 in WSL2 | PASS | §E10, §E12: Ubuntu 24.04.5, kernel 6.6.87.2, user `dev` (sudo, kvm, docker), systemd; `/dev/kvm` present (S0-2); no `.wslconfig` needed |
| P0-ENV-2 Docker Engine + OpenShell v0.1.4 gateway in WSL2 | PASS | §E10 `verify-openshell.sh` (create 1.5 s, exec 112 ms, exit code, byte-exact upload, delete); driver pin in `setup-wsl.sh`; idle shutdown (W14) needs the keepalive |
| P0-ENV-3 tooling + setup scripts | PASS | §E12 `setup-windows.ps1 -Check` (all required present), install mode idempotent; WSL2 toolchain line in §E10. **Fresh-machine install paths not exercised** (everything was already present) |
| P0-ENV-4 `.cargo/config.toml`, longpaths, PROTOC | PASS | §E12/§E13: `CMAKE_POLICY_VERSION_MINIMUM=3.5` + `xtask` alias committed; `core.longpaths=true`; protoc on PATH; heavy deps build (openshell-sdk S0-1, LanceDB S0-9, ort S0-4); short target-dir not needed |
| P0-ENV-5 Linux check leg in WSL2 | PASS | §E10 `verify-linux-leg.sh` (check + doc `-Dwarnings`, fmt, 93 packages) + xtask tests on Linux |
| P0-ENV-6 local OpenAI-compatible server | PASS (endpoint) | §E10: Ollama 0.40.6 + `nemotron-mini`, SSE streaming OK (TTFT 2.3 s warm, 32.8 s cold). **Tool calling flaky: tool_calls in 1 of 4 runs** (the one success was in the previous session; its transcript is not in §E — the two runs in §E10 both show `tool_calls ABSENT`) — risk R22 for P3-X-1, evaluate a stronger small model when disk allows |
| P0-ENV-7 `deny.toml`, cargo-vet, Slint licence decision | IN PROGRESS | §E11 historical audit snapshot; §E29 current `cargo deny check` all ok, `cargo vet --locked` FAIL: 241 unvetted dependencies in the expanded graph. Existing exemptions are not audits; no new exemptions added. Slint licence DECIDED 2026-10-28: royalty-free (ADR-014); attribution via `AboutSlint`; `deny.toml` has per-crate exceptions for the 14 slint / i-slint-* crates only; `cargo deny check licenses` reports "licenses ok" (reported from the 2026-10-28 session; not re-run by this doc edit). The `cargo vet` gap above remains open. |
| P0-XT-1 Rust xtask port of checks | PASS | §E9 (test-first: red → green; real tree OK; negative test FAILs as expected; Windows + WSL2) |

## P0 — spikes

| Spike | Status | Result / fallback chosen (details: `@prime/spikes/<dir>/RESULT.md`) |
|---|---|---|
| S0-1 openshell-sdk on windows-msvc + WSL2 gateway | PASS | builds without protoc; mTLS channel built by Polaris + `OpenShellClient::from_parts`; dial `localhost`; gateway request limit 1 MiB; ~3 s cold start → connect retry |
| S0-2 nested KVM + vm driver | PASS | `/dev/kvm` present; vm driver boots Ubuntu (guest kernel 6.12.76) in 27.5 s; exec 216 ms; one driver per gateway |
| S0-3 NemoClaw custom blueprint | FAIL | → **native blueprint applier** (`polaris-openshell-nemoclaw`); agents are in-tree only; blueprint pins OpenShell 0.0.116 |
| S0-4 GLiNER2.5-Decide ONNX parity + latency | PASS (conditional) | 260/260 = 100 % top-1 (fp32, max Δp 4e-6) **only with the trailing-period rule** (93.85 % without); Rust fp32 p50 363 ms / p95 481 ms (1 task, 4 threads), 2.9× faster than Python; RSS peak 2.35 GiB (= 2.52 GB, borderline vs 2.5 GB); fp16 worse on CPU; CUDA EP not measured |
| S0-5 Token Factory capabilities | DESCOPED | Nebius services are unavailable to the team (Ukraine; decision 2026-10-29, D10) |
| S0-6 Needle 3 on Windows/WSL2 | PASS | Windows engine works: tool call p50 317 ms, embeddings 3,072-d p50 50 ms (Python/C only; `--serve` has no embed endpoint) |
| S0-7 workspace sync performance/fidelity | PASS | 10k files: no-op 1.1 s, initial 61 MiB in 10.4 s, 0 mismatches, CRLF + exec bit kept, deny-list holds; case-fidelity **not tested** |
| S0-8 iceoryx2 on Windows | PASS (condition) | RTT p50 2.0 µs / p99 4.7 µs; peer kill handled, dead-node cleanup OK; **security condition**: default root `C:\Temp\iceoryx2` is writable by all authenticated users → per-user `set_root_path` required |
| S0-9 LanceDB build on Windows | FAIL | build cost (36 min at -j4, 4.8 GB target, 249 MB exe; 0.40.0 needs `remote` feature + protoc) → **usearch + SQLite** default (verified: 1 min 41 s build, 4.3 MB, kNN p50 0.475 ms); LanceDB opt-in feature |
| S0-10 crossterm Shift+Enter on ConPTY | IN PROGRESS | probe built; **manual key test pending** (see RESULT.md for the steps) |
| S0-11 OpenShell mxc driver | FAIL | → **WSL2 tiers only** (no Windows gateway asset in v0.1.4; MXC "coming soon") |
| S0-12 guest build + handshake | PASS | guest built in WSL2, baked into image, handshake + mismatch refusal OK; headless Chromium screenshot OK (`--no-zygote`); Xvfb fails under default policy; `/opt` not readable/executable in sandboxes |
| S0-13 Contree MCP auth + branching | DESCOPED | Nebius services are unavailable to the team (Ukraine; decision 2026-10-29, D10) |

## Cross-cutting phase tasks

| Task | Status |
|---|---|
| P1-X-1 composition-root wiring v0 | PASS (§E30: actual LocalBackends, signed directive dispatch, read tools, durable stores, session typestate and shutdown) |
| P1-X-2 TUI shell | IN PROGRESS (§E33–§E36: real offline TUI, shared textarea/hover/selection, safe native reader, isolated demo and cold services; 350 scoped tests green, complete UI gates remain open) |
| P1-X-3 rustls aws-lc-rs provider in `main` | PASS (§E30: real main and subprocess provider smoke) |
| P2-X-1 Polaris sandbox image + blueprint | IN PROGRESS — final toolchain/guest image and live gateway/sync passed; remaining full audit in [P2 audit](P2_LOGIC_STATUS.md) |
| P2-X-2 `/onboarding` + `polaris doctor` | PASS (§E31: current JSON report covers all 26 inventory items/actions, required setup ready, credential values omitted; doctor and process probe tests pass) |
| P3-X-1 walking skeleton (local endpoint) | IN PROGRESS (§E45–§E46: production readonly tools and durable journal with synthetic-provider multi-turn evidence; complete live tokenizer/provider/edit/test/proof loop remains open) |
| P3-X-2 same loop on Nemotron | DESCOPED (Token Factory; Nemotron runs through OpenRouter instead, live web/TUI turns E53–E54) |
| P3-X-3 Contree sidecar + E3 run | CODE ONLY (E61: the MCP hub reaches a user-run `contree-mcp --mode http` sidecar; `polaris nebius contree-run` and `swe-run`; never run — Nebius unavailable, local Docker/OpenShell sandboxes E56/E60) |
| P3-X-4 live re-recording of SSE fixtures | DESCOPED (Nebius unavailable; fixtures stay labelled synthetic) |
| P4-X-1 ONNX export script + goldens | NOT STARTED |
| P4-X-2 threshold calibration v1 | IN PROGRESS (E60: `polaris s1-calibrate` on the 400-row synthetic set with the live worker; diff_tamper and secret_semantic calibrated at 100 % precision, congruence not reached; real-change calibration open) |
| P4-X-3 MVP scripted demo | NOT STARTED |
| P5-X-1 SearXNG container + provider | IN PROGRESS (E58: `/search-web` and `$websearch` over the redacting SearXNG adapter; loopback container script `@prime/tools/searxng/`; live query transcript open) |
| P5-X-2 Tavily provider live | BLOCKED-ON-ACCESS |
| P5-X-3 NVIDIA docs sync | IN PROGRESS (E58: incremental llms.txt sync into a local seed with per-page provenance, `/nv-docs-sync`, `/nv-docs`, `$nvidia(auto)` passages; a live run with page counts open) |
| P5-X-4 embeddings via Token Factory | CODE ONLY (E61: `/search-local` and `$rag` re-ranked by Token Factory embeddings of masked text when `embedding_model` is set; lexical BM25 stays the keyless path; never run) |
| P6-X-1 SWE env resolver + branching | CODE ONLY (E61: `swe-run` resolves the image with `resolve_environment` over the catalog and trusted annotations; `contree-run` branches up to 8 runs and `/contree-branches` shows the tree; never run) |
| P6-X-2 remote OpenShell gateway (E4) | CODE ONLY (E61: `polaris nebius cloud-up`, `cloud-status`, `cloud-down` through the `nebius` CLI with budget reservation, lifetime and owner label; the gateway stays on the VM's loopback behind an SSH tunnel; never run) |
| P6-X-3 Data Lab export + fine-tune trigger | CODE ONLY (E61: `/datalab-sync` collects proved trajectories, masks and exports them and uploads through the Files API; `/datalab-finetune` starts a job; never run) |
| P7-X-1 `/research` long run | IN PROGRESS (E58: `/research` composes the research loop with the model and a no-network Docker metric sandbox, checkpoints and `/research resume`; a live 20-iteration run with a kill open) |
| P7-X-2 `/hack` on seeded-bug repos | PASS (E55: 5/5 seeded races, blocking-in-async and CUDA barrier bugs, 0 false positives; synthetic repo) |
| P7-X-3 microVM browser loop | NOT STARTED |
| P9-X-1 static welcome page | IN PROGRESS (welcome page from the demo design in web and desktop; visual acceptance open) |
| P9-X-2 named-pipe transport + mTLS remote | IN PROGRESS (named-pipe attach with token auth, security tests pass — E53; `tcp://` attach with mutual TLS and daemon replies — E58; a deployed remote run open) |
| P10-1 full test matrix | IN PROGRESS (E57: workspace 2,630 passed / 4 known ratelimit failures; loom and load/stress spikes pass; fuzz, Miri, Kani, mutants need tools) |
| P10-2 security review | IN PROGRESS (E57: report, unsafe inventory, licence audit; E58: `cargo deny` passes after the pulldown-cmark fix and reasoned ignores, `cargo audit` 1 accepted advisory, `cargo vet` 1,167 unvetted; external review open) |
| P10-3 performance budgets | PASS (E57: all seven PLAN §11.4 budgets measured and recorded; five met, GCRA p99 1.1 µs and DFA 438 MB/s just short, S1 CPU p95 not met) |
| P10-4 `[profile.dist]` submission build | PASS (E57: dist build of five binaries + web site, packaged with README, LICENSE, SHA256SUMS and build-info) |
| P10-5 demo script + video | IN PROGRESS (E58: keyless script `@prime/DEMO.md` with `@prime/tools/demo/prepare.ps1` and fallback paths; the recording is open) |
| P10-6 Linux/macOS adaptation start | IN PROGRESS (E58: Linux `cargo check --workspace --all-targets --locked` passes in Docker after a peer-pid fix; Windows-only decoders no longer warn; Linux test run and macOS open) |

## Milestones

| Milestone | Status |
|---|---|
| M0 skeleton compiles, layers enforced, versions audited, probe green | PASS (§E3–§E6, §E9) |
| M1 TUI shell with sessions + directives | NOT STARTED |
| M2 first sandboxed `cargo test` from the TUI | Headless acceptance PASS: real 3 ordinary + 2 independent tests, exact review and zero unresolved resources; TUI explicitly deferred by user. Full P2 remains IN PROGRESS. |
| M3 walking skeleton (local endpoint) | IN PROGRESS — readonly dispatch slice verified in §E45; full milestone gate remains open |
| M4 same loop on Nemotron + Contree | DESCOPED (Nebius unavailable) |
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
| `crate:polaris-tests` | `polaris-tests` | P0 | IN PROGRESS | Cross-crate suites include actual EventLog-backed skill approval reopen on Windows and Linux; full system acceptance remains open. 2026-10-29 §E57: PLAN §11.4 budget suite (`budgets`, `budget_alloc` with dhat) measured in the `budget` profile. |
| `crate:xtask` | `xtask` | P0 | PASS | P0-XT-1: `check-members` + `check-layers` in Rust, test-first (11 tests, red then green; Windows + WSL2), real repo OK (§E9). §E43: deterministic SPDX 2.3 SBOM for the locked full Cargo graph with explicit timestamp, offline locked metadata, BLAKE3 lock digest, atomic no-clobber/force writes. The official schema checks `created` is a string, not RFC3339; the independent RFC3339 check accepts the real document (exit 0), rejects the impossible-date fixture (exit 1), and the official schema rejects the non-string fixture (exit 1). The original helper accepted the impossible date; that failure history is retained. Not a target-binary inventory, license validation, or security audit; advisories and cargo-vet remain failed. See evidence/continuation-20261026-e43/commands.json. |

### P1

<!-- generated: P1 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-cli` | `polaris-cli` | P1 | IN PROGRESS | UI foundation implementation 2026-10-24: real offline default TUI/dump, bounded signed directive admission/replay and session resume; focused controller/binary suites. See evidence/ui/verification.txt and UI_PHASES.md for open gates and doctor setup failure. Historical P1 non-UI evidence remains in E30. |
| `crate:polaris-sdk` | `polaris-sdk` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: validated contracts and enum forwarding; SDK Miri supported tests and Kani 3/3. §E43: two authorized SDK schema-test fixes; full SDK suite 56 passed, 0 failed, 0 ignored. Schema snapshots unchanged; see evidence/continuation-20261026-e43/commands.json. |
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
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | P1 | PASS | 2026-10-26 §E41: authenticated worker bootstrap is optional through the SDK trait; initialization and retirement are per generation. A generation is retired before slow cleanup, with an at-most-once cleanup guard, bounded startup/stop waits and fatal uncertain cleanup surfaced. The six focused supervisor tests pass. Timeouts do not establish native-work preemption. |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: ordered cancellation-safe teardown and durable journal; release checkpoint batching, emergency process guard, safe orphan handling; 1070-resource workloads on Windows/WSL. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | P1 | PASS | 2026-10-26 §E45 adds bounded authorized readonly dispatch, per-resource checks, bounded local-ignore preflight, caller output caps, bounded CAS reads and pre-open regular-file rejection. Native and Linux domain suites include FIFO/symlink regressions; external parent/global ignores are disabled for authorized calls and stable-workspace assumptions remain. |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: bounded JSON/text streams, real recorded cargo fixtures, actual CAS preservation, subprocess ledger contention and release throughput workloads. Final consistency/fuzz checkpoint recorded there. |
| `crate:polaris-tools` | `polaris-tools` | P1 | IN PROGRESS | P1 builtin/RTK exports and generated catalog consistency pass (§E30). Math child is P3; full child gate stays open. |
| `crate:polaris-kernel` | `polaris-kernel` | P1 | IN PROGRESS | 2026-10-27 E49: actual frame tools mutate, ask/resume, prepare host-approved sandbox tests and validate exact review/source proof evidence. Context-owned human/cancellation registries prevent duplicate terminal answers; restart and scoped cancellation are integrated. UI deferred, P2 otherwise paused, full Phase 3 acceptance open. |
| `crate:polaris-ui-model` | `polaris-ui-model` | P1 | IN PROGRESS | 2026-10-24 UI foundation: North Star/palette migration, transactional bounded composer, global session projection, cockpit and pure questionnaire state; native tests. See UI_PHASES.md and evidence/ui/verification.txt. Human backend protocol, native/formal/performance acceptance remain open; historical E21 retained. |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | P1 | IN PROGRESS | 2026-10-24 UI foundation: pure responsive renderer plus bounded input scheduler, staged Ratatui presenter and fallible terminal guard; 37 native tests. See UI_PHASES.md and evidence/ui/verification.txt; phase gates remain open. |
| `crate:polaris-ui` | `polaris-ui` | P1 | IN PROGRESS | 2026-10-29: facade re-exports model, session, cli, gui, web and ascii-aurora and documents the shared design tokens and UX rules (7fc0039); children track their own gates. |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | P1 | PASS | §E30 / P1_LOGIC_STATUS.md: 1M events with 1/8/64 readers, stalled subscriber, churn, bounded command accounting and RSS; Windows and WSL native suites. Final consistency/fuzz checkpoint recorded there. |

### P2

<!-- generated: P2 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | P2 | IN PROGRESS | main session, test-first; evidence pending |
| `crate:polaris-security-automata` | `polaris-security-automata` | P2 | IN PROGRESS | 5 descriptors; red then green: 27 tests (19-rule default pack with a positive+negative per rule, scan == regex-crate reference proptest, user-regex guard, integer fixed-point entropy, redactor idempotence, dhat zero-alloc scan/any_match/redact_into); 3 planted bugs caught (one after tightening a too-loose assertion) (§E18). Open: fuzz targets, Miri, load/scalability tables, bypass corpus, chunk-boundary policy |
| `crate:polaris-platform-wsl` | `polaris-platform-wsl` | P2 | IN PROGRESS | Real WSL discovery, bounded argv execution, doctor, path/config contracts implemented. Targeted Windows tests pass; full P2 gate remains open. |
| `crate:polaris-platform-process` | `polaris-platform-process` | P2 | IN PROGRESS | 2026-10-26 §E41: child bootstrap delivers its secret once over stdin and cleanup preserves pending or unconfirmed kill outcomes. Process cleanup does not establish endpoint ACL ownership or mutual server authentication; uncertain termination remains explicit. |
| `crate:polaris-platform` | `polaris-platform` | P2 | IN PROGRESS | Facade exposes process and WSL children; native process implementation pulled forward for P1 supervision. |
| `crate:polaris-vcs` | `polaris-vcs` | P2 | IN PROGRESS | Real gix status, bounded git fallback, persistent CAS undo and worktree lifecycle implemented; complete recovery and test audit pending. |
| `crate:polaris-openshell-client` | `polaris-openshell-client` | P2 | IN PROGRESS | Pinned real SDK adapter, explicit policy CID binding, mTLS, bounded transfer/exec and cancellation guards implemented; live acceptance pending. |
| `crate:polaris-openshell-policy` | `polaris-openshell-policy` | P2 | IN PROGRESS | Actual upstream pinned schema, policy validation, role/tier documents and provider YAML implemented; targeted contracts pass, live policy matrix pending. |
| `crate:polaris-openshell-nemoclaw` | `polaris-openshell-nemoclaw` | P2 | IN PROGRESS | Native versioned blueprint validation, canonical digest, actual backend apply and protocol handshake implemented; image/live evidence pending. |
| `crate:polaris-openshell` | `polaris-openshell` | P2 | IN PROGRESS | Actual client/policy/blueprint facades compile. 2026-10-30 E60: live on this machine (gateway 0.1.4, Docker driver in WSL): smoke test PASS and headless acceptance 4/4 (@prime/evidence/p2/openshell-live-acceptance-20261030.json). Policy matrix and fault gates remain open. |
| `crate:polaris-security` | `polaris-security` | P2 | IN PROGRESS | Facade reexports implemented children; child completion tracked separately. |
| `crate:polaris-runtime-leases` | `polaris-runtime-leases` | P2 | IN PROGRESS | 2026-10-23: authority, diagnostics, allocation and path contracts pass. Admission now preserves FIFO among conflicting scopes while allowing independent work; three Loom models exercise actual state transitions. Tokio wake delivery, ArcSwap publication and Kani remain separate open gates. |
| `crate:polaris-runtime-guards` | `polaris-runtime-guards` | P2 | IN PROGRESS | P2 proof regressions: 7 passed, including all Cargo groups, stderr binding, hidden earlier failures and malformed/overflow counts. Blast/AST and live/performance gates remain open; see @prime/P2_LOGIC_STATUS.md. |
| `crate:polaris-runtime` | `polaris-runtime` | P2 | IN PROGRESS | Runtime child facade exports written; guards and DAG logic UNVERIFIED, full frame/context/lease/research assembly remains pending. |
| `crate:polaris-spike-sandbox` | `polaris-spike-sandbox` | P2 | IN PROGRESS | doc-only skeleton (see P0-SKEL-6) |

### P3

<!-- generated: P3 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-ratelimit` | `polaris-ratelimit` | P3 | IN PROGRESS | E50 baseline plus active refund-ledger work: shared-snapshot replay, owned deferred settlement, and saturation-aware accounting implemented; focused behavioral, race, zero-allocation, performance, and formal acceptance checks remain open. Preserve the three contradictory reservation_refund fixtures unchanged. |
| `crate:polaris-security-capability` | `polaris-security-capability` | P3 | IN PROGRESS | 2026-10-26 §E45 adds explicit Workspace scope while preserving right/time/churn/unsafe/diff checks and monotone child attenuation. New frame-root/token binding and actual kernel readonly authorization have scoped native/Linux evidence; full mutation assembly and security audit remain open. |
| `crate:polaris-security-secrets` | `polaris-security-secrets` | P3 | IN PROGRESS | 2026-10-24 E32: bounded zeroizing AEAD vault, SDK source, identifier/storage limits and isolated real Windows keyring store/load/delete pass. Other host backends and full P3 assembly remain open. |
| `crate:polaris-nebius-cloud-token-factory` | `polaris-nebius-cloud-token-factory` | P3 | IN PROGRESS | E50: same-owner pinned llama.cpp token counting/chat adapter and local real token-usage parity checked; keyed Token Factory and full provider matrices remain open. E61: Files upload and fine-tuning jobs (create/get/list) per the OpenAI-compatible reference; never run (no Nebius access). |
| `crate:polaris-nebius-cloud-sandboxes` | `polaris-nebius-cloud-sandboxes` | P3 | IN PROGRESS | E48: stateless typed upstream Contree MCP adapter, exact 19-tool inventory, bounded branch/cancel and approved-path sync contracts. Actual MCP hub registry port preserves effect authorization. Live sidecar/SWE acceptance remains BLOCKED-ON-ACCESS. E61: reached from polaris-ui-session through the MCP hub over streamable HTTP (`polaris nebius contree-run`, `swe-run`, `/contree-branches`); never run. |
| `crate:polaris-nebius-cloud` | `polaris-nebius-cloud` | P3 | IN PROGRESS | 2026-10-26 §E41: facade exports the implemented Nebius AI Cloud surface. E61: polaris-ui-session composes all four services (Token Factory files/fine-tuning/embeddings, Contree over MCP, Data Lab export/upload, AI Cloud through the CLI); no live Nebius run exists. |
| `crate:polaris-models` | `polaris-models` | P3 | IN PROGRESS | E50: production local process loader, exact listener ownership, registry selection, leases through stream EOF and observed owned-tree shutdown checked; full resource/load/formal matrices remain open. |
| `crate:polaris-tools-math` | `polaris-tools-math` | P3 | IN PROGRESS | E50: compound exponent/depth guards, interpolation edge cases and exact NVIDIA citation hosts repaired; 33 focused tests and scoped lint pass; numerical corpus, fuzz/formal/performance acceptance remains open. |
| `crate:polaris-runtime-frames` | `polaris-runtime-frames` | P3 | IN PROGRESS | 2026-10-27 E49: sealed actual ancestry/step/result budgets integrate with the production kernel agent runner, durable questions and subtree cancellation. Native frame/domain tests pass; live M3/M4 and formal acceptance remain open. |
| `crate:polaris-runtime-context` | `polaris-runtime-context` | P3 | IN PROGRESS | 2026-10-27 E47 adds shared tokenizer-independent ConversationWindow validation and bounded verified lossless archive persistence. Kernel production assembly uses complete groups, actual masking, provider counting and capability-gated archive visibility. Real CAS and loopback integration pass; full context lifecycle/live-model acceptance remains open. |
| `crate:polaris-kernel-operators` | `polaris-kernel-operators` | P3 | IN PROGRESS | §E43: RateLimitOp uses the real dual RPM/TPM limiter with non-Clone admission input, retained permit, and usage reconciliation. Scoped evidence passes; production pipeline assembly and full phase acceptance remain open. See evidence/continuation-20261026-e43/commands.json. |
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | P3 | IN PROGRESS | 2026-10-27 E49: real mutation approval receipts, distinct human-answer and exact-target cancellation authorities, and call/result/current-source proof integration are exercised. Synthetic S1 and sandbox transports are labelled; full live, calibration, fuzz/formal and safety acceptance remain open. |
| `crate:polaris-spike-ratelimit` | `polaris-spike-ratelimit` | P3 | IN PROGRESS | 2026-10-27 E49: seven actual-limiter staged pressure, Retry-After, overrun, cancellation and observed-latency workloads pass. Concurrent fairness, full load/stress, unused-budget refunds and raw/live headers remain open. |

### P4

<!-- generated: P4 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-worker` | `polaris-worker` | P4 | IN PROGRESS | 2026-10-26 §E42: native owner drops idle models after a validated 1ms..1day interval (default 45s); explicit eviction acknowledges only after destruction. Installed-model RED/GREEN and authenticated child explicit/idle eviction plus real reload pass. This does not pause heavy-build admission, preempt inference, impose RSS limits, prove RSS reduction or complete nano/OCR/pressure policy. |
| `crate:polaris-system-one-engine` | `polaris-system-one-engine` | P4 | IN PROGRESS | E50: separate raw-byte PII span engine and model/runtime digests implemented; installed Decide inference checked. Live PII quality, full parity/calibration and CUDA gates remain open. |
| `crate:polaris-system-one-client` | `polaris-system-one-client` | P4 | IN PROGRESS | E50: bounded cross-call batching, exact request/checkpoint/generation/incarnation cache and single-flight implemented with cancellation/race fixtures; authenticated real Decide worker checks pass; complete workload/formal gates remain open. |
| `crate:polaris-security-pii` | `polaris-security-pii` | P4 | IN PROGRESS | E50: reversible NER spans, deterministic degradation, sealed marker-map snapshots and kernel private CAS/recovered-key integration checked; actual PII companion inference, corpus/performance/fuzz acceptance remains open. |
| `crate:polaris-security-rules` | `polaris-security-rules` | P4 | IN PROGRESS | Versioned AST/DFA rule packs, bounded diff/command audits, explicit partial-language coverage, fail-closed canonical S1 arbitration, finite same-block UAF indicator and caller-baseline manifest near-name audit implemented; flow-sensitive UAF/taint, broad symlink-race, registry-backed supply-chain and CUDA dataflow coverage remain open. Focused raw test and strict Clippy evidence: @prime/evidence/phase4/security-rules/. |
| `crate:polaris-system-one-schemas` | `polaris-system-one-schemas` | P4 | IN PROGRESS | E50: bounded exact-identity binary calibration metrics and xtask measured-candidate reports implemented; production thresholds remain uncalibrated pending real labelled held-out evidence. |
| `crate:polaris-system-one-router` | `polaris-system-one-router` | P4 | IN PROGRESS | E50: combined intent/effort/parallel/vision request, Medium constrained fallback, bounded input and hard directives checked; labelled ranking/calibration, environments and complete workload matrices remain open. |
| `crate:polaris-system-one-verifier` | `polaris-system-one-verifier` | P4 | IN PROGRESS | Production deterministic safety, preflight/postflight and merge arbitration in progress; UNVERIFIED, validation deferred by user. 2026-10-30 E60: the mutation gates read the changed lines (bounded to the encoder window) instead of whole files, which had exhausted the worker's memory. |
| `crate:polaris-system-one-watchdog` | `polaris-system-one-watchdog` | P4 | IN PROGRESS | Production bounded loop detection, advisory role/context guards and semantic drift inspection in progress; UNVERIFIED, validation deferred by user. |
| `crate:polaris-system-one` | `polaris-system-one` | P4 | IN PROGRESS | E50: explicit per-consumer failure policy, bounded admission, circuit and host epoch fencing checked; whole P4 parity/calibration/MVP acceptance remains open. |
| `crate:polaris-spike-s1` | `polaris-spike-s1` | P4 | IN PROGRESS | E50: real-engine burst dispatch, bounded owned tasks, sanitized per-request outcomes and observed latency quantiles implemented; real crash/load matrices remain open. |

### P5

<!-- generated: P5 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-vector` | `polaris-storage-vector` | P5 | IN PROGRESS | 2026-10-24 E32: real scoped usearch/SQLite mutations, pre-limit payload filtering, cross-handle consistency and recovery contracts; representative throughput and complete P5 acceptance remain open. |
| `crate:polaris-ingest` | `polaris-ingest` | P5 | IN PROGRESS | 2026-10-26 §E40: concrete format parsers, archive/XML/DOM preflight and source provenance, actual CAS-backed worker acceptance; OCR and process resource isolation remain open. |
| `crate:polaris-search-providers` | `polaris-search-providers` | P5 | IN PROGRESS | 2026-10-26 E39: concrete redacting Tavily/SearXNG, pinned DNS/redirect SSRF policy, bounded charset/DOM/Markdown extraction and cancellation-retained parser permits. Eighteen local contracts and strict Clippy pass. 2026-10-30 E58: SearXNG composed as `/search-web` and `$websearch` (`polaris_ui_session::web_search`) with a loopback container script; a live container run and broader fuzz/load remain open. |
| `crate:polaris-search-sift` | `polaris-search-sift` | P5 | IN PROGRESS | 2026-10-26 E39: bounded race/pools/cancellation, deduplicated S1 budget, concrete web bridge and durable hash-verified drop manifests with equal persistence/recovery caps. Nineteen contracts include a connected real-provider/local-HTTP/CAS pipeline; strict Clippy passes. Live source composition, calibration and broader workload gates remain open. |
| `crate:polaris-search` | `polaris-search` | P5 | IN PROGRESS | 2026-10-26 E39: exports real redacting providers and bounded race/sift APIs; query-to-source flow documented. Live service acceptance and kernel retrieval composition remain open. |
| `crate:polaris-nvidia-docs` | `polaris-nvidia-docs` | P5 | IN PROGRESS | 2026-10-29: workspace/prompt profiles, CUDA advisor rules, llms.txt/HTML ingestion, BM25 index and bounded fetcher; wired at the composition root through `polaris_ui_session::nvidia` (E54). 2026-10-30 E58: `sync` downloads the PLAN F20 llms.txt listings into a local seed (`~/.polaris/docs/nvidia/seed.jsonl` + per-page provenance in `state.json`), incrementally (304 keeps chunks, unlisted pages dropped); `/nv-docs-sync`, `/nv-docs` and `$nvidia(auto)` passages. Open: a live sync run, vector search (key), tree-sitter rule packs. |

### P6

<!-- generated: P6 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-nebius-cloud-data-lab` | `polaris-nebius-cloud-data-lab` | P6 | IN PROGRESS | Production local consent-gated trajectory masking/export in progress; UNVERIFIED. Remote API and fine-tuning remain pending API/access. E61: `/datalab-sync` drives collect → verified (event-bound proof resolver) → mask → export from the session log, then uploads through the Token Factory Files API; never run. |
| `crate:polaris-nebius-cloud-ai-cloud` | `polaris-nebius-cloud-ai-cloud` | P6 | IN PROGRESS | 2026-10-26 §E41: real checked budget reservations, terminal TTL handling, strict mTLS/IP ingress checks and owner-label selection. Provider transport is not implemented in this crate; E61: `polaris nebius cloud-up|cloud-status|cloud-down` apply this policy around the official `nebius` CLI (the AI Cloud API is gRPC only); never run. |

### P7

<!-- generated: P7 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-guest` | `polaris-guest` | P7 | IN PROGRESS | P2 protocol-version and locked conditional sync-cas helper implemented; Linux filesystem tests 6 passed plus host/Linux checks. Final image/live sync gate pending. Full structured exec/browser remains P7. |
| `crate:polaris-ext-mcp` | `polaris-ext-mcp` | P7 | IN PROGRESS | 2026-10-26 E39: real RMCP3.5.1 stdio/HTTP client, sandbox-only production launch, bounded protocol framing/body/SSE, schema-aware validation, lifecycle and trust/effect controls. Twenty-eight local contracts and strict all-target Clippy pass; live sandbox-backed MCP, full kernel registry/frame composition and wider lifecycle gates remain open. |
| `crate:polaris-ext-skills` | `polaris-ext-skills` | P7 | IN PROGRESS | 2026-10-23: capability-confined discovery, approval cache and canonical schema selection; Windows 22 and Linux 24 skills tests, durable redb approval reopen on both platforms. Full P7 audit remains open. |
| `crate:polaris-ext` | `polaris-ext` | P7 | IN PROGRESS | Skills exports and catalog are implemented. MCP child and full P7 integration remain open. |
| `crate:polaris-browser` | `polaris-browser` | P7 | IN PROGRESS | 2026-10-28: host side implemented (a11y outline, own DCT pHash visual diff, verified sandbox artifact session); the guest-side Chromium driver and the microVM loop wait for the sandbox image (P2-X-1). |
| `crate:polaris-hacker` | `polaris-hacker` | P7 | IN PROGRESS | 2026-10-26 §E40: bounded metadata-only AST/DFA observations and cancellation-owned sandbox stress. 2026-10-29 §E55: lexical race rules (split lock scopes, atomic load/store) and wider blocking-in-async rules, composed as `/hack` with the CUDA advisor. Sanitizer workloads and deeper race analysis remain open. |
| `crate:polaris-runtime-dag` | `polaris-runtime-dag` | P7 | IN PROGRESS | 2026-10-26 §E44: task-ID-correlated completion, explicit per-node interruptions without invented attempt counts, policy-aware dependency skipping and independent progress, preserved completed outcomes during cleanup, plus fail-fast/cancel/drop contracts. Post-implementation tests and strict scoped checks pass; actual panic execution, full scheduler interruption injection, snapshots, race/pool stress and production composition remain open. Interrupted tasks are never replayed automatically. |
| `crate:polaris-runtime-research` | `polaris-runtime-research` | P7 | IN PROGRESS | 2026-10-26 E39: strict program parser, OS-owned run lease, locked checkpoints/CAS evidence, exact-path capability attenuation and VCS receipts. Twenty local-process/Git iterations, receipt payload reopening, crash recovery and owner kill/reacquire are covered by 31 passing contracts. 2026-10-30 E58: composed as `/research` (model proposals, Docker metric sandbox, resume); `editable: **` no longer bypasses read-only scopes. Critique/skeptic step and broader crash/portability gates remain open. |

### P8

<!-- generated: P8 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-devenv-nix` | `polaris-devenv-nix` | P8 | IN PROGRESS | Nix JSON evaluation, filtered store paths, injected bounded sandbox execution/cache and prompt/static validation implemented. Full live Nix/store/policy/cache integration gates remain pending. |
| `crate:polaris-devenv-docker` | `polaris-devenv-docker` | P8 | IN PROGRESS | 2026-10-23: Docker suite 12 passed; real WSL Reaper journal reopen recovered exactly one UUID-owned pool and confirmed zero remaining. Pinned BuildKit frontend cross-RUN cache sentinel and built-image execution passed. Full P8 latency, Podman and remaining PLAN gates stay open. 2026-10-29 §E56: `DockerSandbox` (SDK SandboxBackend on Docker Desktop's named pipe) with a read-only workspace bind and named cache volumes; live `/ci-run` evidence. |
| `crate:polaris-devenv-cicd` | `polaris-devenv-cicd` | P8 | IN PROGRESS | 2026-10-26 §E41: bounded CI execution, owned cancellation cleanup and late cleanup after ambiguous create outcomes are implemented. Runtime abort or a never-resolving create can leave a bounded owned cleanup task retained without proving remote deletion. Live isolation, Dagger and persistent cache integration remain open; output redaction follows the output cap and is not a preemptive memory limit. 2026-10-29 §E56: cache mounts for backends that apply them and the cached Rust pipeline, run live in Docker (cold 5.1 s → warm 1.4 s). |
| `crate:polaris-devenv` | `polaris-devenv` | P8 | IN PROGRESS | Child module facade exports written. 2026-10-29 §E56: composed as `/ci` and `/ci-run` (cicd pipeline + Docker sandbox with cache volumes) in every frontend; Nix evaluation and Dagger remain open. |
| `crate:polaris-system-one-nano` | `polaris-system-one-nano` | P8 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P9

<!-- generated: P9 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-desktop` | `polaris-desktop` | P9 | IN PROGRESS | 2026-10-28: welcome + session view over a real offline kernel session (`bridge.rs`, lazy kernel start, `--state-dir`, model list from `[nebius.effort]`); 17 tests (args 5, bridge 8, models 4); live open 31 ms new / 13 ms resume. 2026-10-29: model replies with reviewed edits; `--connect/--session/--token-file` attaches to `polarisd` (E53). 2026-10-30 E59: the bridge reports the provider chain and the run's connection/storage to the Models and Settings pages and saves the reduced-motion preference; startup honours it. OPEN: attachments, speech input, message search indexing. Evidence: `@prime/evidence/ui/desktop/desktop-verification.txt` (being updated), live-1..4 PNGs. |
| `crate:polaris-web` | `polaris-web` | P9 | IN PROGRESS | 2026-10-29: Leptos SSR + hydration + PWA over the `polaris-ui-web` API (WEB_UI_PLAN §7); `--connect/--session/--token-file` attaches to `polarisd` (E53). |
| `crate:polaris-daemon` | `polaris-daemon` | P9 | IN PROGRESS | 2026-10-26 E39: real local kernel + authenticated RPC daemon and polarisd CLI, bounded endpoint/TLS configuration, readiness-before-drain and ordered shutdown. 2026-10-30 E58: `serve_daemon_with_replies` answers each typed message with a read-only chat turn (`--env-file`, `--no-replies`) and serves only the reply of a `FrameStepRecorded` transcript to attached clients. Default profile denies remote mutation/terminal; authority provisioning/revocation remain open. |
| `crate:polaris-rpc` | `polaris-rpc` | P9 | IN PROGRESS | 2026-10-24 E32: real Windows pipes, Unix sockets, TCP+mTLS and SDK client; auth/receipt/replay/revocation/shutdown contracts pass. 2026-10-30 E58: frontends reach a `tcp://` daemon through the same client (mutual TLS off loopback). Full P9 acceptance remains open. |
| `crate:polaris-ui-gui` | `polaris-ui-gui` | P9 | IN PROGRESS | 2026-10-28: welcome + session view, composer v2 and backend request/event contract; 36 tests (composer 10, window 8, controller 13, tokens 4, render 1); session view idle CPU 1.39% of one core vs ~19-28% animated welcome. 2026-10-30 E59: Models (model per effort, provider chain with skip reasons, setup), Usage (session tiles, activity) and Settings (reduced-motion switch saved to ui.cli.reduce_motion and applied live, connection, storage paths, keyboard, About + Slint attribution) pages over the content; not rendered or tested yet. OPEN: attachments, speech input. |
| `crate:polaris-ui-ascii-aurora` | `polaris-ui-ascii-aurora` | P9 | IN PROGRESS | 2026-10-28: ASCII aurora backs the welcome screen and pauses in the session view (ADR-016); daemon `--connect` attach done 2026-10-29 (E53). |
| `crate:polaris-ui-session` | `polaris-ui-session` | P9 | IN PROGRESS | 2026-10-28: shared offline session host extracted from polaris-cli; typed user input routing and kernel-free session queries. 2026-10-29: composition root of the local commands and context — RemoteSession attach (E53), `$nvidia(...)`/`/nv-profile`/`/nv-verify` (E54), `/hack` (E55), `/ci`/`/ci-run` (E56), keyless `/search-local` and `$rag` (E57). 2026-10-30 E58: daemon `responder`, attached replies, `tcp://` attach with mutual TLS, `/nv-docs-sync`/`/nv-docs`, `/search-web`/`$websearch`, `/diff`/`/undo`. E60: every session kernel starts the local System One worker (`system_one`, `s1-calibrate`) and, with `~/.polaris/sandbox.json`, the OpenShell sandbox (`sandbox`, model `run_tests` with review). Message history is not indexed yet. |
| `crate:polaris-ui-web` | `polaris-ui-web` | P9 | IN PROGRESS | 2026-10-29: loopback web UI (token + host checks, CSP), SSE events, chat with reviewed edits; attached mode over `RemoteSession` (`WebServer::bind_with`, E53); 2026-10-30 E58: daemon replies shown in attached mode. |
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

## E31 — P2-X-2 headless doctor/onboarding (2026-10-23)

`@prime/evidence/p2/doctor-headless-current.json` is the current `polaris doctor --json` output.
The command exited 0 with all 26 authoritative `member_setup` entries represented, all 26 concrete
onboarding actions present, `ready=true`, and `required_missing=0`. Required Windows MSVC, Python uv,
and WSL OpenShell checks pass. The report checks credential presence only; Token Factory and Tavily
references are missing and no secret payload is emitted. Seven unresolved observations are optional
later-phase items and do not block P0 setup readiness. The readiness contract tests and bounded process
probe tests are recorded in `@prime/evidence/p2/doctor-cli-tests.txt` and
`@prime/evidence/p2/process-probe-tests.txt`.

The first current-CLI report captured an OpenShell `BadSignature` peer-certificate error. Two following
strict-path `openshell status` calls and the final doctor report passed with authenticated mTLS and
version 0.1.4; the cause of the initial transient failure is unknown. The retained reports and repeated
status output are in `@prime/evidence/p2/doctor-headless-transient-badsig.json`,
`@prime/evidence/p2/doctor-headless-intermediate-probe-fixes.json`, and
`@prime/evidence/p2/openshell-status-repeat-current.txt`.

## E32 — non-UI P3/P5/P9 continuation (2026-10-24)

VERIFIED scope: the user explicitly paused Phase 2 work and deferred UI. This continuation implements
P3 secret custody and bounded frame execution, P5 default vector storage, and P9 authenticated RPC.
None of those full phases is declared PASS. The existing P1 acceptance is historical and scoped.

- **P3 secrets:** actual Windows Credential Manager/read-only environment SDK source; validated source
  locations; zeroizing ChaCha20-Poly1305 RAM vault with independent key/ciphertext allocations,
  entry/aggregate-byte bounds, replacement accounting, tamper rejection, and nonprintable sealing key.
  A random test-owned synthetic Windows credential completed store/load/delete. No existing user
  credentials were read. Config references now convert into the validated SDK location, and Token
  Factory resolves exactly that injected source with distinct missing/provider/invalid errors.
- **P3 frames:** bounded multi-turn assemble/model/tool/verifier driver; attenuated role/quota checks,
  conservative token reservations, exact call/result links, CID checks, retry/tool-round/cancellation
  limits, partial assistant capture, clean-EOF/usage lifecycle checks, bounded full prompt serialization and versioned CAS transcript-before-event journal (22 focused tests). The real
  CAS/EventLog integration appends, closes, reopens, and reads the transcript. Completion claims remain
  unproven until independent proof-of-done; this is not the production kernel edit/test/proof assembly.
- **P5 vectors:** actual immutable table/model/dimension/metric and session/global scope; SQLite WAL
  authoritative rows; staged HNSW mutation before SQL commit, generation refresh across handles,
  exact payload filtering before K, and recoverable index cache. Parallel native graph insertion
  failed the original exact-self-neighbor property and was removed; sequential graph insertion
  passes it. Parallel work is confined to bounded byte serialization. The representative 100k × 768
  debug run took 506.315803 seconds (198 vectors/s); release took 568.1559863 seconds (176 vectors/s). Both measured runs miss the 100k/min target. They ran during concurrent development; an isolated performance calibration is still open.
  No optional LanceDB implementation, complete recall study, or whole P5 pipeline is claimed.
- **P9 RPC:** real local Windows pipes (next instance created before handoff), private Unix sockets,
  TCP with mandatory mTLS off loopback, bounded connections/streams/shutdown, and SDK client.
  Every call carries a capability; the client refuses credential mismatch and missing/mismatched
  receipts. The additive optional acceptance-sequence wire field retains the kernel receipt exactly.
  Real network tests exercise missing/malformed/duplicate/expired/wrong-issuer/insufficient-right
  credentials, idle-stream revocation, required client TLS identity and server name, inclusive replay
  from sequence zero, foreign-session isolation, and shutdown. Windows covers 100 pipe cycles;
  Linux covers UDS ownership and path cleanup. Transport tests inject a fixture KernelApi; they do
  not establish real daemon command integration. Only the four published RPCs are implemented.

VERIFIED commands and raw results are retained in [the continuation evidence](evidence/continuation/README.md).
The percentage report is [PHASES_STATUS_PERCENTAGE.md](PHASES_STATUS_PERCENTAGE.md); every percentage is
an engineering estimate, not a release/readiness guarantee.

Open consistency gate: additive frame events require durable event schema 4, and older event kinds
remain valid at that schema. The existing SDK `blast_alert_requires_schema_three` test hardcodes that
only schema 3 is legal. Its assertion remains unchanged; that full SDK test gate is RED. Current-schema
SDK and proto conversion contracts pass. Dependency-wide Clippy/audit debt, live keyed providers,
production daemon and full local S2 integration, wider crash/formal/fuzz/performance gates remain open.
UI-model changes are explicit neutral event-match arms needed to keep the expanded schema compiling;
no rendering or interactive UI implementation was added. Phase 2 remains paused.

Additional VERIFIED open gate at the final checkpoint: `cargo xtask check-witness` reports one violation at `crates/polaris-ext/crates/skills/src/approval_cache.rs:197`, where an existing unit-test fixture calls `ApprovalWitness::new` outside SDK/verifier. The checker and fixture were not weakened or bypassed. Raw output: [consistency.txt](evidence/continuation/consistency.txt). This is not a clean whole-system source-policy verdict.


## E33 — CLI UI foundation (2026-10-24 to 2026-10-25)

Implementation of UI_PLAN is authorized and in progress. The CLI model, responsive renderer, bounded terminal runtime and real offline application are present. Pure questionnaire state includes explicit None of the above, but the P3 human service/protocol is not implemented. No complete UI phase or M1 PASS is claimed.

Evidence and scope: [verification index](evidence/ui/verification.txt), [contract and resource audit](evidence/ui/contract-audit.md), [UI gates](UI_PHASES.md), [module inventory](UI_MODULES.csv). Historical §E21/§E30 results are retained. Current doctor readiness and advisory-cache failures are recorded separately; earlier acceptance is not a claim that this managed environment satisfies setup today.

E33 final checkpoint: the four-package offline command reported 266 passed, 1 failed and 1 ignored. The only failure is the current doctor readiness assertion for missing environment prerequisites. Selected owned-target safety Clippy, all-target UI/application API check and formatting passed. Configured advisory audit could not lock its read-only cache; the separately identified licence/bans/sources subset passed. Exact commands and raw files are in the linked verification index. Full UI gates remain open.


## E34 — CLI reference-design repair (2026-10-25)

**VERIFIED focused repair:** corrected the live-editor surface misunderstanding, actual demo geometry/palette, adaptive wrapped input, neutral conversation and opaque overlays. Cell/color comparison outside the actual-state footer matches the saved demo welcome at five sizes. Footer clicks, quiet startup, concise rejection and /clear are covered.

The focused offline repair run passed **92 tests**, including nine real native PTY scenarios and one VT-parser regression. Selected owned-target safety Clippy passed. [Exact commands, raw output, visual exports, source-to-assertion mapping and limitations](evidence/ui/repair-verification.md). This is separate from E33’s full-package doctor failure and is not full UI/M1 acceptance. The GUI browser was unavailable; visual inspection used rasterized actual Ratatui buffers, not a physical-host screenshot. Runtime model execution, live human questions and the other declared UI gates remain incomplete.


## E35 — CLI cold services and UI preferences (2026-10-25)

**VERIFIED scoped continuation:** added typed `[ui.cli]` preferences in `polaris-config`, an explicit one-field global atomic save API, CLI settings state with memory-only runtime/demo overrides, a bounded settings-save worker, text-only platform clipboard worker and bounded workspace path snapshot worker. UI service paths delegate workspace ignore, secret-path, symlink/reparse and containment policy to `polaris-tools-builtin`; tests do not access or mutate the user's clipboard. No UI phase or M1 acceptance is claimed.

**VERIFIED current commands:** `cargo test --offline -p polaris-config` passed all package tests and doctests; `cargo test --offline -p polaris-cli --lib` passed 45/45; selected all-target safety Clippy passed for config/model/UI CLI/app. [Continuation evidence index](evidence/ui/continuation-services-verification.md) links the exact raw stdout/stderr and command records. Generic Clippy style warnings remain.

**Red evidence gap:** the service descriptor predates implementation, but a service-specific meaningful red transcript was not retained. No red is claimed. The full native clipboard operation, Windows/Linux/macOS matrix, path junction/case/deleted-suggestion cases, settings save retry/restart/malformed-file screen flows, visual QA and cold-path allocation profile remain unverified. Live human-request/backend protocol is still not implemented; WEB-OPT and desktop remain deferred.

## E36 — CLI editor, pointer, safe native input and isolated demo (2026-10-25)

**VERIFIED scoped continuation:** one Unicode geometry now drives textarea paint/caret/click/selection/vertical navigation; Shift+Enter inserts newline, Ctrl+Enter is unbound, and effort changes preserve the physical editor caret after repaint. Typed clipped hit maps add hover, footer/completion/tool interaction, editor drag and canonical single-entry transcript selection. The safe public Windows console wrapper preserves key modifiers/Alt-code characters with bounded CSI/paste processing; oversized paste is discarded through its terminator. Reader teardown restores its mode snapshot before terminal teardown.

The explicit `--demo`/footer profile owns separate drafts/transcript, labelled scripted streaming/cancel/reset/replay and three-question None/notes/atomic-submit behavior. It calls no KernelApi/provider and does not open durable offline state or save preferences. Cold services are integrated with generation/revision-scoped paste and explicit off-thread per-field saves; user clipboard contents are not touched by tests.

**VERIFIED commands:** config/model/renderer package tests passed **263**; selected app targets passed **87** (58 library, 2 RTK, 3 trace, 3 session, 6 startup, 15 terminal). The final self-contained ConPTY target passed **15/15** again; this repeat is not added to the 350 total. Five selected owned-target safety lints, formatting and dependency-policy subset passed. [Exact commands, raw output, meaningful red records and visual exports](evidence/ui/continuation-verification.md); [machine index](evidence/ui/continuation-verification.json).

**Open verdict:** separate headless doctor readiness still fails on missing setup prerequisites (2 passed, 1 failed); advisory audit cannot acquire its read-only cache lock. Native clipboard success, physical-terminal/Linux matrix, cross-entry selection/word drag, full interactive tool/diff demo cards, full settings error/restart flows, real human/model backend integration and measured cache/allocation/performance/release gates remain open. Earlier broad/service initial-red gaps are retained. Web welcome/WEB-OPT and desktop remain deferred. No complete UI phase, M1 or whole-workspace PASS is claimed.


## E37 — CLI reference parity, complete local Demo catalog and bounded layout cache (2026-10-25)

**VERIFIED scoped implementation:** typed local commands open reference-derived bottom sheets; all 27 original preview names plus questions/editor are accepted by the real binary. The compact questionnaire replaces the composer, keeps no defaults and explicit exclusive None, edits Unicode answers/notes with shared geometry, and requires Submit review. Native input handles interleaved key releases, DEL and right/middle mouse release identity; rich transcript styling, multi-entry semantic selection, presentation-only tools and a retained bounded cache are connected to the app. Fixtures remain labelled Demo and grant no P2/P3 authority.

**VERIFIED commands:** config/model/renderer package tests: 297 passed, 0 failed, 1 opt-in benchmark ignored; selected application targets: 129 passed, 0 failed (95 library, 2 RTK, 4 trace, 3 session, 7 startup, 18 terminal). The separate benchmark was explicitly executed in debug and release, not added to this count. Selected five owned-target safety Clippy and formatting passed. [Exact commands, raw output, source mapping, regression REDs, visual exports and limits](evidence/ui/parity-verification.md).

**Open verdict:** the separate current headless target is 2 passed, 1 doctor readiness failure on missing setup prerequisites. The release warm CPU microcase has p50 3.888 ms / max 4.805 ms at 200×60; debug/cold paths exceed 8 ms, so full UI6 acceptance stays open. ConPTY consumes mouse DECSET controls in this environment; exact writer bytes are checked directly and physical Alacritty remains UNVERIFIED. Timed agent/independent child transcripts, full overlay parity, OS clipboard/images, settings error/restart flows, complete paging/performance, real human/provider protocols and formal/Linux/release gates remain open. No full UI/M1 or whole-workspace PASS; web/WEB-OPT/desktop remain deferred. Existing state changes and earlier evidence are preserved.

## E38 — CLI mouse delivery, Demo activation and source-only editor selection (2026-10-25)

**VERIFIED scoped repair:** the native Windows input owner enables mouse/window/extended/VT input and disables Quick Edit through the existing safe adapter. Explicit fallible close restores its snapshot, disarms successful restoration and leaves failed restoration retryable; terminal teardown restores the original full mask. A child ConPTY readback observed initial `0x1f7`, raw `0x1f0`, native `0x3b8`, after input close `0x1f0`, final `0x1f7`. A single profile-row click activates Demo via the same guarded path as Enter.

Shared gesture ownership now handles character drag, double-click word/word drag and triple-click logical line across composer, transcript and question editors. Only source cells are painted; decorative padding cannot begin word/line selection. Mouse release preserves editable ranges for Delete/Backspace, typing, paste and Undo. Diagnostic mouse traces contain categories/buttons/coordinates/modifiers only.

**VERIFIED commands:** current model/CLI-renderer package tests passed **277**, 0 failed, 1 opt-in CPU benchmark ignored; selected app targets passed **145**, 0 failed (108 library, 2 RTK, 5 trace, 3 session, 7 startup, 20 native terminal). The **422** total excludes config and broader workspace targets, repeats and the separate renderer captures. Five selected owned-target safety Clippy rules passed. [Exact commands, raw outputs, RED regressions, four actual-buffer visual exports and limits](evidence/ui/mouse-repair-verification.md).

**Open verdict:** synthetic ConPTY mouse/input and actual OS-mode readback do not verify physical Alacritty keyboard/mouse operation. OS clipboard success, Linux, full strict Clippy/formal/performance/release and full UI/M1 acceptance remain open; prior doctor readiness/advisory-cache failures are unchanged and not rerun in this slice. P2/P3/provider/human authority, web welcome/WEB-OPT and desktop are outside this repair. Earlier evidence and pre-existing local state remain preserved.

## E39 — Non-UI retrieval, extension, research and daemon continuation (2026-10-26)

**VERIFIED scoped implementation:** concrete redacting Tavily/SearXNG adapters and DNS-pinned,
bounded fetch/extraction; named-pool source races with cancellation, deduplicated S1 budgets and
accurate task outcomes; equal-cap persisted/recovered dropped-chunk manifests; real RMCP 3.5.1
stdio/HTTP clients with bounded framing and schema-aware argument checks. MCP production stdio
uses the new SDK/OpenShell dedicated-sandbox duplex resource boundary. The local RMCP subprocess
fixture is test-only, and the live configured gateway was unavailable.

Research now parses immutable programs, narrows tool capabilities to snapshotted paths, stores
actual tool/metric/VCS receipt payloads in CAS, and serializes durable checkpoint/ledger writes.
The 20-iteration local-process/Git fixture reopens all 60 evidence payloads and recovers one abrupt
exit. A lifetime OS run lease prevents two orchestration instances; fresh initialization refuses
an existing checkpoint, and child-process termination releases the lease. These local process
fixtures do not prove sandbox isolation or live model behavior.

The new `polarisd` composes the real local kernel and authenticated RPC service, validates bounded
listener/TLS configuration, closes readiness before draining and propagates signal/log-flush
failures. Its default P1 profile supports health and event replay and explicitly denies remote
mutation/terminal operations. Full remote agent composition, authority provisioning/revocation,
single-instance deployment and live remote mTLS acceptance remain open.

**Current evidence:** the scoped command run records 157 passing tests, with exact argv,
exit codes and raw output in [commands.json](evidence/continuation-20261026/commands.json).
See the [evidence index and limitations](evidence/continuation-20261026/README.md). The SDK's full
suite still has two unchanged schema-boundary assertion failures. The source-policy witness gate
is green after moving approval-cache coverage to verifier tests using authentic issued witnesses;
no checker or cache assertion was weakened. The dependency advisory gate still fails on four
unmaintained crates after the ctutils 0.4.4 vulnerability fix, and cargo-vet retains 724 unvetted
dependencies. These are open release gates, not phase acceptance.

**Consistency:** central dependency ownership, module exports, descriptors and generated phase
tables are updated. The percentage report refreshes only this continuation's affected rows;
other requirement estimates retain their earlier audit basis. No UI implementation is changed
by this continuation. Phase 2's broader completion work remains paused; its stdio seam is needed
for P7 MCP. All full-system and complete-phase acceptance claims remain open.

## E40 — Non-UI native inference, authenticated workers, ingestion and deterministic auditing (2026-10-26)

**VERIFIED scoped implementation:** explicit local CPU/fp32 GLiNER/ORT inference, authenticated
SDK DecisionEngine IPC and real s1/ingest worker modes. Fixed-format parent credentials are read
from inherited stdin, checked on every RPC and removed before handlers run. Windows pipe
authentication/collision/teardown tests and a real child/public-client inference path pass. The
native model stays on its owner thread; canceled waiters cannot release its admission permit.

Concrete text/Markdown/HTML/CSV/TSV/OOXML/PDF parsing now retains available source provenance.
Archive/XML/DOM/sparse-sheet preflight, output caps and hash-verified CAS-backed ingestion are
exercised. The worker stores an explicitly untrusted provenance record and exact Markdown CIDs.
Ingest drain regressions failed before the fix and pass afterwards. OCR remains unconfigured;
synchronous library work still requires OS process memory/time isolation.

The deterministic audit path reports bounded AST/DFA observations without copying source into
evidence, generates bounded boundary inputs and owns SDK sandbox cleanup across caller
cancellation and execution timeout. Actual sanitizer/isolation acceptance is not implied by the
local SDK fixture. Quadratic/race/CUDA analysis remains unfinished.

**Current evidence:** 119 scoped tests pass; two opt-in artifact tests separately pass one
real input each. The all-target workspace check and six-package safety Clippy pass. Exact argv,
statuses, raw output and local artifact hashes are in [commands.json](evidence/continuation-20261026-e40/commands.json)
and the [evidence index](evidence/continuation-20261026-e40/README.md). The full SDK suite retains
two unchanged schema-boundary failures. Dependency audit failures remain open, with their exact
current output preserved. No test assertion, checker or audit exemption was weakened.

**Dependency remediation:** the initial ZIP8 advisory failures are retained in
[initial-zip8](evidence/continuation-20261026-e40/initial-zip8/README.md). The final graph uses
ZIP 9.1.1 through a minimal published-source DOCX backport. Source/license provenance and
adversarial tamper checks pass; image preview decoding is disabled. NUL member names are
rejected after a compiled behavioral RED. Existing unmaintained-crate and cargo-vet gates
remain failed, without exemptions. Worker shutdown cancellation, drain-lock contention,
busy-pipe retries and bounded runtime waiting have retained regression evidence. A timeout
does not prove preemption or resource release for native/blocking work.

**Consistency and remaining scope:** root dependency ownership, actual API descriptions, phase
tables and the affected percentage rows are updated. SDK hex decoding uses checked arithmetic
with an exhaustive 256-byte test; generated tonic status casts use equivalent From conversions
and a build-time drift guard. Schema snapshots remain unchanged. Automatic supervisor credential
provisioning is not wired because SpawnSpec has no bootstrap-bytes API. Bearer auth is not mutual
server authentication or an OS ACL proof. Full parity/calibration, PII-NER/CUDA, OCR/nano, resource
policy, production kernel assembly and full phase acceptance remain open. No UI source changed
in this continuation; broader P2 completion remains paused.

## E41 — Worker lifecycle, cloud contracts and bounded CI cleanup (2026-10-26)

**VERIFIED scoped implementation:** SDK optional-trait bootstrap provisions authenticated worker
startup; worker secrets arrive once through stdin and cleanup preserves pending or unconfirmed-kill
truth. The supervisor owns initialization and retirement per generation, bounds startup and reports
fatal uncertain cleanup. S1WorkerLifecycle readiness and generation fencing are exercised through
the actual worker integration. This supersedes E40's earlier note that automatic supervisor
credential provisioning was not wired.

P6 AI Cloud adds checked budget reservations, terminal TTL handling, strict mTLS/IP ingress checks
and owner-label selection. The Nebius facade exports these APIs. No provider transport or live cloud
request is claimed. P8 CI adds execution caps, owned cancellation cleanup and late cleanup after
ambiguous create results; live isolation, Dagger and persistent caching remain open.

**Current evidence:** 254 scoped tests pass. Exact command arguments, exit codes and raw outputs
are indexed in [commands.json](evidence/continuation-20261026-e41/commands.json). The SDK full suite
retains 54 passing tests and two schema-boundary failures; advisory and cargo-vet gates remain failed
with six unmaintained-crate findings and 804 unvetted dependencies.
The helper preserves those outcomes and does not convert them into acceptance. Workspace, member,
layer and witness checks are required to pass before this checkpoint can be recorded. The two
supervisor behavioral REDs (`retirement-order-red` and `bounded-stop-red`) precede six passing
supervisor tests; the API compile RED is compile evidence, not a behavioral regression. Scoped
all-targets Clippy covers seven packages, with separate checks for supervisor production/bootstrap,
kernel production library and SDK production library. CI review baseline comparisons were captured
after implementation, so they are not described as historical test-first evidence.

**Boundaries:** a bounded startup or cleanup wait does not prove native work stopped or resources
were released; pending and unconfirmed kill states remain explicit. The provider transport and live
AI Cloud behavior are absent. CI does not yet prove live isolation, Dagger execution or cache behavior.
No full phase acceptance is implied. The one-input real-model check is not parity or calibration.
Bearer authentication does not prove mutual server authentication or server endpoint ACL ownership.
CI runtime abort and a create that never resolves can retain a bounded cleanup task without proving
remote deletion; output redaction runs after the output cap and does not provide a preemptive memory
limit. The process package retains its prior 90% code / 80% verified estimate; broader P2 work remains paused;
historical P1 verdicts remain unchanged. Central dependency ownership was updated for the E41
bootstrap/client/process work, with 167 external dependencies and no new global external alias. No
UI source changed.

## E42 — Native model idle eviction, real reload and model lifecycle contracts (2026-10-26)

**VERIFIED scoped implementation:** the native owner enforces monotonic idle eviction (default
45 seconds, validated 1ms..1day configuration), then lazily reloads the actual model for later work.
Explicit eviction acknowledges completed handle destruction. The kernel-side client authenticates
and bounds eviction calls and validates the single-engine acknowledgement. A real 46-second idle
regression failed before the change and passes afterwards; actual authenticated Windows children
pass explicit/idle eviction followed by real inference reload. No RSS reduction is inferred.

**Evidence:** 160 scoped tests pass, with six ignored artifact/platform cases; four installed-artifact
cases are separately exercised (one idle owner, two child reload cases, one supervised inference).
Twenty existing catalog/lease/manager contract cases pass on Windows and Linux. Workspace all-target
compilation, three-package all-target safety Clippy, member/layer/witness and format checks pass.
[Exact commands and outcomes](evidence/continuation-20261026-e42/commands.json) and
[scope/evidence index](evidence/continuation-20261026-e42/README.md) retain the limitations.
The broader offline Linux worker attempt cannot compile because its cache lacks aead 0.6.1.
The two unchanged SDK schema-boundary failures and dependency audit failures remain open.

**Remaining scope:** this does not supply heavy-build admission exclusion, RSS enforcement,
native-work preemption, production model-slot assembly, checkpoint-bound caching, batching,
PII-NER/CUDA/calibration, nano/OCR, full stress/Loom/formal evidence or complete phase acceptance.
P1 historical acceptance is retained; broader P2 work remains paused and UI remains deferred.
Subsequent phase work has its own evidence; this checkpoint does not certify those later edits.

## E43 — Lossless context records, dual rate-limit admission, and locked-graph SBOM (2026-10-26)

**VERIFIED scoped results:** context assembly stores complete omitted message groups in CAS, applies a hard
byte cap, and provides bounded expansion with an archive recovery notice. `RateLimitOp` uses the shared
dual RPM/TPM limiter; its admission input is non-Clone, and its permit follows admitted work
through dispatch and final-usage reconciliation. Neither feature is composed into the production
root pipeline. The SDK schema-test changes cover two authorized test files; the SDK full suite
reports 56 passed, 0 failed, 0 ignored, with the production schema version unchanged.

`cargo xtask release-sbom` generated a deterministic SPDX 2.3 document for the full locked Cargo
graph using offline metadata and an explicit timestamp. The SBOM records the BLAKE3 lock digest;
same-timestamp output was byte-identical. The official SPDX schema checks only that
`creationInfo.created` is a string, not RFC3339. The initial helper accepted an impossible date;
that validation miss remains in the history. An independent RFC3339 check accepts the real
document (exit 0) and rejects the impossible-date fixture (exit 1); official schema validation
rejects a non-string `created` value (exit 1). This is not a target-binary inventory, license
validation, or security audit.

**Evidence:** member, layer, witness, all-target workspace compile, scoped tests, SDK full suite,
scoped safety Clippy, SDK production/schema-test Clippy, scoped formatting, repeat SBOM generation,
and SBOM schema validation exited 0. The scoped five-package suite reports 155 passed, 0 failed,
2 ignored. Exact argv, exit codes, and raw outputs are indexed in
[commands.json](evidence/continuation-20261026-e43/commands.json). The advisory gate remains failed
with six unmaintained-crate findings; `cargo vet --locked` remains failed with 804 unvetted
dependencies. The SBOM does not close either gate.

**Remaining scope:** full-system and complete-phase acceptance remain open. Broader P2 completion
remains paused and UI remains deferred. The separate percentage report refreshes only the recorded
E42/E43 component estimates and partial P10 inventory task; no phase acceptance is claimed.

## E44 — DAG task identity and policy-aware interruption (2026-10-26)

**VERIFIED scoped implementation:** Tokio task identities are tracked from spawn through join.
Normal completions must match their tracked graph index, node ID and pool; unknown/mismatched
identities fail closed. A join interruption records a terminal per-node Aborted/Panicked category
without exposing panic text or inventing an attempt count. ContinueIndependent skips failed
descendants and preserves independent work; FailFast aborts/drains remaining owned tasks.
Interrupted tasks are not automatically replayed under Retry. Caller cancellation remains distinct.
Cleanup preserves completed, previously unconsumed outcomes rather than overwriting them as Cancelled.
The order_started accessor records scheduler dispatch order, not observed backend entry order.

**Evidence:** 92 scoped tests pass, with 2 ignored. The current
workspace all-target check, member/layer/witness checks, two-package all-target safety Clippy and
format checks pass. Real Tokio aborts cover normalization and the shared report transition;
public scheduler cases cover ordinary failure policy, diamond ordering and cancellation/drop.
The standalone Linux DAG suite separately reports 12 passed, 0 failed. Initial dependency-artifact
build failures and capture/launcher failures are retained in the checkpoint's review history.
[Exact commands and outputs](evidence/continuation-20261026-e44/commands.json).
The release-inventory test now reads a JSON field with get() rather than indexing; its assertion
retains the same dependency-edge requirement. No production SBOM logic changed in E44.

**Process and acceptance limits:** the descriptor was publicly emitted before the implementation,
but its file and initial tests were written after source edits. No blanket historical test-first
claim is made. The later cleanup-retention regression compiled and failed before its targeted fix;
the same test then passed. Actual panic execution, full-scheduler join interruption,
native/blocking preemption, live snapshots, race/pool stress and complete P7 integration remain
unverified. E43's advisory/vet failures remain open; no exemption or phase acceptance is added.
Historical P1 is retained, broader P2 remains paused and UI remains deferred.

## E45 — Production readonly frame tools and authority boundaries (2026-10-26)

**VERIFIED scoped implementation:** KernelToolRunner now implements the real frame ToolRunner
over the actual workspace, CAS and session-bound history. Sealed bindings carry actual frame,
root, role and exact-token identity; explicit issuer verification, per-resource Biscuit checks,
consumed runner-bound permits and bounded local revocation fail closed. Workspace scope may
attenuate to narrower resources; no child may widen to Workspace. CAS references require both
frame visibility and Read on the full CID; pure queries require ExecuteTool on tools/<name>.
CAS/history are conservatively classified as reads at the role boundary. Result envelopes retain
truncation metadata, obey the caller/registry bound and are verified after real CAS write/readback.
The global 64-owner admission bound and shutdown guards cover detached blocking operations.

**Evidence:** final five-package native suite: 205 passed, 0 failed,
2 ignored. Linux capability/frames/builtin domain suite: 84 passed,
0 failed, including regular-file/FIFO/symlink and ignore-file regressions. Actual run_step tests
use the production runner with synthetic provider/assembler/verifier protocol fixtures; a child
reads only its attenuated path, and one denied call prevents every execution in its response.
Strict six-safety-lint plus warnings checks cover the kernel production library, both new public
integration targets and all targets of the three changed domain crates. Workspace all-target
compilation and member/layer/witness checks pass. [Commands and raw logs](evidence/continuation-20261026-e45/commands.json).

**Security and process limits:** directory enumeration observes names/metadata to find resources.
Authorized traversal preauthorizes bounded local ignore files, disables external parent/global
ignore rules and fails closed for denied ignored subtrees. Filesystems must remain stable; no
race-free hostile-filesystem claim is made. Cancelling an async future signals its blocking owner,
but does not preempt OS reads; shutdown drains that owner. Cancellation or failure during CAS
publication may leave bounded orphan data. Revocation is runner-local/in-memory, not global or
durable. The initial six adapter tests/abbreviated descriptor preceded source but their RED was
an API compile failure; canonical fields, extra regressions and loop tests followed implementation.
No full historical behavioral-TDD claim is made. The later pure-role regression compiled and
failed before its targeted policy fix, then the identical test passed. Full strict workspace lint,
new Miri/Kani proofs, complete kernel portability and live production walking skeleton remain open.
Earlier advisory/vet failures are unresolved. Broader P2 stays paused; UI stays deferred.

## E46 — Bounded durable frame journal over the actual kernel (2026-10-27)

**VERIFIED scoped implementation:** PreparedFrameStep validates explicit record/result/count
limits, schema and nonzero step, bounds JSON output before cloning result text, grows its buffer
geometrically, verifies receipt/content CIDs and exact model-visible bytes, then verifies transcript
CAS write/readback. Even a backend ignoring the requested read limit cannot return a successful
oversized receipt. KernelFrameJournal binds actual frame/root/session, acquires the bounded owner
before allocating prepared buffers, offloads store I/O, and publishes through the real log-then-bus
path. A started owner survives caller-future drop and is drained by kernel shutdown. Debug is opaque.

**Evidence:** final five-package native regression: 227 passed, 0 failed,
2 ignored; Linux capability/frames/builtin: 96 passed, 0 failed.
The real two-turn driver uses production tool/journal adapters and temporary stores: event steps,
session/frame, exact provider call ID/arguments, tool bytes, transcript hashes and replayed second
request are checked. A second-turn provider failure is journaled distinctly. Provider, tokenizer
bound and verifier fingerprint are synthetic protocol fixtures, not live-model or proof evidence.
Strict safety checks, scoped formatting, workspace compilation and member/layer/witness checks
pass; initial Clippy/test fixture failures are retained. [Commands](evidence/continuation-20261026-e46/commands.json).

**Limits:** the frame driver owns reserved-step order and outcome semantics; the journal is an
audit sink, not a global step-order/idempotency service or independent proof. Each successful call
publishes an event; blind retry can duplicate it. CAS persistence followed by publication failure
may leave an orphan. Per-record limits/global owner slots do not establish whole-process RSS or
preemption guarantees. The generic JournalAdapter remains synchronous and documented; the actual
kernel wrapper offloads its I/O. New kernel portability, full strict workspace lint, Miri/Kani,
live tokenizer/provider/context/edit/test/proof assembly and earlier advisory/vet gates remain open.
Broader P2 stays paused and UI stays deferred. Initial API/oversize-variant REDs are compile failures;
canonical-path Debug and admission-order regressions have compiled pre-fix failures and final greens.
The automatic-review usage-limit interruption changed no requested action; later access was
revalidated and resumed. No test assertion, dependency exemption or phase gate was loosened.

## E47 — Bound prompt assembly and actual tokenizer transport (2026-10-27)

**VERIFIED scoped implementation:** SDK PromptTokenizer counts the actual complete chat template;
VllmChatClient explicitly opts into same-origin /tokenize and shares model/messages/tools/reasoning
serialization with chat. The endpoint must be supplied by the deployment; no Nebius support or
live model is assumed. Token counts, capacity, token IDs, encoded bytes and deadlines fail closed.
The configured JSON response allowance must include the required complete token-ID array.

KernelPromptAssembler binds actual kernel/frame/role/model/effort/token and uses the actual PII
vault before any count request. It obtains tool schemas/effects from the real catalog, normalizes
CAS/history as reads, filters before counting, preserves complete exchanges, and offloads bounded
CAS archive work under inspection admission. The final notice is counted exactly. Archive
visibility also requires the existing full-CID Read right. All candidate archive writes share a
cumulative byte cap. Count deadlines and encoded context limits are explicit. Its run_frame entry
point verifies the actual active-frame selection and uses one shared tokenizer/chat owner.

**Evidence:** native seven-package regression: 288 passed, 0 failed, 3 ignored. Linux domain
result: 176 passed, 0 failed, 1 ignored. The real HTTP/frame composition verifies identical counted/dispatched wire
messages/tools/model, masked email data and actual durable CAS/event transcripts. The HTTP server,
token IDs/count and independent-verifier fingerprint are synthetic fixtures, not live-model or
proof evidence. Strict scoped safety/format checks and full workspace all-target compilation pass.
[Commands and retained failures](evidence/continuation-20261027-e47/commands.json).

**Limits:** generic low-level frame assembly/dispatch remains caller-injected; use the production
assembler's bound run_frame entry to enforce shared backend identity. Context snapshots are
explicit trusted inputs; this does not create an indexer or persisted scratchpad resolver.
Masking is DFA/entropy only with degraded semantic coverage, not complete DLP. Archives preserve
the sanitized model-visible transcript, and failed fitting/cancellation can leave bounded orphans.
Fitting may drop all old history on its final allowed count; maximal retention is not promised.
Per-count deadlines and shared owner slots do not establish total RSS, tokenization billing/rate
accounting or preempt OS I/O. Live vLLM/Nebius compatibility, complete edit/test/proof loop, kernel
Linux portability, new Miri/Kani and full strict workspace/advisory/vet acceptance remain open.
P2 remains paused; UI remains deferred. Initial API REDs were compile failures. The later deadline,
encoded-context and cumulative-archive regressions compiled and failed before their targeted fixes.
Additional dispatch/property fixtures followed implementation; no universal test-first claim is made.
The new archive fixture parser was corrected to the actual SDK §ref marker; retrieval and exact
content assertions remain. No existing production test contract or phase gate was loosened.


## E48 — Phase 3 human backend, mutation service and Contree adapter (2026-10-27)

**IN PROGRESS; no Phase 3 exit.** SDK/proto schema 5 adds bounded human request/answer contracts while retaining legacy envelopes and the unchanged fixed-size Event limit. The answer authority lives in the L4 verifier; the kernel persists exact approval/answer receipts and routes trusted host answers through the actual command bus. Request-scoped interruption/replay and answer races are covered. Model ask_user dispatch and actual frame-tree cancellation remain unfinished.

Frame-bound write/edit/patch service now performs real path/capability/AST/lease/OCC/hybrid-verifier checks, commits actual bytes, and stores read-back receipts. Eleven integration cases pass with explicitly synthetic semantic safety assessment. Rust unsafe syntax inventory is complete only for valid source without macros/attributes/opaque token trees; other coverage remains conservative. New tests proved and fixed stale answers, oversized reads, missing-final-newline evidence, role bypass, post-verification cancellation and non-durable answer witnesses.

The Contree adapter follows upstream immutable-image/run semantics and the exact 19-tool inventory, with bounded branching/cancellation and hash-checked approved-path upload projection through the SDK MCP port. Six client fixtures are synthetic; no live sidecar/SWE claim is made. Windows fragmented HTTP fixture regression reproduced WouldBlock before the explicit accepted-socket blocking-mode fix; both HTTP cases then passed in 20 consecutive executions. No existing acceptance assertion was loosened.

**Evidence:** nine-package native regression **583 passed / 0 failed / 3 ignored**; full workspace all-target compilation, six-library scoped safety, kernel integration safety, member/layer/witness, format and generator-preservation checks pass. Broader Code Intel arithmetic safety remains RED (38 violations); strict whole-workspace, new Linux/Miri/Kani, calibrated S1, workloads and dependency audit acceptance are not closed. [Retained commands, failures and limitations](evidence/phase3-20261027/README.md).

**Still open:** model mutation/ask_user/test dispatch, independently proven completion of the current source tree, local M3 transcript, actual frame-tree scoped cancellation/sibling continuation, Contree deployment/SWE resolver/workloads, and the remaining P3 owner matrices. Token Factory/Contree keyed M4 checks are BLOCKED-ON-ACCESS (user confirmed unavailable). P2 remains paused; UI remains deferred. All-phase goal remains active.

## E49 — Frame tool dispatch, durable cancellation and current-tree proof (2026-10-27)

VERIFIED scoped E49 checkpoint. Phase 3 remains IN PROGRESS; P2 is paused and UI deferred.

The model-facing kernel runner now dispatches real frame-bound write/edit/patch, file revisions,
durable ask_user and explicitly host-approved sandbox test requests. Exact sealed step/frame ancestry,
shared bounded cancellation registries, durable subtree receipts, independent runner cancellation,
shutdown ownership and complete-set admission protect these operations. SDK event schema 6 and
proto tags 28–30 preserve older records and the existing compact-event size bound. Host CancelFrame
uses a separate L4 right; the model does not receive host approval authority.

The production frame verifier validates exact recorded tool-result envelopes, mutation receipts,
approvals and the current source manifest. Completion requires the latest successful ordinary and
independent test verdict bound to the exact session/frame/review/source tree. New schema-2 proof
reports supply those bindings; older unbound proof reports require a fresh test run. These checks
do not constitute live S1 calibration or a full live model/edit/test/proof acceptance transcript.

Selected native regression: 655 passed, 0 failed, 3 ignored. Linux SDK/proto/capability/frames/
Code Intel/verifier/spike domain: 238 passed, 0 failed, 0 ignored. Kernel Linux is not covered.
Seven production libraries pass the recorded unsafe/panic/cast/index/arithmetic lint subset;
focused new kernel test targets pass the same subset. Workspace all-target compilation and
member/layer/witness/generated-doc/generator-preservation gates pass. Code Intel's 38 production
arithmetic violations reported in E48 are fixed; existing example/test and dependency lint debt
is not thereby closed. Exact commands, failed attempts and raw outputs remain below.

Seven limiter spike cases now exercise actual limiter instances: staged priority, shedding,
changing Retry-After floors, reservation/overrun, hostile typed durations, owned waiter cancellation
and real-clock admissions. They do not prove concurrent priority scheduling, fixed-floor anti-herd
spread, the 100-repetition A–D matrix, 10-minute load, fairness or formal/stress acceptance.
New polaris-ratelimit reservation_refund acceptance tests remain RED (four failures, three passes
in the retained investigation). They are excluded from the selected 655-test command. This is
not a green whole-workspace test claim. No refund assertion has been weakened.

The pinned local llama.cpp/Qwen runtime has separately produced three actual structured-tool-call
probes with exact count/chat prompt-token parity. See the tracked runtime provenance manifest.
This observation is not yet an integrated production provider/loader or M3 transcript. Downloaded
binary/weight artifacts are ignored and excluded from first-party inventory. Token Factory and
Contree keyed acceptance remains BLOCKED-ON-ACCESS. Full owner matrices, calibrated S1, real
M3/M4, new Miri/Kani, strict workspace lint and dependency audit/vet gates remain open.

Behavioral REDs include duplicate cross-service answers and stale/current-proof handling; missing
API compilation failures are distinguished in retained logs. Some additional tests followed
implementation, so no universal test-first claim is made. No source lint suppressions were added.

[Commands and limitations](evidence/phase3-20261027-dispatch/README.md).

## E50 — Local providers, System One and privacy integration (2026-10-27)

VERIFIED scoped implementation/evidence checkpoint; Phase 3 and Phase 4 remain IN PROGRESS.
UI is deferred and P2 remains paused except the sandbox image repair required for P3 tests.

Production additions include: same-owner pinned local llama.cpp counting/chat and an owned-process
loader; exact test-receipt review/frame/session lookup; human-answer reference access and authority
expiry checks; compatible Contree environment resolution; math expression guards; S1 batching,
checkpoint/generation/incarnation-bound exact cache and bounded single-flight; separate raw-byte
PII span engine/RPC/client/vault integration; exact environment ranking and bounded unique tool/file
candidates; owned S1 burst dispatch with answer-integrity validation; encrypted private CAS marker snapshots with explicit
host seal-key recovery; semantic prompt masking before tokenization; combined task routing; shared
consumer fail-open/closed policy with bounded admission and worker epoch fencing; binary calibration
metrics and a measured-candidate xtask; guarded pinned export/golden tooling. These additions do not
certify real-model quality, production thresholds, owner matrices or phase exit.

The selected 27-package native run recorded 620 passed / 2 failed / 16 ignored before the final
weighted-admission fix and single-flight follow-up. Both failures were refund acceptance cases.
The subsequent complete limiter/spike run recorded 16 passed / 3 failed / 0 ignored before new
independent refund regressions. The current refund suite records 7 passed / 7 failed: safe non-tail
refund capacity remains unrecovered, and three preserved fixtures have conflicting contracts.
All original refund assertions remain. Whole-phase tests are RED; no selected-green claim replaces it.
Weighted burst tests first failed twice, then passed twice after admission began checking the full
new weighted debt rather than the old unit-cost threshold.

Actual retained runtime observations include the local Qwen read-only tool/continuation transcript,
exact count/chat prompt-usage parity, owned loader shutdown, and the real OpenShell test pipeline
with fixed synthetic S2/S1 fixtures. The combined actual model/edit/test attempt remains RED on
invalid model arguments, denied before mutation. The rebuilt P3 image is immutable
sha256:b7a2dc2a53a71c5695242dd890d2705aa5d0e3b45dafcd885ae6a3ea2bbba4b2;
the paused P2 image/tag was not replaced. Direct Decide, actual authenticated Windows worker,
idle eviction/reload and supervised-worker inference pass using rehashed existing local artifacts.
The original four real-model cases do not establish 200-row parity or labelled calibration.
All nine companion PII artifacts are now pinned by exact revision/size/SHA256. An interrupted
encoder partial failed whole-file verification, was retained separately and never used; a fresh
full fetch passed every hash. Actual PII CPU inference passes a planted raw-UTF8 span fixture.
Actual authenticated PII/Decide/PII reload, idle eviction and warm/post-eviction 50-request bursts
pass (3 cases). Each repeated-input burst answered 50/50 at peak 8 owned tasks; this exercises
cache/single-flight, not 50 distinct native model invocations or the full throughput matrix.
Two exact owned worker kills exposed a fatal-versus-transient retirement bug. After correction,
actual replacement/authenticated readiness/final retirement pass; the supervisor suite records
50 passed. These two kills do not complete the full crash-loop/intensity/workload matrix.

A later scoped Linux run recorded 164 passed / 0 failed / 2 ignored for kernel prompt/model/privacy,
client/PII/facade/router/schemas/secrets/proto, before the newest rules/router/watchdog changes.
Workspace all-target compilation previously passed after propagating additive health fields to RPC;
new security-rule/watchdog integration is accumulating and final-tree compilation is not certified.
Member/layer/witness, generated docs and generator preservation checks pass. Five selected production
libraries and four selected kernel test targets pass the retained unsafe/panic/cast/index/arithmetic
lint subset. This does not certify strict whole-workspace lint or a fresh final Linux/full formal run.

Still open: correct bounded out-of-order refund accounting and conflicting late-refund assumptions;
actual combined local edit/test/proof acceptance with calibrated S1; Token Factory and Contree keyed
acceptance (BLOCKED-ON-ACCESS, user confirmed unavailable); full PII corpus/quality/resource results;
full S1 parity, held-out calibration and MVP recording; watchdog/context/role/candidate integrations
and complete owner load, scalability, stress, fuzz, Miri, Kani, security and release matrices.
Export inputs are present, but the 10 GiB available-RAM precondition was not satisfied in the last
probe; no RAM guard or assertion was weakened. Snapshot key provisioning/rotation and rollback
protection are host lifecycle obligations; the implemented key recovery never silently replaces keys.

[Exact retained commands and limitations](evidence/phase3-20261027-local/README.md).

## E51 — Non-UI task and host-control implementation (2026-10-28)

**IN PROGRESS; implementation is UNVERIFIED.** The user explicitly moved verification to the end
of implementation. No phase exit, successful final-tree build, test pass, production-readiness
claim or completion-percentage increase is inferred from this source batch.

The task entry now composes explicit host-authorized mutation capabilities, the actual kernel
mutation runner and current supervised S1 safety engine, pinned measured schema loading, real
OpenShell startup/policy/image configuration, questions, and exact-digest sandbox-test review.
Inspection-only profiles remain supported. A bounded host-control API supplies actual pending
questions, exact-revision answers, pending review discovery, immutable review inspection,
execution approval and frame cancellation. The CLI uses an explicit private request directory
with a sole-owner file lock and claimed request files; it does not start an uncancellable stdin
reader. Interrupted processing files are never automatically retried.

Optional encrypted private frame checkpoints persist the reversible marker map and authenticated
session/frame/step/transcript association before publishing the transcript event. Restore checks
the actual durable event and CAS contents. Host key references use the existing environment or
keyring source without provisioning replacement keys. The host must protect the directory and
retain the key; external rollback protection and Windows directory-rename power-loss guarantees
are not established by this implementation.

Prompt assembly now has actual-worker tool-pruning composition after role and privacy filtering,
retaining available expansion/revision/question/test tools. Measured watchdog policy is an explicit
host input. Calibrated safety/watchdog consumers use an added pinned decision path: a discarded
warmup can establish Decide identity after cold start or a PII swap; accepted results require the
exact expected native hash and stable model generation/incarnation, followed by the kernel's
exact owned-process-generation check. Built-in uncalibrated advice remains explicit.

**Verification deferred:** the new implementation/descriptors have not received a final compile,
integration, regression, concurrency, allocation or acceptance pass. The retained E50/earlier
failures remain failures: the later real S1 burst failed at 42/50 post-eviction responses before
reaching the distinct-input round; limiter tests/Clippy also have retained incomplete or failing
attempts. No assertion was loosened to change those results. Existing percentages remain dated
estimates, not a measurement of E51. Full Phase 3/4 matrices, calibrated artifacts and actual
edit/test/proof acceptance remain open. Token Factory/Contree access is still unavailable;
P2 stays paused except necessary integration, and UI remains deferred.

## E52 — Heavy execution model pause and pure-tool composition (2026-10-28)

**IN PROGRESS; new implementation is UNVERIFIED.** Verification remains deferred until the end
of implementation by explicit user instruction. This entry does not certify a build, regression
result, workload, resource limit or phase exit.

The worker protocol adds optional monotonic pause/resume epochs to model eviction and advances
the compatible minor version to 1.2. The thread-affine native actor handles pause, eviction and
resume in the same command order as inference. Queued/new Decide and PII calls cannot reload a
paused actor. The client requires exact epoch/state acknowledgement; an older worker's ordinary
eviction response cannot authorize heavy execution.

Kernel lifecycle code owns serialized pause leases, rejects inference results crossing admission
revisions, and applies a requested pause before publishing a replacement worker. Dropping a lease
leaves admission closed; a later build cannot overwrite an abandoned pause. Explicit recovery uses
a fresh epoch to settle lost acknowledgements. Kernel recovery additionally owns the execution
lease and refuses recovery while any sandbox teardown ownership remains in the Reaper.

The production sandbox dispatcher now uses the pause-aware execution entry after signed review
verification and fresh-source checks. The pause lasts through backend teardown and durable intent
release. Unconfirmed cleanup preserves the pause; failed resume blocks the terminal proof. Early
accepted setup failures now publish exact-bound blocked evidence and a terminal execution event,
allowing the corresponding test-review waiter to continue. Cancellation before create releases its
intent without deleting a name it never created. Unexpected returned sandbox identities keep the
pause closed when their own teardown is unconfirmed. No RSS reduction amount or hard memory cap
is inferred from a native handle eviction acknowledgement.

The real deterministic math dispatcher is now a frame tool: typed input/role/capability checks,
owned blocking evaluation, bounded result serialization and durable CAS receipts. The frame
verifier checks the recorded operation against a recomputed exact result. This consistency check
does not independently prove the mathematical implementation correct. Unknown math input fields
are rejected. Existing assertion contracts remain; protobuf fixture construction only adds defaults
for the new optional pause fields.

Prompt fitting now retains tool rank order, supplements the masked goal with a bounded sanitized
recent assistant step, and can reduce optional tool schemas using actual tokenizer counts after
history is archived. Required expansion/revision/question/test tools and at least one optional
operation remain. Uncertain advice initially retains the catalog; exact context fitting can still
require fewer schemas. Repeated schema-fitting counts reuse an unchanged archive instead of
charging/writing that same prefix again. All count/archive/deadline ceilings remain in force.

**Still open:** final compilation, relevant contract/snapshot updates, new descriptor cases,
integration and broad verification for E51/E52; existing limiter/S1 failures; all remaining Phase
3/4 owner matrices, measured calibration/parity/resource evidence, actual local edit/test/proof,
keyed acceptance and demo requirements. Token Factory/Contree access remains unavailable.
Percentages are unchanged dated estimates. UI remains deferred and P2 otherwise stays paused.

## E53 — P9 frontends attach to `polarisd` (2026-10-29)

**IN PROGRESS (phase acceptance open).** Desktop, `polaris web` and the Leptos `polaris-web` attach
to a running `polarisd` over its local endpoint (`polaris-*` named pipe on Windows, private Unix
socket elsewhere) with a credential the daemon issued.

- **Protocol 1.3.** `SubmitText` records typed text as user input or one directive; `ReadText`
  returns the text behind a CID. The server authorizes `ProposeDiff` for submit and `Read` for
  read before calling the injected `SessionTextApi`; text is capped at 64 KiB both ways.
- **Daemon authority.** `DaemonText` submits through the kernel's own control path (clients never
  mint commands) and serves only CIDs that a directive/user-input event of the daemon's session
  references (subscription index plus durable-log reindex); other digests are `NOT_FOUND`.
  `DaemonResolver` pins both calls to the daemon session.
- **Credentials.** `polarisd --client-token-out <file> [--client-token-hours N]` issues a Biscuit
  (Read + ProposeDiff on `sessions/<id>/rpc`, 1 s..7 d) under a fresh root, writes it hex-encoded
  with `create_new` (0600 on Unix) and trusts only that root.
- **Frontends.** `polaris_ui_session::RemoteSession` replays the session from the first record with
  text read back; desktop `bridge::spawn_attached`, web `WebServer::bind_with(.., attach)`.
  In attached mode model replies are not served (announced as provider-unavailable); search and
  activity stay local.
- **Security tests.** `tests/apps/polaris-daemon/daemon_lifecycle.rs` (read-only credential cannot
  write, foreign issuer `UNAUTHENTICATED`, other session `PERMISSION_DENIED`, unreferenced CID
  `NOT_FOUND`, malformed directive `INVALID_ARGUMENT`, oversize rejected, bounded token lifetime);
  attach tests against a real daemon: `session/remote.rs` (2), `desktop/bridge_attach.rs` (2),
  `web/web_attach.rs` (1), desktop args (5). Evidence: `@prime/evidence/p9/attach-verification.txt`.

**Still open:** model replies in attached mode (the daemon has no provider chain), remote mTLS
deployment, attachments and speech, independent security audit, P9-X-1 visual acceptance.

## E54 — P5 `$nvidia(...)`, `/nv-profile` and `/nv-verify` wired (2026-10-29)

**IN PROGRESS (phase acceptance open).** PLAN §6.14 asks for the wiring "at the composition root";
it lives in `polaris_ui_session::nvidia` and every frontend uses it.

- **Kernel directive state.** `KernelHandle::directives()` returns the directives in effect as the
  command owner last committed them (seeded at start from defaults and the replayed log, updated
  after each `$directive`). Resume restores `$nvidia(off)` from the durable log (tested).
- **`$nvidia(auto)`** (also the mode with no `$nvidia` directive): each chat turn profiles the
  workspace and prompt (`auto_profile`, bounded walk) and, only when a stack is found, appends the
  profile, per-topic invariants and the first eight `/nv-verify` findings to the system prompt.
  `$nvidia(list)` keeps it out of the model's context; `$nvidia(off)` disables the commands too.
- **`/nv-profile`** (stack, build targets, evidence, local GPUs from `nvidia-smi` with a 3 s
  bound, notes where the targets exceed the local GPU) and **`/nv-verify`** (CUDA advisor over
  `.cu`/`.cuh`, skipping vendored/build directories, bounded) answer in the TUI (sheet), the web
  pages and the desktop window (a report entry, not recorded or sent to the model), and headless
  as `polaris nv-profile` / `polaris nv-verify`. Attached frontends decline them (no local kernel).
- **Sample repositories.** `tests/fixtures/nvidia/{cuda-vector-add,trtllm-serving,plain-rust}`,
  stored as `*.fixture` and materialized at test time so this repository never profiles as CUDA;
  provenance in `tests/fixtures/MANIFEST.toml`.
- **Tests.** `session/nvidia.rs` (5: planted findings at exact lines and severities, vendored code
  skipped, profiles of all three samples, turn context per mode, directive mode across resume,
  exact command matching), `session/chat.rs` (the model request carries the profile, and not after
  `$nvidia(off)`), desktop `bridge.rs` and web `web_api.rs` (`/nv-verify` answered with a report),
  Leptos reducer test.
- **Evidence.** `@prime/evidence/p5/nvidia-sample-repos.txt` (real binary on the three samples;
  this machine's GPU is a Quadro RTX 4000, sm_75, so the sm_90 target note fires) and
  `@prime/evidence/p5/live-chat-cuda-sample.txt` (one live turn through the provider chain on the
  synthetic CUDA sample; the reply follows the advisor findings but misquotes three line numbers —
  recorded as observed).

**Still open:** NVIDIA docs sync run and offline seed (P5-X-3), cited doc slices in turns (needs
the index), vector search (Token Factory key), tree-sitter rule packs and per-arch limit tables,
S1 topic scoring, `$rag` and `/search-local` with provenance.

## E55 — P7 `/hack` finds the seeded bugs (2026-10-29)

**P7-X-2 PASS on a synthetic seeded repository; P7 phase acceptance open.**

- **Race rules** (`polaris-hacker`, advisory): `race-split-lock` — a value read under one lock
  (bound and used in the write, or tested in a condition within 8 lines) written under a second
  lock of the same mutex/`RwLock`; `race-atomic-load-store` — the same for an atomic `load` and a
  separate `store` with no read-modify-write between and no guard held across both. Bound guards
  (`let g = m.lock().unwrap()`, `if let Ok(g) = m.lock()`) count only when dropped first; writes
  are not checks; `read()`/`write()` count only when unwrapped, expected or awaited. Comments and
  literals are blanked first. Blocking-in-async now also covers `thread::sleep` without the `std`
  prefix, `std::fs` reads/writes and `TcpStream::connect`.
- **`/hack`** (`polaris_ui_session::hack`, through `commands::LocalCommand` shared with
  `/nv-profile` and `/nv-verify`): Rust/Python/C++/TypeScript through the hacker within its
  budgets, precise credential formats and destructive shell commands from the automata pack (the
  generic `name: value` secret rule and PII rules are left out of source audits), CUDA warnings
  from the advisor. Answered in the TUI, web pages, desktop window and as `polaris hack`.
- **Seeded repository** `tests/fixtures/hack/seeded-service` (stored as `*.fixture`), with
  `SEEDED.toml`: two races, two blocking calls in async handlers, one CUDA barrier in a divergent
  branch, and `clean.rs` doing the same work correctly.
- **Result** (`@prime/evidence/p7/hack-seeded-service.txt`, real binary): recall 5/5, 0 extra
  findings. Calibration on this repository cut the first version's 11 race candidates and 9 generic
  secret matches in `polaris-kernel` to 1 advisory finding (`model_lifecycle.rs` stop: check of
  an atomic flag, then store), 0 in seven other crates; it also caught a blocking file read in
  this session's own async `polaris web --connect` path, fixed with `tokio::fs`.
- **Tests:** `polaris-hacker` `race_rules` (5), `polaris-ui-session` `hack` (4), plus the shared
  command parsing in `nvidia` tests.

**Still open (P7):** `/research` with a real model and sandbox (P7-X-1: the loop, durable resume
and 20-iteration contract exist; the composition needs a sandbox — the OpenShell gateway and the
Polaris sandbox image, P2-X-1); the microVM browser loop (P7-X-3: host-side outline/pHash/session
exist; the guest Chromium driver needs the same image); `/redteam`, sanitizer workloads.

## E56 — P8 `/ci` runs in Docker with cache mounts (2026-10-29)

**`/ci` exit item PASS on a local Docker Engine; P8 phase acceptance open (`$env(nix)`, Needle).**

- **`DockerSandbox`** (`polaris-devenv-docker`, implements the SDK `SandboxBackend`): one
  container per sandbox (`sleep infinity`, labelled `polaris.ci=1`), the workspace bind-mounted
  read-only, named cache volumes that outlive the container, every capability dropped,
  `no-new-privileges`, CPU/memory/PID limits, bridge network for cold dependency fetches (off with
  `network: false`); exec with working directory, output cap and timeout; file transfer refused
  (the mount is the workspace); `remove_caches`, `remove_orphan_ci_containers`. Nothing is pulled.
- **Cache mounts in the pipeline** (`polaris-devenv-cicd`): `validate()` still rejects them (a
  generic backend cannot apply them); `validate_with_cache_mounts()` accepts valid keys/absolute
  targets for a backend that mounts them, `cache_mounts()` returns the distinct mounts, and
  `rust_test_pipeline_cached` puts `CARGO_HOME` and `CARGO_TARGET_DIR` on `<prefix>-cargo` /
  `<prefix>-target`.
- **`/ci` and `/ci-run`** (`polaris_ui_session::ci`, through `LocalCommand`): the plan (steps,
  image, per-workspace volumes `polaris-ci-<12 hex>` with cold/warm state, Docker readiness and the
  image build command when missing) and the run (verdict, per-step status/time/exit, cache state
  before and after, the failing step's log tail; logs redacted by the automata redactor). Answered
  in the TUI, web pages and desktop window off their session loops (a run can take minutes), and
  as `polaris ci|ci-run`. `$env(nix|host)` is reported: CI still runs in Docker.
- **CI image** `@prime/tools/ci/Dockerfile`: `rust:1.99.0-bookworm` (pulled with the user's
  approval) plus rustfmt and clippy → `polaris-ci-rust:1.99.0`.
- **Evidence** `@prime/evidence/p8/ci-run-docker.txt` (real binary, Docker Desktop 28.4.0):
  sample `tests/fixtures/ci/rust-lib` cold 5.1 s (check 1.2 s, test 2.8 s) → warm 1.4 s (0.2 s
  each), 9 MB of artifacts in the target volume; a copy with one unformatted line fails at
  `format` with the rustfmt diff and skips the rest; no container left behind.
- **Tests:** `cicd` `cache_mounts` (2), `polaris-ui-session` `ci` (3, the live one with
  `POLARIS_CI_DOCKER_LIVE=1`: cold then warm, then removes its volumes).

**Still open (P8):** `$env(nix)` devShell evaluation (needs Nix in a sandbox), `/nix-flake`
drafting, Dagger, Podman, matrix legs beyond linux-x86_64, Needle T0 (engine and weights are not on
this machine; the S0-6 spike passed elsewhere).

## E57 — No Nebius (D10), keyless search, and P10 hardening (2026-10-29)

**Decision D10.** Nebius services (Token Factory, Contree, AI Cloud, Data Lab) are not available to
the team in Ukraine. Thirteen rows that needed them are `DESCOPED` (new status, counted by
`workspace.py render-docs`); models run through OpenRouter or a local server, execution through the
local Docker sandbox (E56), knowledge through lexical search. Only the Tavily rows stay
`BLOCKED-ON-ACCESS`.

**P5, keyless.** `/search-local <query>` and `$rag` (`polaris_ui_session::local_search`): BM25 over
40-line windows (step 30) of the workspace's text files, terms including `camelCase`/`snake_case`
parts, overlapping windows of a file reported once, each hit as `path:first-last` with its best
lines. Files are indexed once per process and reused while size and modification time hold (this
repository: first query 5.1 s, next 0.33 s, optimized). `$rag`/`$rag(on)`/`$rag(workspace)` add the
best five passages to each turn's request as labelled untrusted data. Answered in every frontend and
as `polaris search-local`. Tests: `session/local_search.rs` (5), `session/chat.rs` (`$rag` carries
passages, not without it). Evidence: `@prime/evidence/p5/search-local-polaris.txt`.

**P10-1 (IN PROGRESS).** Full workspace `cargo test --workspace --no-fail-fast`: 549 test binaries,
2,630 passed, 4 failed (the in-progress `polaris-ratelimit` refund ledger, unchanged), 50 ignored.
Loom models (bus ordering, cache single-flight, leases) and the bus/ratelimit/web/sandbox/S1 spike
suites pass optimized (7 live sandbox tests ignored without OpenShell). Fuzz, Miri, Kani and mutants
need tools that are not installed. Evidence: `@prime/evidence/p10/test-matrix.txt`.

**P10-2 (IN PROGRESS).** `@prime/SECURITY_REVIEW.md`: the §6.11 threat model against the code and
its tests; no `unsafe` in first-party code (workspace `deny`, 15 roots `forbid`); offline licence
audit of 1,293 third-party packages against `deny.toml` (`@prime/tools/licence_audit.py`,
`@prime/evidence/p10/licence-audit.txt`) — one WTFPL crate (`terminfo`) reachable only through
ratatui's disabled termwiz backend. RustSec advisories and `cargo vet` need their tools.

**P10-3 (PASS — measured and recorded).** `tests/tests/budgets.rs` and `budget_alloc.rs` (dhat)
in the new `budget` profile; numbers in PLAN §11.4 and `@prime/evidence/p10/budgets.txt`: bus
publish p99 0.5–4.5 µs (met), GCRA p50 0.7 µs / p99 1.1 µs with 0 allocations (p99 just over), DFA
438 MB/s (not met, ≥ 500), SSE 0 allocations per warm frame (met), sync 10k files 1.14 s (met), TUI
200×60 p99 5.1 ms (met), S1 fp32 CPU p95 758 ms from S0-4 (not met; int8 decision pending).

**P10-4 (PASS).** `cargo build --profile dist` of `polaris`, `polarisd`, `polaris-desktop`,
`polaris-web`, `polaris-worker` and the Leptos site; `@prime/tools/package_dist.py` packages them
with the new root `README.md` and `LICENSE` into `target/package/polaris-<version>-<target>.zip`
and records `SHA256SUMS.txt` and `build-info.json` in `@prime/evidence/p10/` (binaries are not
committed).

**Docs.** `render-docs` regenerated the crate tables from `workspace.toml` after the rows edited in
recent sessions were written back to it, so a re-render no longer reverts them.

**Still open (P10):** fuzz/Miri/Kani/mutants and advisory/vet runs (tool installs), the Linux build
leg on this machine (crate downloads in a container), the demo video (P10-5), an external review.

## E58 — Remaining composition written; no new test runs (2026-10-30)

The user asked to finish the code without running tests, so this entry records code only:
`cargo check --workspace` passes on Windows; nothing below has a new test or live run.

- **P9 replies in attached mode.** `polarisd` answers each typed message with a read-only chat
  turn (`polaris_ui_session::responder`; `--env-file`, `--no-replies`). Attached web and desktop
  windows read the reply back from the step's `FrameStepRecorded` record; the daemon serves only
  the reply text of a transcript, never its prompt or tool results.
- **P9-X-2.** Frontends attach to `tcp://host:port`; off loopback the client's mutual TLS files
  come from `POLARIS_TLS_CA/CERT/KEY` (`AttachTls`).
- **P5-X-3.** `polaris_nvidia_docs::sync`: the PLAN F20 `llms.txt` listings into
  `~/.polaris/docs/nvidia/seed.jsonl` with per-page provenance in `state.json`, incrementally
  (304 keeps chunks, failed pages keep theirs, unlisted pages are dropped), written atomically.
  `/nv-docs-sync` (= `$nvidia-docs(sync)`), `/nv-docs <query>`, and the best passages in
  `$nvidia(auto)` turns.
- **P5-X-1.** `/search-web <query>` and `$websearch` through the redacting SearXNG adapter;
  `@prime/tools/searxng/run.ps1|run.sh` start a loopback container with minimal capabilities.
- **P7-X-1.** `/research <goal>|status|report|resume` (`polaris_ui_session::research`): the model
  proposes one hypothesis as the content of one editable file, the loop writes it under an
  attenuated capability, the metric runs in a no-network Docker container with the workspace
  read-only, and checkpoints allow `/research resume` after a kill. A program parser bug let
  `editable: **` overlap read-only scopes (so `program.md` and the harness were editable); fixed
  in `program.rs`.
- **Chat edits.** `/diff` lists the session's applied changes from its `DiffApproved` diffs;
  `/undo` restores the newest one whose file is unchanged since (host write, outside review).
  Moves and multi-file changes are explained to the model as write + delete and one reviewed
  change per file.
- **P10.** `cargo deny` passes (pulldown-cmark wikilink advisory fixed in ingest, other
  advisories ignored with reasons), `cargo audit` reports one accepted advisory, `cargo vet`
  1,167 unvetted (`@prime/evidence/p10/supply-chain.txt`). Linux `cargo check --workspace
  --all-targets --locked` passes in Docker after a peer-pid fix; the Linux test run was cut short
  while compiling and is not evidence. Demo script `@prime/DEMO.md` with
  `@prime/tools/demo/prepare.ps1`.

**Still open:** live runs of all of the above, the Linux test leg, Miri/fuzz/mutants, the demo
recording, System One nano (Needle `--serve` protocol unverified, S0-6), the microVM browser loop
(P7-X-3), calibration (P4-X-2).

## E59 — Desktop Models, Usage and Settings pages (2026-10-30)

Code only, like E58 (`cargo check` of `polaris-ui-gui` and `polaris-desktop` passes; nothing was
rendered or tested). The three rail panels of the desktop window become pages over the content
(`ui/pages.slint`), with the rail still usable and the accessible labels kept:

- **Models:** the model in use, the model for every effort (click to switch), the provider chain
  as it was tried with each skip reason (`BackendEvent::Providers` from the bridge), and setup notes.
- **Usage:** the open session's requests and tokens as tiles, then activity across local sessions.
- **Settings:** a reduced-motion switch applied to the aurora at once and saved as
  `ui.cli.reduce_motion` (shared with the terminal UI; read again at startup), the connection
  (local store or the attached daemon), workspace, session store, preferences file and keys file
  (path only), keyboard notes, and About with the Slint attribution (ADR-014).

Attachments and speech input stay open.

## E60 — System One and the OpenShell sandbox in every session (2026-10-30)

Release builds only (`cargo build --release -p polaris-cli -p polaris-worker -p polaris-desktop
-p polaris-daemon -p polaris-web` passes); live results below are from this machine.

- **OpenShell live (P2).** Docker Engine in WSL Ubuntu (disk moved to D:), OpenShell gateway
  v0.1.4 with the Docker driver on `https://127.0.0.1:17670` (mTLS). `verify-openshell.sh`:
  `RESULT: PASS` — create 10.1 s, exec 172 ms, exit-code propagation, byte-exact upload, delete.
  Sandbox image `polaris-sandbox:p2` = `sha256:50163fb3…7137` on
  `nvcr.io/nvidia/base/ubuntu@sha256:d9461f42…e378`, `polaris-guest --protocol-version` = 1,
  rustc/cargo 1.99.0 inside (`@prime/tools/sandbox/bringup-wsl.sh`; root steps in
  `@prime/tools/setup/setup-wsl-root.sh`; apt forced to IPv4 because WSL IPv6 is broken).
  Live headless acceptance (`verify-headless.ps1`, release binary, std-only fixture):
  positive → `proof_passed` (exit 0, 1.4 s in the sandbox); zero-tests, failing-command and
  minimum-tests → `blocked`; one sandbox run each, no unresolved resources; replaying the
  consumed approval after a restart is refused without a second run
  (`@prime/evidence/p2/openshell-live-acceptance-20261030.json`).
- **Sandbox in chat sessions.** `polaris_ui_session::sandbox`: `~/.polaris/sandbox.json` (endpoint,
  server name, mTLS directory, policy, immutable image id, WSL distro, quotas) makes every
  session kernel start with `Kernel::start_sandbox`; the WSL distro is kept awake while the
  session is open. The model gets `run_tests`; the kernel's immutable execution review is shown
  in the review panel (commands, independent check, timeout), approval runs it in the sandbox
  with Proof of Done, a decline stops the reply. An unreachable gateway opens the session
  without the sandbox and says why. Status in every frontend and on the desktop Settings page.
- **System One in chat sessions (P4).** Sessions start the local GLiNER2.5-Decide worker (fp32,
  CPU) named in `~/.polaris/config.toml` `[system_one]`; agent changes go through the configured
  verifier (safety.diff_tamper, safety.secret_semantic, watch.congruence) with change review.
  `polaris s1-calibrate <fixtures>` measures every schema on a labelled set with the running
  worker (calibration context = fixture BLAKE3 + the worker's model identity) and keeps a schema
  only at ≥ 99 % precision with ≥ 10 accepted; the catalog is `~/.polaris/system-one/
  safety-schemas.toml`. Synthetic labelled set: `@prime/tools/system-one/make_safety_fixtures.py`
  (400 rows: 100 real changes from this repository's history, 100 test tampers, 100 secrets,
  100 intent mismatches). Measured with the worker (model identity `fdc0f0ab8f3cb9e1…`, ~1.6 s
  per decision on CPU): **safety.diff_tamper** accepts at P ≥ 0.753 with precision 100 % and
  recall 14 % (300 cases), **safety.secret_semantic** at P ≥ 0.521 with precision 100 % and
  recall 21 % (300 cases); **watch.congruence** did not reach 99 % at any threshold (200 cases)
  and stays uncalibrated, so such changes always go to the person. Synthetic data: the numbers
  say the model separates these generated cases, not that it is safe on real changes. The first
  run measured almost nothing: whole BEFORE/AFTER files (13 KB on average) exhausted the
  worker's memory; the gates now read the changed lines bounded to the encoder window.
- **Stubs removed.** TUI `/plan`, `/proof-of-done`, `/context-map`, `/parallel-dag`,
  `/concurrency-stats`, `/cache stats`, `/session replay`, `/redteam` act; the Nebius commands say
  they are descoped (D10). Stale "documentation-only" headers of `polaris-guest` and
  `polaris-openshell-policy` replaced.

**Still open:** `/goal`, `/context-expand`, `/cache purge`, `/screenshot`, `/browser`, `/ui-diff`,
`/nix-flake` (P7 browser and microVM profiles), the daemon terminal (no host shell by design),
System One nano (S0-6).

## E61 — Nebius integration written for teams with access; never run (2026-10-30)

The team has no Nebius access (D10), so this entry is code only: `cargo clippy --release` of the
changed crates and the release build pass; no Nebius endpoint was contacted. Contracts follow the
published documentation (Token Factory OpenAI-compatible reference for files, fine-tuning and
embeddings; `contree-mcp` configuration; the `nebius` CLI page for compute instances — the AI
Cloud API itself is gRPC only). Settings: `~/.polaris/nebius.json` (global layer only).

- **Token Factory client:** `upload_file` (multipart `POST files`, purpose `fine-tune`),
  `create_fine_tuning_job`, `fine_tuning_job`, `list_fine_tuning_jobs`, through the same bounded,
  retrying, key-redacting transport as chat and embeddings.
- **P5-X-4:** `/search-local` and `$rag` take three times the lexical candidates and re-rank them
  by cosine similarity of Token Factory embeddings of redactor-masked text (`embedding_model`).
- **P6-X-3 Data Lab:** `/datalab-sync` reads the session log for frames with an applied change, a
  sandbox run and a passed Proof of Done, records consent for that session (the command is the
  user's decision), binds each proof through an event-bound `ProofResolver`, masks and rescans
  through `polaris-nebius-cloud-data-lab`, exports ChatML JSONL and uploads the conversations;
  `/datalab-finetune [model]` starts a job (`data_lab.base_model`, `suffix`, `hyperparameters`).
- **P3-X-3 / P6-X-1 Contree:** the MCP hub connects over streamable HTTP to a user-run
  `contree-mcp --mode http` on loopback (no host process is launched), `ContreeBackend` verifies
  the pinned tool inventory; `polaris nebius contree-run [--sync] <image> <cmd>…` syncs the
  workspace (secrets and build output excluded) and runs up to eight branches, recorded in
  `.polaris/contree/branches.json` and shown by `/contree-branches`; `swe-run <owner/repo>
  <commit> <cmd>` resolves the image with `resolve_environment` over the catalog and the user's
  trusted annotations (`~/.polaris/contree-images.json`, newest first on a tie).
- **P6-X-2 AI Cloud:** `polaris nebius cloud-up | cloud-status | cloud-down` drive the `nebius`
  CLI: a `ValidatedVmSpec` (disk, hard lifetime, idle limit, mTLS ingress), a `BudgetGuard`
  reservation of the whole lifetime, an owner label selected with `select_owned_resources`, and
  cloud-init installing Docker and the pinned OpenShell gateway on the VM's loopback (reached
  through an SSH tunnel; `sandbox.json` then points at it).
- **`/nebius-status`** reports the key, model listing with each effort's model, fine-tuning
  jobs, embeddings, Contree reachability, the last Data Lab upload and job, and AI Cloud settings.
- **`$budget(usd=…|tokens=…)`** is enforced: a new turn is refused once the session's
  `ModelRequestCompleted` costs or tokens reach it.
- TUI `/nebius-status`, `/contree-branches`, `/datalab-sync`, `/datalab-finetune` act (desktop and
  web through the shared local commands); `polaris nebius <command>` on the command line.

**Unverified by construction:** every Nebius request shape above, the CLI flags marked in the
code, and the dataset format the fine-tuning service accepts.
