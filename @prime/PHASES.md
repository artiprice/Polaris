# Polaris — Phases

> **Document:** `@prime/PHASES.md` · **Date:** 2026-10-20 · **Companions:** `@prime/PLAN.md` (what & why),
> `@prime/PHASES_STATUS.md` (evidence-backed status). Crate task rows are generated from
> `@prime/tools/workspace.toml` (`python -I @prime/tools/workspace.py phase-tables --kind phases`);
> `check-docs` verifies every package has a task here.

## 0. Rules

- **Statuses** (PHASES_STATUS.md): `PASS` (command + raw output + conditions pasted), `FAIL`, `BLOCKED`,
  `BLOCKED-ON-ACCESS` (missing key/account), `IN PROGRESS`, `NOT STARTED`. Doc-only skeletons are never
  "implemented".
- **Test-first** (prompt.md §6.3): every crate task starts with JSON descriptors → generated failing tests
  (observed red) → implementation → observed green. PLAN.md §11.2.
- **Keyless first** (D9): no API keys exist today. Work that needs Token Factory / Contree / Tavily / AI Cloud
  is marked `BLOCKED-ON-ACCESS` and never faked; synthetic fixtures are labelled.
- **Gates**: a phase exits only when its exit gate has PASS evidence. A crate task exits when its PLAN §13
  TODO list is complete and its descriptors' tests pass (plus the category tests named in its matrix).
- **Dependencies**: a crate task may start when the crate tasks it depends on are at least `IN PROGRESS`
  with their public API frozen (trait signatures); it may exit only after they PASS.
- **Cut line**: P0–P4 + P10 are **must**; P5–P7 **should**; P8–P9 **could** for the hackathon submission.
  v2 items are declared in PLAN §16.

## 1. Phase overview

| Phase | Goal | Priority | Needs access | Exit gate (evidence in PHASES_STATUS) |
|---|---|---|---|---|
| P0 | Foundations & de-risking: skeleton, toolchain, WSL2/OpenShell, keys requested, spikes | must | keys requested only | skeleton gates PASS; WSL2 + OpenShell gateway reachable; all keyless spikes decided (PASS or fallback chosen) |
| P1 | Kernel core (offline): sdk, proto, config, log, telemetry, storage, bus, directives, supervisor, reaper, code-intel, RTK, read-only tools, TUI shell | must | none | TUI runs a session, parses all `$`/`/`/`@`, persists/resumes sessions; Linux check leg green |
| P2 | Execution plane: platform, OpenShell client/policy/blueprint, sync, sandboxed shell, vcs, leases, guards | must | none (local WSL2) | sandbox create/exec/destroy from Windows; sync byte-exact; blast-radius + proof-of-done gate work on a sample repo |
| P3 | S2 walking skeleton → Token Factory; ratelimit; models; frames; context; operators; capability; verifier; Contree | must | Token Factory key; Contree | agent loop edits + tests a sample repo against local OpenAI-compatible server; same against Token Factory once keyed |
| P4 | System One: worker, engine, client, schemas, router, verifier, watchdog; PII + secrets | must | none (models downloaded) | S1 parity (S0-4) and latency recorded; fail-open/closed matrix tested; **MVP demo** recorded |
| P5 | Knowledge: nvidia-docs, search (SearXNG/Tavily), embeddings + vectors, ingest + OCR, full context virtualization | should | Tavily, Token Factory (embeddings) | `$nvidia(auto)` profile on sample repos; `/nv-verify` rules; `$rag` + `/search-local` answer with provenance |
| P6 | Nebius depth: SWE env resolver, branching, Data Lab, remote gateway, `$budget`, `/nebius-status` | should | Contree, Data Lab, AI Cloud | branch search over `directory_state_id` on a SWE-bench task; remote gateway E4 works |
| P7 | Advanced agentics: `/research`, DAG + races, hacker, microVM + browser + guest, MCP, skills, Reflect, HUD | should | — | `/research` runs ≥ N iterations with resume; `/hack` finds seeded bugs; screenshot + a11y loop |
| P8 | DevEnv: docker, nix, cicd, Needle T0 | could | — | `/ci` pipeline with cache mounts; `$env(nix)`; Needle embeddings if S0-6 passed |
| P9 | Interfaces: rpc daemon, Slint desktop, Leptos SSR + PWA web | could | — | desktop + web attach to `polarisd` over named pipe with token auth; welcome page matches demo design |
| P10 | Hardening & submission: full test matrix, security review, budgets, `[profile.dist]`, demo + video | must | keys | all heavy suites run with evidence; cargo-deny clean; submission artifacts |

## 2. Critical path

```
P0 (skeleton ✔, WSL2, OpenShell, S0-1, S0-4, S0-7, keys requested)
  └─► P1 kernel core ─► P2 execution plane ─► P3 S2 loop (local OpenAI-compatible → Token Factory*) ─► P4 S1 ─► MVP demo
                                              └─ Contree* (E3)                                       │
                                                                                                     ▼
                                   P5 knowledge ─► P6 Nebius depth* ─► P7 advanced ─► (P8, P9) ─► P10 hardening + submission*
  * BLOCKED-ON-ACCESS until keys/accounts exist
```

## 2.1 Milestones

| ID | Milestone | Depends on | Evidence |
|---|---|---|---|
| M0 | Skeleton compiles; layers enforced; versions audited; resolution probe green | P0-SKEL-* | PHASES_STATUS P0-SKEL rows |
| M1 | TUI shell with sessions + directives (offline) | P1 crate tasks | recorded `--dump` snapshots + smoke |
| M2 | First sandboxed `cargo test` from the TUI | P2 | transcript + sandbox ids |
| M3 | Walking skeleton: agent edits & tests a repo via a local OpenAI-compatible endpoint | P3 (keyless part) | transcript |
| M4 | Same loop on Nemotron (Token Factory) + Contree run | P3 (keyed) | transcript + usage records |
| M5 | **MVP demo** (PLAN §11.7) | P4 | recorded video/transcript |
| M6 | Submission build + video | P10 | artifacts |

---

## P0 — Foundations & de-risking

**Entry:** repo at INITIAL COMMIT. **Exit gate:** every P0-SKEL task PASS; P0-ENV-1..6 PASS; every keyless
spike decided (PASS or named fallback chosen); P0-ACC requests submitted.

### P0 tasks

| Task | Description | Depends on | Exit criteria (evidence) | Access |
|---|---|---|---|---|
| P0-SKEL-1 | `.gitignore` root-anchored; Cargo.lock tracked; fixtures logs re-included | — | `git check-ignore` outputs | — |
| P0-SKEL-2 | `.gitattributes` (LF; CRLF for .ps1/.bat/.cmd; binaries) | — | file present | — |
| P0-SKEL-3 | `@prime/tools/workspace.toml` (93 packages, 156 external deps) | — | `workspace.py summary` | — |
| P0-SKEL-4 | `@prime/tools/workspace.py` (gen, check-members, check-layers, audit-versions, probe, chatlog-inventory, phase-tables, check-docs) | P0-SKEL-3 | commands exit 0 | — |
| P0-SKEL-5 | Root `Cargo.toml` + `rust-toolchain.toml` generated | P0-SKEL-4 | `cargo metadata` 93 packages | — |
| P0-SKEL-6 | Doc-only skeleton for all 93 packages | P0-SKEL-5 | `cargo check --workspace --all-targets` (-Dwarnings), `cargo doc` (-Dwarnings), `cargo fmt --check` | — |
| P0-SKEL-7 | Layer & member checks | P0-SKEL-6 | `check-members`, `check-layers` OK | — |
| P0-SKEL-8 | Version audit (sparse index) + resolution probe | P0-SKEL-3 | `audit-versions` 156/156; `probe` locks graph | — |
| P0-SKEL-9 | PLAN.md, PHASES.md, PHASES_STATUS.md; chatlog inventory 100 % covered | P0-SKEL-3 | `chatlog-inventory --check`, `check-docs` OK | — |
| P0-ACC-1 | Request Nebius Token Factory API key | — | key stored in keyring (never in git) | **requests access** |
| P0-ACC-2 | Request Token Factory Sandboxes (Contree) access | — | `contree-mcp` authenticates | **requests access** |
| P0-ACC-3 | Request Nebius AI Cloud quota (Linux VM; GPU optional) | — | VM can be created | **requests access** |
| P0-ACC-4 | Request Tavily API key | — | key stored in keyring | **requests access** |
| P0-ENV-1 | Install Ubuntu 24.04 in WSL2; `.wslconfig` (memory ≈ 6 GB, `nestedVirtualization=true`) | — | `wsl -l -v`; `/proc/meminfo`; `ls /dev/kvm` result recorded | — |
| P0-ENV-2 | Install OpenShell v0.1.4 in Ubuntu; start gateway (docker driver) | P0-ENV-1 | `openshell sandbox create` + exec + delete transcript | — |
| P0-ENV-3 | Tooling: cargo-nextest, cargo-fuzz (nightly, WSL2), cargo-mutants, cargo-leptos, cargo-deny, cargo-vet, cargo-audit, uv; kani in WSL2 | — | `--version` outputs | — |
| P0-ENV-4 | `.cargo/config.toml` `[env]`: `CMAKE_POLICY_VERSION_MINIMUM=3.5`, `PROTOC` (vendored path); short `target-dir`; `git config core.longpaths true` | — | config committed; a heavy dep builds | — |
| P0-ENV-5 | Linux check leg: `cargo check --workspace` inside WSL2 Ubuntu | P0-ENV-1 | transcript | — |
| P0-ENV-6 | Local OpenAI-compatible server for the P3 walking skeleton (Ollama or llama.cpp on GTX 1650) with a small coder model | — | `/v1/chat/completions` streaming transcript | — |
| P0-ENV-7 | `deny.toml` + cargo-vet policy; Slint licence decision (ADR-014) | P0-ENV-3 | `cargo deny check` output | — |
| P0-XT-1 | Rust `xtask` port of workspace.py checks (`check-members`, `check-layers`) | P0-SKEL-4 | `cargo xtask check-layers` OK | — |

### P0 spikes (each: question → kill criterion → fallback). Results go to PHASES_STATUS.md.

| Spike | Question | Kill criterion | Fallback | Access |
|---|---|---|---|---|
| S0-1 | Does `openshell-sdk` (rev 5a7728c3) build on `x86_64-pc-windows-msvc` and reach the WSL2 gateway (create/exec/stream/destroy)? | does not compile, or cannot connect within 1 day of effort | generate a tonic client from OpenShell `proto/` at the pinned rev | — |
| S0-2 | Is `/dev/kvm` available in WSL2 Ubuntu on the Ryzen 2700 and does the OpenShell `vm` driver boot a sandbox? | no `/dev/kvm` or vm driver fails | E4 remote gateway on Nebius AI Cloud for microVM work | AI Cloud for fallback |
| S0-3 | Can NemoClaw onboard a **custom** (Polaris) blueprint/agent? | unsupported/undocumented | native Polaris blueprint applier over OpenShell (`polaris-openshell-nemoclaw`) | — |
| S0-4 | GLiNER2.5-Decide → ONNX fragments for gliner2-rs: top-1 parity ≥ 99 % vs Python `gliner2` on a 200-item golden set; p95 latency CPU and CUDA EP (fp32/fp16); RSS | parity < 99 % or RSS > 2.5 GB | supervised Python `gliner2` sidecar (uv) | — |
| S0-5 | Token Factory: SSE streaming, tool calling, Responses API, `usage` (incl. cached tokens), rate-limit headers, batch API, reasoning controls per Nemotron model | — (measurement spike) | local prefix-stability metric; client-side coalescing; 429-driven limiter | **Token Factory key** |
| S0-6 | Cactus Needle 3 on Windows or WSL2: engine `--serve` protocol, embeddings, latency, RSS | no working build | defer Tier 0; Token Factory embeddings (D8) | — |
| S0-7 | Workspace sync Windows → sandbox: 10k-file repo initial/incremental time; byte-exact EOL/case; `.polarisignore` | incremental > 5 s for no-op, or any byte mismatch | tar-stream snapshot per run (slower, simpler) | — |
| S0-8 | iceoryx2 0.10 between two Windows processes: latency, robustness, cleanup after crash | crashes/leaks | gRPC over named pipes only | — |
| S0-9 | LanceDB 0.40 build on Windows (protoc, CMake 4, RAM, time) + query latency | build fails or > 30 min / > 12 GB RAM | usearch + SQLite metadata behind `VectorStore` | — |
| S0-10 | crossterm 0.29 on Windows Terminal/ConPTY: Shift+Enter (CSI-u), Shift+Tab, bracketed paste, Alt-codes | Shift+Enter unreliable | Alt+Enter / Ctrl+J newline; unsafe exception crate only if rejected | — |
| S0-11 | OpenShell `mxc` driver (Windows AppContainer) for Windows-native builds | not usable on this Windows edition | WSL2 tiers only | — |
| S0-12 | `polaris-guest` build in WSL2, packaging into the sandbox image, `protocol_version` handshake, Xvfb + Chromium screenshot | image build or handshake fails | exec-only guest without browser | — |
| S0-13 | Contree MCP: auth, `run`/`rsync`/`list_images`, branching via `directory_state_id`, SWE-bench image selection | cannot authenticate or branch | CLI wrapper or Python SDK sidecar; local OpenShell for demo | **Contree access** |

### P0 crate tasks (generated from workspace.toml)

<!-- generated: P0 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-tests` | `polaris-tests` | L8 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-storage`, `crate:polaris-kernel`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:xtask` | `xtask` | L8 | — | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |


---

## P1 — Kernel core (offline, keyless)

**Entry:** P0-SKEL PASS. **Exit gate:** M1 + Linux check leg green + counting-allocator tests for bus/log
enqueue + fuzz targets `directives_grammar`, `rtk_stream`, `cas_short_ref_roundtrip`, `eventlog_decode`,
`proto_decode`, `diff_apply` running in WSL2.

Cross-cutting tasks:

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P1-X-1 | Composition-root wiring v0 (`Backends` with local-only backends) | `crate:polaris-kernel` | session typestate tests PASS |
| P1-X-2 | TUI shell (composer, completion, `$`/`/`/`@`, sessions, `/status` `/help` `/stats` `/quit`) | `crate:polaris-ui-cli`, `crate:polaris-cli` | insta `--dump` snapshots + smoke |
| P1-X-3 | rustls aws-lc-rs provider installed in `main` (ADR-005) | `crate:polaris-cli` | smoke asserts provider |

### P1 crate tasks (generated from workspace.toml)

<!-- generated: P1 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-cli` | `polaris-cli` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-cli`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-sdk` | `polaris-sdk` | L0 | — | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-proto` | `polaris-proto` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-config` | `polaris-config` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-log` | `polaris-log` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-telemetry` | `polaris-telemetry` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-code-intel` | `polaris-code-intel` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-storage-cas` | `polaris-storage-cas` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-storage-eventlog` | `polaris-storage-eventlog` | L1 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-storage-registry` | `polaris-storage-registry` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-storage-cache` | `polaris-storage-cache` | L1 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-storage` | `polaris-storage` | L1 | `crate:polaris-storage-cas`, `crate:polaris-storage-eventlog`, `crate:polaris-storage-registry`, `crate:polaris-storage-cache`, `crate:polaris-storage-vector`, `crate:polaris-storage-sync` | re-exports compile; children's tasks PASS |
| `crate:polaris-kernel-bus` | `polaris-kernel-bus` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-kernel-directives` | `polaris-kernel-directives` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | L2 | `crate:polaris-sdk`, `crate:polaris-log`, `crate:polaris-telemetry`, `crate:polaris-kernel-bus` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | L2 | `crate:polaris-sdk`, `crate:polaris-log`, `crate:polaris-kernel-bus` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | L3 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | L3 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-tools` | `polaris-tools` | L3 | `crate:polaris-tools-builtin`, `crate:polaris-tools-math`, `crate:polaris-tools-rtk` | re-exports compile; children's tasks PASS |
| `crate:polaris-kernel` | `polaris-kernel` | L5 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-telemetry`, `crate:polaris-ratelimit`, `crate:polaris-kernel-bus`, `crate:polaris-kernel-directives`, `crate:polaris-kernel-supervisor`, `crate:polaris-kernel-reaper`, `crate:polaris-kernel-operators`, `crate:polaris-kernel-verifier`, `crate:polaris-storage`, `crate:polaris-security`, `crate:polaris-platform`, `crate:polaris-vcs`, `crate:polaris-openshell`, `crate:polaris-nebius-cloud`, `crate:polaris-system-one`, `crate:polaris-models`, `crate:polaris-tools`, `crate:polaris-search`, `crate:polaris-ext`, `crate:polaris-nvidia-docs`, `crate:polaris-browser`, `crate:polaris-devenv`, `crate:polaris-hacker`, `crate:polaris-runtime` | re-exports compile; children's tasks PASS |
| `crate:polaris-ui-model` | `polaris-ui-model` | L6 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ui` | `polaris-ui` | L6 | `crate:polaris-ui-model`, `crate:polaris-ui-cli`, `crate:polaris-ui-gui`, `crate:polaris-ui-web` | re-exports compile; children's tasks PASS |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | L8 | `crate:polaris-kernel-bus` | burst scenarios run; pass criteria met with evidence |


---

## P2 — Execution plane (keyless, local WSL2)

**Entry:** P1 exit, P0-ENV-2 PASS, S0-1 decided. **Exit gate:** M2; sync S0-7 criteria met in tests;
blast-radius approvals and Proof-of-Done gate demonstrated; reaper leaves no sandbox/process behind (stress).

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P2-X-1 | Polaris sandbox image (base `nvcr.io/nvidia/base/ubuntu:24.04` or NemoClaw base) + blueprint | `crate:polaris-openshell-nemoclaw` | image digest recorded |
| P2-X-2 | `/onboarding` + `polaris doctor` (WSL2, OpenShell, Docker, KVM, keys presence) | `crate:polaris-platform-wsl` | doctor report on this machine |

### P2 crate tasks (generated from workspace.toml)

<!-- generated: P2 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | L1 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-security-automata` | `polaris-security-automata` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-platform-wsl` | `polaris-platform-wsl` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-platform-process` | `polaris-platform-process` | L2 | `crate:polaris-sdk`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-platform` | `polaris-platform` | L2 | `crate:polaris-platform-wsl`, `crate:polaris-platform-process` | re-exports compile; children's tasks PASS |
| `crate:polaris-vcs` | `polaris-vcs` | L2 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-openshell-client` | `polaris-openshell-client` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-openshell-policy` | `polaris-openshell-policy` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-openshell-nemoclaw` | `polaris-openshell-nemoclaw` | L2 | `crate:polaris-sdk`, `crate:polaris-openshell-policy` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-openshell` | `polaris-openshell` | L2 | `crate:polaris-openshell-client`, `crate:polaris-openshell-policy`, `crate:polaris-openshell-nemoclaw` | re-exports compile; children's tasks PASS |
| `crate:polaris-security` | `polaris-security` | L3 | `crate:polaris-security-capability`, `crate:polaris-security-automata`, `crate:polaris-security-secrets`, `crate:polaris-security-pii`, `crate:polaris-security-rules` | re-exports compile; children's tasks PASS |
| `crate:polaris-runtime-leases` | `polaris-runtime-leases` | L4 | `crate:polaris-sdk`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime-guards` | `polaris-runtime-guards` | L4 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-tools-rtk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime` | `polaris-runtime` | L4 | `crate:polaris-runtime-frames`, `crate:polaris-runtime-context`, `crate:polaris-runtime-dag`, `crate:polaris-runtime-leases`, `crate:polaris-runtime-guards`, `crate:polaris-runtime-research` | re-exports compile; children's tasks PASS |
| `crate:polaris-spike-sandbox` | `polaris-spike-sandbox` | L8 | `crate:polaris-openshell` | burst scenarios run; pass criteria met with evidence |


---

## P3 — System Two walking skeleton → Token Factory; Contree

**Entry:** P2 exit, P0-ENV-6 PASS. **Exit gate:** M3 (keyless) and M4 (keyed: Token Factory + Contree).
Keyed items stay `BLOCKED-ON-ACCESS` until P0-ACC-1/2 are done.

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P3-X-1 | Walking skeleton against local OpenAI-compatible server | `crate:polaris-nebius-cloud-token-factory`, `crate:polaris-runtime-frames` | transcript: plan → edit → sandbox test → proof-of-done | — |
| P3-X-2 | Same loop on Nemotron 3 (effort tiers) | P3-X-1 | transcript + usage | **Token Factory key** |
| P3-X-3 | Contree sidecar + E3 run of a SWE-bench task | `crate:polaris-nebius-cloud-sandboxes` | transcript | **Contree** |
| P3-X-4 | Re-record synthetic SSE fixtures live; contract tests PASS against real service | P3-X-2 | contract suite output | **Token Factory key** |

### P3 crate tasks (generated from workspace.toml)

<!-- generated: P3 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-ratelimit` | `polaris-ratelimit` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-security-capability` | `polaris-security-capability` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-security-secrets` | `polaris-security-secrets` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-nebius-cloud-token-factory` | `polaris-nebius-cloud-token-factory` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-ratelimit`, `crate:polaris-telemetry` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-nebius-cloud-sandboxes` | `polaris-nebius-cloud-sandboxes` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-nebius-cloud` | `polaris-nebius-cloud` | L2 | `crate:polaris-nebius-cloud-token-factory`, `crate:polaris-nebius-cloud-sandboxes`, `crate:polaris-nebius-cloud-data-lab`, `crate:polaris-nebius-cloud-ai-cloud` | re-exports compile; children's tasks PASS |
| `crate:polaris-models` | `polaris-models` | L3 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-tools-math` | `polaris-tools-math` | L3 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime-frames` | `polaris-runtime-frames` | L4 | `crate:polaris-sdk`, `crate:polaris-security-capability`, `crate:polaris-models` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime-context` | `polaris-runtime-context` | L4 | `crate:polaris-sdk`, `crate:polaris-storage-cas`, `crate:polaris-code-intel`, `crate:polaris-tools-rtk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-kernel-operators` | `polaris-kernel-operators` | L4 | `crate:polaris-sdk`, `crate:polaris-security-pii`, `crate:polaris-system-one-router`, `crate:polaris-ratelimit`, `crate:polaris-kernel-directives` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | L4 | `crate:polaris-sdk`, `crate:polaris-security-capability`, `crate:polaris-code-intel`, `crate:polaris-system-one-verifier` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-spike-ratelimit` | `polaris-spike-ratelimit` | L8 | `crate:polaris-ratelimit` | burst scenarios run; pass criteria met with evidence |


---

## P4 — System One (+ MVP)

**Entry:** S0-4 decided; P3 keyless exit. **Exit gate:** S1 parity and latency evidence; fail-open/closed
matrix tests; PII-egress security tests (nothing PII-like leaves in captured requests); **M5 MVP demo**.

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P4-X-1 | ONNX export script (xtask/Python) + golden set generation | S0-4 | export hash + goldens committed (fixtures policy) |
| P4-X-2 | Threshold calibration v1 for routing, tool pruning, safety | `crate:polaris-system-one-schemas` | precision/recall table recorded |
| P4-X-3 | MVP scripted demo (PLAN §11.7) | P3-X-2, all P4 crate tasks | recorded transcript/video |

### P4 crate tasks (generated from workspace.toml)

<!-- generated: P4 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-worker` | `polaris-worker` | L7 | `crate:polaris-system-one-engine`, `crate:polaris-system-one-nano`, `crate:polaris-ingest`, `crate:polaris-proto`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-system-one-engine` | `polaris-system-one-engine` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one-client` | `polaris-system-one-client` | L2 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-storage-cache` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-security-pii` | `polaris-security-pii` | L3 | `crate:polaris-sdk`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-security-rules` | `polaris-security-rules` | L3 | `crate:polaris-sdk`, `crate:polaris-security-automata`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one-schemas` | `polaris-system-one-schemas` | L3 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one-router` | `polaris-system-one-router` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one-verifier` | `polaris-system-one-verifier` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas`, `crate:polaris-security-automata`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one-watchdog` | `polaris-system-one-watchdog` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-system-one` | `polaris-system-one` | L3 | `crate:polaris-system-one-client`, `crate:polaris-system-one-schemas`, `crate:polaris-system-one-router`, `crate:polaris-system-one-verifier`, `crate:polaris-system-one-watchdog` | re-exports compile; children's tasks PASS |
| `crate:polaris-spike-s1` | `polaris-spike-s1` | L8 | `crate:polaris-system-one-client` | burst scenarios run; pass criteria met with evidence |


---

## P5 — Knowledge (should)

**Entry:** P4 exit (or parallel after P3 for keyless parts). **Exit gate:** see §1.

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P5-X-1 | SearXNG container in WSL2 + provider | `crate:polaris-search-providers` | query transcript | — |
| P5-X-2 | Tavily provider live | P5-X-1 | query transcript | **Tavily key** |
| P5-X-3 | NVIDIA docs initial sync + offline seed artifact (not in git) | `crate:polaris-nvidia-docs` | page counts + provenance | — |
| P5-X-4 | Embeddings via Token Factory (PII-masked) | `crate:polaris-storage-vector` | embedding run evidence | **Token Factory key** |

### P5 crate tasks (generated from workspace.toml)

<!-- generated: P5 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-storage-vector` | `polaris-storage-vector` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ingest` | `polaris-ingest` | L2 | `crate:polaris-sdk`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-search-providers` | `polaris-search-providers` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-ratelimit` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-search-sift` | `polaris-search-sift` | L3 | `crate:polaris-sdk`, `crate:polaris-search-providers` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-search` | `polaris-search` | L3 | `crate:polaris-search-providers`, `crate:polaris-search-sift` | re-exports compile; children's tasks PASS |
| `crate:polaris-nvidia-docs` | `polaris-nvidia-docs` | L3 | `crate:polaris-sdk`, `crate:polaris-storage-registry`, `crate:polaris-storage-vector`, `crate:polaris-code-intel`, `crate:polaris-ratelimit` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |


---

## P6 — Nebius depth (should)

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P6-X-1 | SWE env resolver + `$nebius-branching` over `directory_state_id` | P3-X-3 | branch tree transcript | **Contree** |
| P6-X-2 | Remote OpenShell gateway on Nebius AI Cloud (E4) | `crate:polaris-nebius-cloud-ai-cloud` | remote sandbox transcript | **AI Cloud** |
| P6-X-3 | Data Lab export + fine-tune trigger (after API verification) | `crate:polaris-nebius-cloud-data-lab` | export evidence | **Data Lab access** |

### P6 crate tasks (generated from workspace.toml)

<!-- generated: P6 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-nebius-cloud-data-lab` | `polaris-nebius-cloud-data-lab` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-nebius-cloud-ai-cloud` | `polaris-nebius-cloud-ai-cloud` | L2 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |


---

## P7 — Advanced agentics (should)

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P7-X-1 | `/research` long run with checkpoint/resume | `crate:polaris-runtime-research` | ≥ 20 iterations, resume after kill |
| P7-X-2 | `/hack` on seeded-bug repos (race, blocking-in-async, CUDA barrier misuse) | `crate:polaris-hacker` | findings vs seeded list |
| P7-X-3 | microVM browser loop: screenshot + a11y + pHash `/ui-diff` | S0-2/S0-12, `crate:polaris-browser` | screenshots + diff report |

### P7 crate tasks (generated from workspace.toml)

<!-- generated: P7 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-guest` | `polaris-guest` | L7 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-browser`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-ext-mcp` | `polaris-ext-mcp` | L2 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ext-skills` | `polaris-ext-skills` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ext` | `polaris-ext` | L2 | `crate:polaris-ext-mcp`, `crate:polaris-ext-skills` | re-exports compile; children's tasks PASS |
| `crate:polaris-browser` | `polaris-browser` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-hacker` | `polaris-hacker` | L3 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime-dag` | `polaris-runtime-dag` | L4 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-runtime-research` | `polaris-runtime-research` | L4 | `crate:polaris-sdk`, `crate:polaris-runtime-frames`, `crate:polaris-runtime-context` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |


---

## P8 — DevEnv (could)

### P8 crate tasks (generated from workspace.toml)

<!-- generated: P8 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-devenv-nix` | `polaris-devenv-nix` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-devenv-docker` | `polaris-devenv-docker` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-devenv-cicd` | `polaris-devenv-cicd` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-devenv` | `polaris-devenv` | L2 | `crate:polaris-devenv-nix`, `crate:polaris-devenv-docker`, `crate:polaris-devenv-cicd` | re-exports compile; children's tasks PASS |
| `crate:polaris-system-one-nano` | `polaris-system-one-nano` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |


---

## P9 — Interfaces (could)

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P9-X-1 | Static welcome page (approved demo design) — may be pulled forward for the Design criterion | `crate:polaris-ui-web` | visual comparison to demo |
| P9-X-2 | Named-pipe transport with token auth; mTLS remote | `crate:polaris-rpc` | security tests |

### P9 crate tasks (generated from workspace.toml)

<!-- generated: P9 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria |
|---|---|---|---|---|
| `crate:polaris-desktop` | `polaris-desktop` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-gui`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-web` | `polaris-web` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-web`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-daemon` | `polaris-daemon` | L7 | `crate:polaris-kernel`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done |
| `crate:polaris-rpc` | `polaris-rpc` | L5 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-security-capability` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ui-gui` | `polaris-ui-gui` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-ui-web` | `polaris-ui-web` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths |
| `crate:polaris-spike-web` | `polaris-spike-web` | L8 | `crate:polaris-ui-web` | burst scenarios run; pass criteria met with evidence |


---

## P10 — Hardening & submission (must)

| Task | Description | Exit criteria | Access |
|---|---|---|---|
| P10-1 | Full test matrix: load, scalability, stress, spike, fuzz (WSL2), Kani, Miri, loom, mutants | evidence per suite | — |
| P10-2 | Security review (threat model §6.11, unsafe count, cargo-deny/vet/audit, licences ADR-014) | report | — |
| P10-3 | Performance budgets (PLAN §11.4) measured and recorded | table with numbers | — |
| P10-4 | `[profile.dist]` submission build; installer/readme | artifacts + SHA256 | — |
| P10-5 | Demo script + video; fallback path for judges without WSL2 (E3/E4) | recorded | **keys** |
| P10-6 | Linux/macOS adaptation start (seams validated) | Linux build leg green | — |

---

## v2 (declared, not scheduled)

Self-hosting, Slint mobile, voice (`polaris-audio`, `polaris-stt`), Telegram bot, WireGuard mesh,
cellular-resilient sessions, push notifications, macOS/Linux first-class — PLAN.md §16.
