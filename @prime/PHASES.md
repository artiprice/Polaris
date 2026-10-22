# Polaris — Phases

> **Document:** `@prime/PHASES.md` · **Date:** 2026-10-20 · **Companions:** `@prime/PLAN.md` (what & why),
> `@prime/PHASES_STATUS.md` (evidence-backed status). This file is edited by hand **except** the blocks that
> start with `<!-- generated: … -->` (crate task tables, §0.1 member-setup table): they come from
> `@prime/tools/workspace.toml` and are refreshed in place by `python -I @prime/tools/workspace.py render-docs`
> (`render-docs --check` fails when they are stale; `check-docs` verifies every package and setup id appears).

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
- **Where** (team work): every task and setup item is marked with where it lives —
  **per-PC** = each team member installs/does it on their own machine (`[per-PC]`, see §0.1);
  **repo** = done once, committed, arrives with `git pull`;
  **team account** = requested once for the team; each member then stores their own key in their keyring.

## 0.1 Team member setup (per-PC)

Every team member needs these on **their own PC**; nothing in this list is committed to git. The list is
generated from `[[member_setup]]` in `@prime/tools/workspace.toml`; crate tasks below name the ids they need in
their "Per-PC setup" column.

**Quick start** (Windows 11, from the repo root, normal non-admin PowerShell; winget may raise UAC prompts):

```powershell
# clone with long paths enabled: git clone -c core.longpaths=true <repo-url>   (the script re-applies it)
powershell -NoProfile -ExecutionPolicy Bypass -File @prime\tools\setup\setup-windows.ps1                       # installs + verifies
powershell -NoProfile -ExecutionPolicy Bypass -File @prime\tools\setup\setup-windows.ps1 -RegisterKeepalive    # optional: WSL keepalive task
powershell -NoProfile -ExecutionPolicy Bypass -File @prime\tools\setup\setup-windows.ps1 -Check                # verify only
```

`setup-windows.ps1` installs the Windows-side items, creates the WSL2 `Ubuntu-24.04` distro (first install may
need a reboot — then re-run), bootstraps the `dev` user and runs `@prime/tools/setup/setup-wsl.sh` inside it
(Docker Engine, OpenShell 0.1.4 + gateway service + driver pin, Rust + nightly + nextest + cargo-fuzz;
`--with-kani` adds Kani). Both scripts are idempotent. Then verify:

```powershell
wsl -d Ubuntu-24.04 -- bash @prime/tools/setup/verify-openshell.sh     # sandbox create/exec/upload/delete
wsl -d Ubuntu-24.04 -- bash @prime/tools/setup/verify-linux-leg.sh     # Linux cargo check/doc/fmt leg
python -I @prime/tools/setup/verify-local-llm.py                       # Ollama streaming + tool call
cargo xtask check-members; cargo xtask check-layers
```

Notes:
- The `dev` WSL user gets **passwordless sudo and a locked password** so `setup-wsl.sh` runs unattended; set a
  password afterwards if your policy requires one (`wsl -d Ubuntu-24.04 -u root passwd dev`).
- Each PC's OpenShell gateway generates **its own mTLS CA and client certificate**
  (`~/.config/openshell/gateways/openshell/mtls` in WSL2); Polaris reads them at runtime — never copy them
  between machines or into the repo.
- API keys are **per member**: each stores their own in the Windows Credential Manager (`win-keyring`); keys
  never go into git, `.env` files, CLI arguments or logs.
- Machine-specific Cargo settings (`target-dir`, a `PROTOC` path) go in the member's user-level Cargo config or
  environment, never in the repo's `.cargo/config.toml`.

<!-- generated: member setup (workspace.toml [[member_setup]]) -->
| Id | Where | Component | Version (verified) | Install | From | Required |
|---|---|---|---|---|---|---|
| `win-msvc` | Windows | Visual Studio Build Tools, VC tools x64 (MSVC linker; C/C++ for aws-lc-sys, lance, ort) | 18.11 (VS 2026) | setup-windows.ps1 (winget Microsoft.VisualStudio.BuildTools + VCTools workload) | P0 | **yes** |
| `win-git` | Windows | Git for Windows | 2.52.0 | setup-windows.ps1 (winget Git.Git) | P0 | **yes** |
| `win-rust` | Windows | rustup + Rust 1.99.0 (rustfmt, clippy); pinned by rust-toolchain.toml | rustup 1.29.1, rustc 1.99.0 | setup-windows.ps1 (winget Rustlang.Rustup) | P0 | **yes** |
| `win-python` | Windows | Python 3.14 (workspace.py tooling; uv host) | 3.14.5 | setup-windows.ps1 (winget Python.Python.3.14) | P0 | **yes** |
| `win-cmake-nasm` | Windows | CMake + NASM + Ninja for native build scripts (CMake 4 relies on the repo's CMAKE_POLICY_VERSION_MINIMUM) | CMake 4.3.0-rc3, NASM 3.01, Ninja 1.13.2 | setup-windows.ps1 (winget Kitware.CMake, NASM.NASM, Ninja-build.Ninja) | P0 | **yes** |
| `win-protoc` | Windows | protoc on PATH, only for the opt-in `lancedb` feature (S0-9 failed without it); polaris-proto uses protoc-bin-vendored | libprotoc 36.0 | setup-windows.ps1 (winget Google.Protobuf) | P0 | optional |
| `win-longpaths` | Windows | LongPathsEnabled=1 (admin, once per PC) + repo-local `git config core.longpaths true` | - | setup-windows.ps1 sets the git flag and prints the admin command for the registry value | P0 | **yes** |
| `win-cargo-tools` | Windows | cargo-binstall, cargo-nextest, cargo-mutants, cargo-leptos, cargo-deny, cargo-vet, cargo-audit | 1.26.0 / 0.9.148 / 27.1.0 / 0.3.11 / 0.20.2 / 0.10.2 / 0.22.2 | setup-windows.ps1 (`cargo install --locked cargo-binstall`; `cargo binstall <tool>@<version>`) | P0 | **yes** |
| `win-uv` | Windows | uv for Python envs (spikes, ONNX export script, contree-mcp sidecar); run as `python -m uv` | 0.13.3 | setup-windows.ps1 (`python -m pip install --user uv==0.13.3`) | P0 | **yes** |
| `win-ollama` | Windows | Ollama + `nemotron-mini`: keyless OpenAI-compatible endpoint for the P3 walking skeleton | Ollama 0.40.6; nemotron-mini 2.7 GB | setup-windows.ps1 (winget Ollama.Ollama; `ollama pull nemotron-mini`); `-SkipOllama` defers it | P0 | optional |
| `win-keyring` | Windows | API keys in Windows Credential Manager (Token Factory, Tavily, Contree): each member stores their own; never in git, env files or logs | - | `polaris /onboarding` (P2-X-2) once keys exist; BLOCKED-ON-ACCESS until then | P3 | optional |
| `win-models` | Windows | Model weights under `%USERPROFILE%\.polaris\models` (GLiNER2.5-Decide ONNX + onnxruntime DLL; Needle 3 `.cact`); downloaded, never in git | per spike results S0-4 / S0-6 | P4-X-1 export script (not automated yet) | P4 | optional |
| `win-cuda` | Windows | CUDA runtime + cuDNN for the ort CUDA execution provider (optional; the CPU EP is the default) | not installed; decided by S0-4 | manual NVIDIA installers, only if S0-4 recommends the CUDA EP | P4 | optional |
| `win-pdfium` | Windows | pdfium DLL beside the binary (pdfium-render, scanned PDFs) | not installed | to be scripted in P5 (polaris-ingest) | P5 | optional |
| `win-wasm32` | Windows | Rust target wasm32-unknown-unknown (Leptos hydrate build via cargo-leptos) | not installed | `rustup target add wasm32-unknown-unknown --toolchain 1.99.0` | P9 | optional |
| `wsl-distro` | WSL2 | WSL2 + Ubuntu-24.04, default user `dev` (sudo, kvm, docker), systemd on | WSL 2.6.3; Ubuntu 24.04.5 LTS, kernel 6.6.87.2 | setup-windows.ps1 (`wsl --install -d Ubuntu-24.04 --no-launch` + user bootstrap) | P0 | **yes** |
| `wsl-docker` | WSL2 | Docker Engine inside Ubuntu (OpenShell docker driver); Docker Desktop not required | 29.9.1 | setup-wsl.sh step 2 | P0 | **yes** |
| `wsl-openshell` | WSL2 | OpenShell CLI + gateway (systemd user service, linger, `OPENSHELL_COMPUTE_DRIVER=docker` pin) | 0.1.4 | setup-wsl.sh steps 3-4 | P0 | **yes** |
| `wsl-keepalive` | Windows | Keep the WSL2 distro (and the gateway in it) from idling out (pitfall W14) | - | setup-windows.ps1 `-RegisterKeepalive` (per-user logon task), or keep a WSL session open | P0 | optional |
| `wsl-rust` | WSL2 | Rust 1.99.0 + nightly (rust-src, miri), cargo-binstall, cargo-nextest, cargo-fuzz (Linux check leg, fuzzing, guest build) | 1.99.0; nightly 1.101.0; nextest 0.9.148; cargo-fuzz 0.13.2 | setup-wsl.sh steps 5-6 | P0 | **yes** |
| `wsl-kani` | WSL2 | Kani verifier (+ CBMC) | Kani 0.68.0, CBMC 6.11 | setup-wsl.sh `--with-kani` | P1 | optional |
| `wsl-kvm` | WSL2 | `/dev/kvm` in WSL2 for the OpenShell vm driver (microVM tier E2); CPU virtualization enabled in BIOS | present on the reference machine (S0-2 PASS) | hardware/BIOS; checked by `polaris doctor` (P2-X-2) | P7 | optional |
| `wsl-sandbox-image` | WSL2 | Polaris sandbox image (base `nvcr.io/nvidia/base/ubuntu:24.04` + guest + Xvfb/Chromium), built locally unless the team agrees on a registry | not built yet (P2-X-1, S0-12) | to be scripted in P2 | P2 | optional |
| `wsl-searxng` | WSL2 | SearXNG container (local search provider) | not installed | to be scripted in P5 (P5-X-1) | P5 | optional |
| `wsl-nix` | WSL2 | Nix package manager (`$env(nix)`, `/nix-flake`) | not installed | to be scripted in P8 | P8 | optional |
| `contree-mcp` | Windows | contree-mcp sidecar (Token Factory Sandboxes MCP server) | not installed (needs Contree access) | `python -m uv tool install contree-mcp` once access exists (P3-X-3) | P3 | optional |

**Repo-level (done once, committed — nothing to install):** `Cargo.lock`, `rust-toolchain.toml` (1.99.0),
`.cargo/config.toml` (`CMAKE_POLICY_VERSION_MINIMUM=3.5`, `cargo xtask` alias), `.gitattributes`, `deny.toml`,
`supply-chain/` (cargo-vet config, imports, exemptions), `@prime/tools/setup/*`.

**Team accounts (requested once — P0-ACC-1..4):** Nebius Token Factory API key, Token Factory Sandboxes
(Contree) access, Nebius AI Cloud quota, Tavily API key.

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

| Task | Description | Depends on | Exit criteria (evidence) | Access | Where |
|---|---|---|---|---|---|
| P0-SKEL-1 | `.gitignore` root-anchored; Cargo.lock tracked; fixtures logs re-included | — | `git check-ignore` outputs | — | repo |
| P0-SKEL-2 | `.gitattributes` (LF; CRLF for .ps1/.bat/.cmd; binaries) | — | file present | — | repo |
| P0-SKEL-3 | `@prime/tools/workspace.toml` (93 packages, 156 external deps, `[[member_setup]]`) | — | `workspace.py summary` | — | repo |
| P0-SKEL-4 | `@prime/tools/workspace.py` (gen, check-members, check-layers, audit-versions, probe, chatlog-inventory, phase-tables, check-docs, render-docs) | P0-SKEL-3 | commands exit 0 | — | repo |
| P0-SKEL-5 | Root `Cargo.toml` + `rust-toolchain.toml` generated | P0-SKEL-4 | `cargo metadata` 93 packages | — | repo |
| P0-SKEL-6 | Doc-only skeleton for all 93 packages | P0-SKEL-5 | `cargo check --workspace --all-targets` (-Dwarnings), `cargo doc` (-Dwarnings), `cargo fmt --check` | — | repo |
| P0-SKEL-7 | Layer & member checks | P0-SKEL-6 | `check-members`, `check-layers` OK | — | repo |
| P0-SKEL-8 | Version audit (sparse index) + resolution probe | P0-SKEL-3 | `audit-versions` 156/156; `probe` locks graph | — | repo |
| P0-SKEL-9 | PLAN.md, PHASES.md, PHASES_STATUS.md; chatlog inventory 100 % covered | P0-SKEL-3 | `chatlog-inventory --check`, `check-docs`, `render-docs --check` OK | — | repo |
| P0-ACC-1 | Request Nebius Token Factory API key | — | key stored in keyring (never in git) | **requests access** | team account → per-PC keyring |
| P0-ACC-2 | Request Token Factory Sandboxes (Contree) access | — | `contree-mcp` authenticates | **requests access** | team account → per-PC keyring |
| P0-ACC-3 | Request Nebius AI Cloud quota (Linux VM; GPU optional) | — | VM can be created | **requests access** | team account |
| P0-ACC-4 | Request Tavily API key | — | key stored in keyring | **requests access** | team account → per-PC keyring |
| P0-ENV-1 | Ubuntu 24.04 in WSL2 with user `dev` (sudo, kvm, docker) and systemd; `.wslconfig` only if RAM/KVM require it (not needed on the reference PC) | — | `wsl -l -v`; `/proc/meminfo`; `ls /dev/kvm` recorded | — | **per-PC** `wsl-distro` |
| P0-ENV-2 | Docker Engine + OpenShell v0.1.4 in Ubuntu; gateway user service, linger, `OPENSHELL_COMPUTE_DRIVER=docker` pin; WSL keepalive | P0-ENV-1 | `verify-openshell.sh` create + exec + upload + delete transcript | — | **per-PC** `wsl-docker`, `wsl-openshell`, `wsl-keepalive` |
| P0-ENV-3 | Tooling: cargo-binstall, cargo-nextest, cargo-mutants, cargo-leptos, cargo-deny, cargo-vet, cargo-audit, uv, protoc (Windows); Rust 1.99 + nightly, nextest, cargo-fuzz, kani (WSL2); `setup-windows.ps1` + `setup-wsl.sh` | — | `--version` outputs; `setup-windows.ps1 -Check` all ok | — | **per-PC** `win-*`, `wsl-rust`, `wsl-kani` |
| P0-ENV-4 | `.cargo/config.toml` `[env] CMAKE_POLICY_VERSION_MINIMUM=3.5` (repo); `PROTOC` on PATH and `core.longpaths` (per-PC); short `target-dir` not needed (measured) | — | config committed; a heavy dep builds | — | repo + **per-PC** `win-protoc`, `win-longpaths` |
| P0-ENV-5 | Linux check leg: `cargo check/doc/fmt` inside WSL2 Ubuntu (`verify-linux-leg.sh`) | P0-ENV-1 | transcript | — | **per-PC** `wsl-rust` |
| P0-ENV-6 | Local OpenAI-compatible server for the P3 walking skeleton (Ollama + `nemotron-mini` on the GTX 1650) | — | `verify-local-llm.py` streaming + tool-call transcript | — | **per-PC** `win-ollama` |
| P0-ENV-7 | `deny.toml` + cargo-vet policy (imports + exemptions); Slint licence decision (ADR-014) | P0-ENV-3 | `cargo deny check`, `cargo vet` output | — | repo |
| P0-XT-1 | Rust `xtask` port of workspace.py checks (`check-members`, `check-layers`), test-first | P0-SKEL-4 | `cargo test -p xtask`; `cargo xtask check-layers` OK | — | repo |

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
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-tests` | `polaris-tests` | L8 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-storage`, `crate:polaris-kernel`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-rust`, `wsl-kani` |
| `crate:xtask` | `xtask` | L8 | — | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-python` |


---

## P1 — Kernel core (offline, keyless)

**Entry:** P0-SKEL PASS. **Exit gate:** M1 + Linux check leg green + counting-allocator tests for bus/log
enqueue + fuzz targets `directives_grammar`, `rtk_stream`, `cas_short_ref_roundtrip`, `eventlog_decode`,
`proto_decode`, `diff_apply` running in WSL2.

**Current user scope (2026-10-22): complete P1 logic and defer UI.** The non-UI verdict is tracked in
[P1_LOGIC_STATUS.md](P1_LOGIC_STATUS.md). Full M1/TUI acceptance remains open. P1 composition uses
`LocalBackends` for real offline stores, controls, read tools and native ownership; complete
model/sandbox/S1 execution bundles retain their P2/P3/P4 gates. Facade export verification is separate
from later-phase child completion; no future child is marked PASS by a P1 facade doctest.

Cross-cutting tasks:

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P1-X-1 | Composition-root wiring v0 (`LocalBackends` offline profile) | `crate:polaris-kernel` | session typestate tests PASS |
| P1-X-2 | TUI shell (composer, completion, `$`/`/`/`@`, sessions, `/status` `/help` `/stats` `/quit`) | `crate:polaris-ui-cli`, `crate:polaris-cli` | insta `--dump` snapshots + smoke |
| P1-X-3 | rustls aws-lc-rs provider installed in `main` (ADR-005) | `crate:polaris-cli` | smoke asserts provider |

### P1 crate tasks (generated from workspace.toml)

<!-- generated: P1 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-cli` | `polaris-cli` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-cli`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-sdk`, `crate:polaris-security`, `crate:polaris-tools-rtk`, `crate:polaris-storage-cas`, `crate:polaris-openshell`, `crate:polaris-platform` | real `main`; smoke test PASS; PLAN §13 TODO done | — |
| `crate:polaris-sdk` | `polaris-sdk` | L0 | — | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-proto` | `polaris-proto` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-config` | `polaris-config` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-log` | `polaris-log` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-telemetry` | `polaris-telemetry` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-code-intel` | `polaris-code-intel` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-storage-cas` | `polaris-storage-cas` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-storage-eventlog` | `polaris-storage-eventlog` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-storage-registry` | `polaris-storage-registry` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-storage-cache` | `polaris-storage-cache` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-storage` | `polaris-storage` | L1 | `crate:polaris-storage-cas`, `crate:polaris-storage-eventlog`, `crate:polaris-storage-registry`, `crate:polaris-storage-cache`, `crate:polaris-storage-vector`, `crate:polaris-storage-sync` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-kernel-bus` | `polaris-kernel-bus` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-kernel-directives` | `polaris-kernel-directives` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | L2 | `crate:polaris-sdk`, `crate:polaris-log`, `crate:polaris-telemetry`, `crate:polaris-kernel-bus` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | L2 | `crate:polaris-sdk`, `crate:polaris-log`, `crate:polaris-kernel-bus` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | L3 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-security-automata`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | L3 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-tools` | `polaris-tools` | L3 | `crate:polaris-tools-builtin`, `crate:polaris-tools-math`, `crate:polaris-tools-rtk` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-kernel` | `polaris-kernel` | L5 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-telemetry`, `crate:polaris-ratelimit`, `crate:polaris-kernel-bus`, `crate:polaris-kernel-directives`, `crate:polaris-kernel-supervisor`, `crate:polaris-kernel-reaper`, `crate:polaris-kernel-operators`, `crate:polaris-kernel-verifier`, `crate:polaris-storage`, `crate:polaris-security`, `crate:polaris-platform`, `crate:polaris-vcs`, `crate:polaris-openshell`, `crate:polaris-nebius-cloud`, `crate:polaris-system-one`, `crate:polaris-models`, `crate:polaris-tools`, `crate:polaris-search`, `crate:polaris-ext`, `crate:polaris-nvidia-docs`, `crate:polaris-browser`, `crate:polaris-devenv`, `crate:polaris-hacker`, `crate:polaris-runtime` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-ui-model` | `polaris-ui-model` | L6 | `crate:polaris-sdk`, `crate:polaris-kernel-directives` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ui` | `polaris-ui` | L6 | `crate:polaris-ui-model`, `crate:polaris-ui-cli`, `crate:polaris-ui-gui`, `crate:polaris-ui-web` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | L8 | `crate:polaris-kernel-bus`, `crate:polaris-sdk` | burst scenarios run; pass criteria met with evidence | — |


---

## P2 — Execution plane (keyless, local WSL2)

**Entry:** P1 exit, P0-ENV-2 PASS, S0-1 decided. **Exit gate:** M2; sync S0-7 criteria met in tests;
blast-radius approvals and Proof-of-Done gate demonstrated; reaper leaves no sandbox/process behind (stress).

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P2-X-1 | Polaris sandbox image (base `nvcr.io/nvidia/base/ubuntu:24.04` or NemoClaw base) + blueprint; binaries under `/usr` (S0-12: `/opt` is not executable under the default policy) — [per-PC] `wsl-sandbox-image` unless a registry is agreed | `crate:polaris-openshell-nemoclaw` | image digest recorded |
| P2-X-2 | `/onboarding` + `polaris doctor` (WSL2, OpenShell, Docker, KVM, keys presence) — checks every §0.1 item on the member's PC | `crate:polaris-platform-wsl` | doctor report on this machine |

### P2 crate tasks (generated from workspace.toml)

<!-- generated: P2 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-openshell` |
| `crate:polaris-security-automata` | `polaris-security-automata` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-platform-wsl` | `polaris-platform-wsl` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-distro`, `wsl-keepalive` |
| `crate:polaris-platform-process` | `polaris-platform-process` | L2 | `crate:polaris-sdk`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-platform` | `polaris-platform` | L2 | `crate:polaris-platform-wsl`, `crate:polaris-platform-process` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-vcs` | `polaris-vcs` | L2 | `crate:polaris-sdk`, `crate:polaris-storage-cas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-openshell-client` | `polaris-openshell-client` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-openshell-policy` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-openshell`, `wsl-docker` |
| `crate:polaris-openshell-policy` | `polaris-openshell-policy` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-openshell` |
| `crate:polaris-openshell-nemoclaw` | `polaris-openshell-nemoclaw` | L2 | `crate:polaris-sdk`, `crate:polaris-openshell-policy`, `crate:polaris-openshell-client` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-openshell`, `wsl-sandbox-image` |
| `crate:polaris-openshell` | `polaris-openshell` | L2 | `crate:polaris-openshell-client`, `crate:polaris-openshell-policy`, `crate:polaris-openshell-nemoclaw` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-security` | `polaris-security` | L3 | `crate:polaris-security-capability`, `crate:polaris-security-automata`, `crate:polaris-security-secrets`, `crate:polaris-security-pii`, `crate:polaris-security-rules` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-runtime-leases` | `polaris-runtime-leases` | L4 | `crate:polaris-sdk`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime-guards` | `polaris-runtime-guards` | L4 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-tools-rtk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime` | `polaris-runtime` | L4 | `crate:polaris-runtime-frames`, `crate:polaris-runtime-context`, `crate:polaris-runtime-dag`, `crate:polaris-runtime-leases`, `crate:polaris-runtime-guards`, `crate:polaris-runtime-research` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-spike-sandbox` | `polaris-spike-sandbox` | L8 | `crate:polaris-openshell` | burst scenarios run; pass criteria met with evidence | — |


---

## P3 — System Two walking skeleton → Token Factory; Contree

**Entry:** P2 exit, P0-ENV-6 PASS. **Exit gate:** M3 (keyless) and M4 (keyed: Token Factory + Contree).
Keyed items stay `BLOCKED-ON-ACCESS` until P0-ACC-1/2 are done.

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P3-X-1 | Walking skeleton against local OpenAI-compatible server — [per-PC] `win-ollama` | `crate:polaris-nebius-cloud-token-factory`, `crate:polaris-runtime-frames` | transcript: plan → edit → sandbox test → proof-of-done | — |
| P3-X-2 | Same loop on Nemotron 3 (effort tiers) | P3-X-1 | transcript + usage | **Token Factory key** |
| P3-X-3 | Contree sidecar + E3 run of a SWE-bench task — [per-PC] `contree-mcp`, `win-keyring` | `crate:polaris-nebius-cloud-sandboxes` | transcript | **Contree** |
| P3-X-4 | Re-record synthetic SSE fixtures live; contract tests PASS against real service | P3-X-2 | contract suite output | **Token Factory key** |

### P3 crate tasks (generated from workspace.toml)

<!-- generated: P3 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-ratelimit` | `polaris-ratelimit` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-security-capability` | `polaris-security-capability` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-security-secrets` | `polaris-security-secrets` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-keyring` |
| `crate:polaris-nebius-cloud-token-factory` | `polaris-nebius-cloud-token-factory` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log`, `crate:polaris-ratelimit`, `crate:polaris-telemetry` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-ollama`, `win-keyring` |
| `crate:polaris-nebius-cloud-sandboxes` | `polaris-nebius-cloud-sandboxes` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `contree-mcp`, `win-uv`, `win-keyring` |
| `crate:polaris-nebius-cloud` | `polaris-nebius-cloud` | L2 | `crate:polaris-nebius-cloud-token-factory`, `crate:polaris-nebius-cloud-sandboxes`, `crate:polaris-nebius-cloud-data-lab`, `crate:polaris-nebius-cloud-ai-cloud` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-models` | `polaris-models` | L3 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-tools-math` | `polaris-tools-math` | L3 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime-frames` | `polaris-runtime-frames` | L4 | `crate:polaris-sdk`, `crate:polaris-security-capability`, `crate:polaris-models` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime-context` | `polaris-runtime-context` | L4 | `crate:polaris-sdk`, `crate:polaris-storage-cas`, `crate:polaris-code-intel`, `crate:polaris-tools-rtk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-kernel-operators` | `polaris-kernel-operators` | L4 | `crate:polaris-sdk`, `crate:polaris-security-pii`, `crate:polaris-system-one-router`, `crate:polaris-system-one-schemas`, `crate:polaris-ratelimit`, `crate:polaris-kernel-directives` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | L4 | `crate:polaris-sdk`, `crate:polaris-security-capability`, `crate:polaris-code-intel`, `crate:polaris-system-one-verifier` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-spike-ratelimit` | `polaris-spike-ratelimit` | L8 | `crate:polaris-ratelimit` | burst scenarios run; pass criteria met with evidence | — |


---

## P4 — System One (+ MVP)

**Entry:** S0-4 decided; P3 keyless exit. **Exit gate:** S1 parity and latency evidence; fail-open/closed
matrix tests; PII-egress security tests (nothing PII-like leaves in captured requests); **M5 MVP demo**.

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P4-X-1 | ONNX export script (xtask/Python) + golden set generation; export needs ≈ 8 GB free RAM (S0-4) — [per-PC] `win-models`, `win-uv` | S0-4 | export hash + goldens committed (fixtures policy) |
| P4-X-2 | Threshold calibration v1 for routing, tool pruning, safety | `crate:polaris-system-one-schemas` | precision/recall table recorded |
| P4-X-3 | MVP scripted demo (PLAN §11.7) | P3-X-2, all P4 crate tasks | recorded transcript/video |

### P4 crate tasks (generated from workspace.toml)

<!-- generated: P4 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-worker` | `polaris-worker` | L7 | `crate:polaris-system-one-engine`, `crate:polaris-system-one-nano`, `crate:polaris-ingest`, `crate:polaris-proto`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done | `win-models` |
| `crate:polaris-system-one-engine` | `polaris-system-one-engine` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-models`, `win-cuda` |
| `crate:polaris-system-one-client` | `polaris-system-one-client` | L2 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-storage-cache` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-security-pii` | `polaris-security-pii` | L3 | `crate:polaris-sdk`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-security-rules` | `polaris-security-rules` | L3 | `crate:polaris-sdk`, `crate:polaris-security-automata`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-system-one-schemas` | `polaris-system-one-schemas` | L3 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-system-one-router` | `polaris-system-one-router` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-system-one-verifier` | `polaris-system-one-verifier` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas`, `crate:polaris-security-automata`, `crate:polaris-code-intel` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-system-one-watchdog` | `polaris-system-one-watchdog` | L3 | `crate:polaris-sdk`, `crate:polaris-system-one-schemas` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-system-one` | `polaris-system-one` | L3 | `crate:polaris-system-one-client`, `crate:polaris-system-one-schemas`, `crate:polaris-system-one-router`, `crate:polaris-system-one-verifier`, `crate:polaris-system-one-watchdog` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-spike-s1` | `polaris-spike-s1` | L8 | `crate:polaris-system-one-client` | burst scenarios run; pass criteria met with evidence | — |


---

## P5 — Knowledge (should)

**Entry:** P4 exit (or parallel after P3 for keyless parts). **Exit gate:** see §1.

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P5-X-1 | SearXNG container in WSL2 + provider — [per-PC] `wsl-searxng` | `crate:polaris-search-providers` | query transcript | — |
| P5-X-2 | Tavily provider live | P5-X-1 | query transcript | **Tavily key** |
| P5-X-3 | NVIDIA docs initial sync + offline seed artifact (not in git) | `crate:polaris-nvidia-docs` | page counts + provenance | — |
| P5-X-4 | Embeddings via Token Factory (PII-masked) | `crate:polaris-storage-vector` | embedding run evidence | **Token Factory key** |

### P5 crate tasks (generated from workspace.toml)

<!-- generated: P5 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-storage-vector` | `polaris-storage-vector` | L1 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-protoc`, `win-cmake-nasm` |
| `crate:polaris-ingest` | `polaris-ingest` | L2 | `crate:polaris-sdk`, `crate:polaris-log` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-pdfium` |
| `crate:polaris-search-providers` | `polaris-search-providers` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-ratelimit` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-searxng`, `win-keyring` |
| `crate:polaris-search-sift` | `polaris-search-sift` | L3 | `crate:polaris-sdk`, `crate:polaris-search-providers` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-search` | `polaris-search` | L3 | `crate:polaris-search-providers`, `crate:polaris-search-sift` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-nvidia-docs` | `polaris-nvidia-docs` | L3 | `crate:polaris-sdk`, `crate:polaris-storage-registry`, `crate:polaris-storage-vector`, `crate:polaris-code-intel`, `crate:polaris-ratelimit` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |


---

## P6 — Nebius depth (should)

| Task | Description | Depends on | Exit criteria | Access |
|---|---|---|---|---|
| P6-X-1 | SWE env resolver + `$nebius-branching` over `directory_state_id` | P3-X-3 | branch tree transcript | **Contree** |
| P6-X-2 | Remote OpenShell gateway on Nebius AI Cloud (E4) | `crate:polaris-nebius-cloud-ai-cloud` | remote sandbox transcript | **AI Cloud** |
| P6-X-3 | Data Lab export + fine-tune trigger (after API verification) | `crate:polaris-nebius-cloud-data-lab` | export evidence | **Data Lab access** |

### P6 crate tasks (generated from workspace.toml)

<!-- generated: P6 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-nebius-cloud-data-lab` | `polaris-nebius-cloud-data-lab` | L2 | `crate:polaris-sdk`, `crate:polaris-config`, `crate:polaris-storage-cas`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-keyring` |
| `crate:polaris-nebius-cloud-ai-cloud` | `polaris-nebius-cloud-ai-cloud` | L2 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-keyring` |


---

## P7 — Advanced agentics (should)

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P7-X-1 | `/research` long run with checkpoint/resume | `crate:polaris-runtime-research` | ≥ 20 iterations, resume after kill |
| P7-X-2 | `/hack` on seeded-bug repos (race, blocking-in-async, CUDA barrier misuse) | `crate:polaris-hacker` | findings vs seeded list |
| P7-X-3 | microVM browser loop: screenshot + a11y + pHash `/ui-diff` — [per-PC] `wsl-kvm`, `wsl-sandbox-image` | S0-2/S0-12, `crate:polaris-browser` | screenshots + diff report |

### P7 crate tasks (generated from workspace.toml)

<!-- generated: P7 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-guest` | `polaris-guest` | L7 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-browser`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done | `wsl-rust`, `wsl-sandbox-image` |
| `crate:polaris-ext-mcp` | `polaris-ext-mcp` | L2 | `crate:polaris-sdk`, `crate:polaris-config` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ext-skills` | `polaris-ext-skills` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ext` | `polaris-ext` | L2 | `crate:polaris-ext-mcp`, `crate:polaris-ext-skills` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-browser` | `polaris-browser` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-sandbox-image`, `wsl-kvm` |
| `crate:polaris-hacker` | `polaris-hacker` | L3 | `crate:polaris-sdk`, `crate:polaris-code-intel`, `crate:polaris-security-automata` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime-dag` | `polaris-runtime-dag` | L4 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-runtime-research` | `polaris-runtime-research` | L4 | `crate:polaris-sdk`, `crate:polaris-runtime-frames`, `crate:polaris-runtime-context` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |


---

## P8 — DevEnv (could)

### P8 crate tasks (generated from workspace.toml)

<!-- generated: P8 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-devenv-nix` | `polaris-devenv-nix` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-nix` |
| `crate:polaris-devenv-docker` | `polaris-devenv-docker` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `wsl-docker` |
| `crate:polaris-devenv-cicd` | `polaris-devenv-cicd` | L2 | `crate:polaris-sdk` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-devenv` | `polaris-devenv` | L2 | `crate:polaris-devenv-nix`, `crate:polaris-devenv-docker`, `crate:polaris-devenv-cicd` | re-exports compile; children's tasks PASS | — |
| `crate:polaris-system-one-nano` | `polaris-system-one-nano` | L2 | `crate:polaris-sdk`, `crate:polaris-proto` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-models`, `win-uv` |


---

## P9 — Interfaces (could)

| Task | Description | Depends on | Exit criteria |
|---|---|---|---|
| P9-X-1 | Static welcome page (approved demo design) — may be pulled forward for the Design criterion — [per-PC] `win-wasm32` | `crate:polaris-ui-web` | visual comparison to demo |
| P9-X-2 | Named-pipe transport with token auth; mTLS remote | `crate:polaris-rpc` | security tests |

### P9 crate tasks (generated from workspace.toml)

<!-- generated: P9 crate tasks -->
| Task | Crate | Layer | Depends on (crate tasks) | Exit criteria | Per-PC setup (§0.1) |
|---|---|---|---|---|---|
| `crate:polaris-desktop` | `polaris-desktop` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-gui`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done | — |
| `crate:polaris-web` | `polaris-web` | L7 | `crate:polaris-kernel`, `crate:polaris-ui-web`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done | `win-wasm32`, `win-cargo-tools` |
| `crate:polaris-daemon` | `polaris-daemon` | L7 | `crate:polaris-kernel`, `crate:polaris-rpc`, `crate:polaris-config`, `crate:polaris-log` | real `main`; smoke test PASS; PLAN §13 TODO done | — |
| `crate:polaris-rpc` | `polaris-rpc` | L5 | `crate:polaris-sdk`, `crate:polaris-proto`, `crate:polaris-security-capability` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ui-gui` | `polaris-ui-gui` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | — |
| `crate:polaris-ui-web` | `polaris-ui-web` | L6 | `crate:polaris-sdk`, `crate:polaris-ui-model` | PLAN §13 TODO done; descriptors' tests PASS; no unwrap/panic paths | `win-wasm32`, `win-cargo-tools` |
| `crate:polaris-spike-web` | `polaris-spike-web` | L8 | `crate:polaris-ui-web` | burst scenarios run; pass criteria met with evidence | — |


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
