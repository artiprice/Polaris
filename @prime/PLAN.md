# Polaris — Architecture & Delivery Plan

> **Document:** `@prime/PLAN.md` · **Version:** 1.0 · **Date:** 2026-10-20 · **Branch:** `mvp`
> **Companions:** `@prime/PHASES.md` (tasks) · `@prime/PHASES_STATUS.md` (evidence-backed status)
> **Single source of truth for packages:** `@prime/tools/workspace.toml` (regenerate with `python -I @prime/tools/workspace.py gen`)
> **Binding rules:** `@prime/prompt.md` (engineering) · `AGENTS.md` (truth kernel + completion contract) | IF ALL OF IT EXISTS IN WORKSPACE.

---

## 0. How to read this document

**Truth labels** (truth kernel). Every factual claim is tagged:

| Label | Meaning |
|---|---|
| **VERIFIED** | Checked this session against live output, a command, or a fetched primary source. |
| **LIKELY** | Consistent with evidence, not directly verified. |
| **UNVERIFIED** | Plausible but unchecked; a spike or a phase task must confirm it. |
| **GUESSING** | Placeholder estimate. Never used for decisions. |

**Status vocabulary** (PHASES_STATUS.md): `PASS` (with pasted command + raw output + conditions), `FAIL`,
`BLOCKED` / `BLOCKED-ON-ACCESS` (missing key/access), `NOT STARTED`, `IN PROGRESS`.
A crate that exists only as a documentation skeleton is **never** called implemented.

**Sources analysed** (all completed before this document was written):

1. `@prime/chatlog.json` — 42 messages (21 user, 21 model; ~290 K chars of model drafts), inventoried
   losslessly in three slices (msgs 0–13, 14–27, 28–41) plus a mechanical regex inventory (396 items,
   `@prime/tools/chatlog_inventory.json`). Every item is dispositioned in §14–§15.
2. `@prime/initial.md`, `@prime/hackaton_content.md`, `@prime/prompt.md`, `AGENTS.md`.
3. `dist/demo-design-preview-slop` — design reference only (TUI design; web **welcome page** only).
4. `@prime/tmp/setup` — RTK reference (`harness rtk` diagnostic filter + issue ledger).
5. Live verification: Nebius Token Factory catalog API, Nebius/NVIDIA docs, OpenShell repo at v0.1.4,
   Hugging Face model files, crates.io (feature maps, dependency requirements, sparse index), local toolchain.
6. A red-team review of the draft plan; valid findings folded in (§5, §7, §12).

## 0.1 P0 outcomes (2026-10-20) — these supersede earlier statements in this plan

Evidence: `@prime/PHASES_STATUS.md` (§E9–§E13) and `@prime/spikes/<spike>/RESULT.md` (+ raw files). Where an
older paragraph below disagrees, this section wins; the load-bearing places (§2 ADR table, F21, §5, §6.9, §7,
§10, §13 `polaris-storage-vector` and `polaris-tools-math`, §15.5) were edited in place as well.

| Topic | Outcome (VERIFIED unless marked) | Consequence |
|---|---|---|
| Team setup | Per-PC installs are data (`[[member_setup]]` in `workspace.toml`, 26 items) rendered into PHASES.md §0.1; `setup-windows.ps1` (Windows PowerShell 5.1-compatible, idempotent, `-Check`) runs `setup-wsl.sh` inside WSL2 | every member runs one script; `polaris doctor` (P2-X-2) re-checks the same list |
| Licences | **evalexpr ≥ 12 is AGPL-3.0-only** (11.3.1 was MIT) → replaced by **exmex 0.21.0** (MIT OR Apache-2.0). `cargo deny check` clean; `cargo vet` passes with 5 audit imports + 29 exemptions (audit backlog, not audits). Only Slint's triple licence remains open (ADR-014) | team decision on Slint before P9 |
| S0-1 OpenShell SDK | builds on windows-msvc without protoc; SDK `ClientConfig` has **no mTLS identity** → Polaris builds the tonic channel (CA + client `Identity`, `domain_name("localhost")`) and calls `OpenShellClient::from_parts(channel, EdgeAuthInterceptor::noop())`; dial **`https://localhost:17670`** (127.0.0.1 fails from Windows); gateway rejects requests **> 1 MiB decoded**; after a WSL2 boot the gateway listens only after ~3 s; the gateway needs `OPENSHELL_COMPUTE_DRIVER=docker` pinned (boot race) | `polaris-openshell-client`: own channel, retry/backoff, ≤ 896 KiB chunks; `polaris-platform-wsl`: cert discovery + keepalive |
| S0-2 vm driver | `/dev/kvm` in WSL2; vm driver boots Ubuntu (guest 6.12.76) in 27.5 s, exec 216 ms; one driver per gateway | E2 microVM tier is local; P7 needs a second gateway instance or a driver switch |
| S0-3 NemoClaw | custom agents unsupported (in-tree only); blueprint pins OpenShell 0.0.116 | native blueprint applier in `polaris-openshell-nemoclaw` (reuse blueprint shape, base images, policy presets) |
| S0-4 GLiNER2.5-Decide | fp32 ONNX via gliner2-rs' own exporter: 260/260 = 100 % top-1 vs Python **only when the text gets a trailing `.`** like Python `gliner2` 2.1.0 does (93.85 % without); Rust fp32 p50 363 ms / p95 481 ms per 1-task call (4 threads), 2.9× faster than Python; RSS peak 2.35 GiB (borderline vs 2.5 GB); fp16 is worse on CPU; export needs ≈ 8 GB free RAM; CUDA EP not measured | ADR-008 updated; S1 client replicates the period rule with a regression test; memory budget is tight for a second model (PII-NER) |
| S0-6 Needle 3 | runs on Windows (Python/C engine): tool call p50 317 ms, embeddings 3,072-d p50 50 ms; `--serve` has no embed endpoint | Tier 0 stays a supervised sidecar (P8) |
| S0-7 sync | 10k files: no-op manifest+diff 1.1 s, 61 MiB initial push 10.4 s, sha256-exact, CRLF/exec bit kept, deny-list holds; error paths leaked a sandbox | P2 sync = blake3 manifest + tar-over-exec-stdin; reaper/RAII delete on every exit path; case-fidelity tests still TODO |
| S0-8 iceoryx2 | RTT p50 2.0 µs; survives peer kill; dead-node cleanup works; **default root `C:\Temp\iceoryx2` inherits `Authenticated Users:(M)`** | ADR-013 condition: per-user `set_root_path` with an owner-only ACL before any use |
| S0-9 LanceDB | 0.40.0 **does not compile without `features = ["remote"]`** (job.rs gating bug) and needs protoc; 36 min build at -j4, 4.8 GB target, 249 MB exe → kill criterion met; fallback verified: usearch 2.26.5 + SQLite builds in 1 min 41 s, 4.3 MB, kNN p50 0.475 ms | ADR-007/§6.9: **usearch + SQLite is the default vector store**; LanceDB = opt-in `lancedb` feature |
| S0-10 crossterm | probe built; needs a manual key test per terminal (IN PROGRESS) | newline chord decided after the test |
| S0-11 mxc | no Windows gateway asset in v0.1.4; MXC "coming soon" | WSL2 tiers only (E1w dropped for now) |
| S0-12 guest + browser | guest built in WSL2 and baked into an image; `protocol_version` handshake + mismatch refusal work; default sandbox policy: `/opt` neither readable nor executable, `/usr` fine, downloads only from `/sandbox`, Chromium zygote denied → `--no-zygote`; headless Chromium screenshot works; Xvfb fails | images install under `/usr`; P7 browser loop = headless Chromium over CDP |
| Local LLM (P3) | Ollama 0.40.6 + `nemotron-mini`: SSE streaming OK; **tool calls in 1 of 4 runs** | evaluate a stronger ≤ 4 GB model before P3-X-1 (new risk R22) |
| xtask (P0-XT-1) | Rust `check-members` / `check-layers`, test-first; `workspace.py gen` never touches crates marked `implemented = true` | `cargo xtask check-members` / `check-layers` alongside the Python checks |

---

## 1. Executive summary

**Polaris** is a Rust-native, security-first coding agent (Claude Code / Codex / Antigravity class) built
for the **Nebius × NVIDIA Global AI Hackathon — Coding & Agentic Engineering track**. Its thesis:

- **Two-speed cognition.** A local *System One* (GLiNER2 family encoders, typed Choice/Score/Noul
  decisions) handles routing, tool pruning, safety, PII, verification and agent-health checks with
  **zero cloud tokens**; **NVIDIA Nemotron on Nebius Token Factory** (System Two) is called only for real
  reasoning and code synthesis.
- **Agents are context frames, not processes.** One S2 client and one S1 engine serve every sub-agent;
  frames differ by role, capability token, scratchpad and history.
- **Zero-trust execution.** Every build/test/run happens in **NVIDIA OpenShell** sandboxes (WSL2 locally,
  optionally a remote gateway on Nebius AI Cloud) or **Token Factory Sandboxes (Contree)**; credentials
  are injected by OpenShell provider profiles and never seen by agent code; mutations are authorized by
  **Biscuit** capability tokens and a three-layer verifier.
- **Lossless context.** Nothing is destructively summarized: tool outputs fold into a content-addressed
  store with byte-exact expansion; a stable prompt prefix maximizes provider-side caching.
- **NVIDIA expertise built in.** A local NVIDIA documentation engine (`llms.txt`, `.md` pages, docs MCP)
  plus CUDA advisor rule packs (`$nvidia`, `/nv-profile`, `/nv-verify`).

### 1.1 Hackathon requirement compliance

| Requirement (`@prime/hackaton_content.md`) | How Polaris satisfies it | Status |
|---|---|---|
| Runs on Nebius Token Factory or AI Cloud | S2 = Token Factory OpenAI-compatible API; optional remote OpenShell gateway on AI Cloud | Planned (P3, P6); **BLOCKED-ON-ACCESS** (no key yet) |
| Uses ≥1 NVIDIA open model | Nemotron 3 Nano / 3.5 Lightning / 3 Super / 3 Ultra / 3.5 Super VL (VERIFIED in catalog) | Planned (P3) |
| Track: agents that write, run and test code *in Token Factory* | Token Factory Sandboxes (Contree) as execution tier E3 via `contree-mcp` | Planned (P3/P6); BLOCKED-ON-ACCESS |
| Working project | MVP scripted demo at end of P4 (§11.7) | Planned |

### 1.2 Judging criteria mapping

| Criterion | Polaris answer |
|---|---|
| Technological implementation | Rust workspace (93 packages, layer-checked), OpenShell + Contree + Token Factory integration, local S1 encoders, Biscuit + CQRS core, lossless context engine. |
| Design | Coherent product across TUI (first), Slint desktop, Leptos SSR+PWA web; design tokens inherited from the approved demo welcome page. |
| Potential impact | Solves host-side execution risk, token cost of routine decisions, multi-agent edit thrashing, and context loss in long sessions — for teams writing GPU/systems code. |
| Quality of the idea | Non-obvious split of System One (local typed decisions) vs System Two (Nemotron), executed in NVIDIA's own security runtime and Nebius's SWE sandboxes. |

---

## 2. Decisions register

### 2.1 User decisions (final, this session)

| # | Decision | Consequence |
|---|---|---|
| D1 | Core + UI run natively on **Windows**; execution plane = **OpenShell gateway in WSL2 (Ubuntu 24.04)**; optional **remote gateway on a Nebius AI Cloud VM**. | `polaris-platform-wsl`, `polaris-openshell-client` support local and remote gateways; host-initiated traffic only. |
| D2 | MicroVM tier = **OpenShell `vm` driver (libkrun)**; no separate Firecracker crate. | Chatlog's `polaris-firecracker` is replaced by OpenShell's vm driver (tier E2); fallback remote gateway (E4). |
| D3 | **`panic = "unwind"`** + hybrid supervision. | Rust tasks restart in-process (tokio `JoinError::Panic`); FFI/abort-class work isolated in `polaris-worker` processes. |
| D4 | Deliver docs + root `Cargo.toml` + **compiling doc-only skeleton**. | Done in this task (see PHASES_STATUS P0-SKEL-*). |
| D5 | `.gitignore` root-anchored; **Cargo.lock tracked**; `.gitattributes` LF. | Nested crate dirs (`cache/`, `sessions/`, `bin/`…) are trackable. |
| D6 | Web = **Axum + Leptos SSR + PWA**; **Tauri dropped**; **Slint** for desktop (mobile in v2). | `polaris-web` uses the cargo-leptos dual-feature layout; service worker is the only hand-written JS. |
| D7 | Web search = **Tavily + SearXNG** (both fully integrated). | `polaris-search-providers`. SearXNG runs in WSL2 (keyless). |
| D8 | Embeddings = **Token Factory `Qwen/Qwen3-Embedding-8B`** now; **Needle 3** local after spike; add **`quick_cache`**. | Text is PII-masked before embedding; L1 cache uses quick_cache. |
| D9 | Deadline > 1 month; **no API keys yet**. | Keyless work first; key-dependent items marked BLOCKED-ON-ACCESS. |
| D10 | **No lint attributes or lint config anywhere** (Cargo.toml, lib.rs, clippy.toml). | `[workspace.lints]` exists only as an empty table (template inherits it). prompt.md rules are followed by discipline + review; `xtask lint` may run the list on demand. |
| D11 | **`$directives` are session-scoped**, persisted with the session for `/session resume`. | New sessions start from config defaults; `/status` shows the layer that set each value. |

### 2.2 Architecture Decision Records (summary; full text in §7)

| ADR | Title |
|---|---|
| ADR-001 | Stack from initial.md overrides prompt.md §3.2 allowlist; vetting via cargo-deny + cargo-vet |
| ADR-002 | Composition root: `Backends` associated-type bundle + enum dispatch; `dyn` only on cold paths |
| ADR-003 | Zero-allocation hot path is a closed, tested set |
| ADR-004 | Unsafe policy: own `unsafe` = 0 except declared exceptions; vetted unsafe-containing deps |
| ADR-005 | TLS: aws-lc-rs everywhere; explicit process-level provider install |
| ADR-006 | Fault tolerance: in-process supervision + per-mode worker processes (D3) |
| ADR-007 | Storage: redb + SQLite FTS5 + usearch/SQLite vectors (LanceDB opt-in, S0-9) + file CAS; CockroachDB/sled/LMDB/DuckDB rejected for v1 |
| ADR-008 | System One inference: gliner2-rs on ONNX Runtime (load-dynamic) in a worker; Python sidecar fallback |
| ADR-009 | Execution plane: OpenShell (WSL2/remote) + Contree via MCP; sync is byte-exact and secret-free |
| ADR-010 | Logging: `tracing` facade + spdlog-rs-backed async sinks |
| ADR-011 | Context: lossless folding into CAS; stable prefix; no destructive summarization |
| ADR-012 | Web: Leptos SSR + PWA (no Tauri); desktop/mobile: Slint |
| ADR-013 | IPC: tonic over named pipes (token-authenticated) now; iceoryx2 later if S0-8 passes |
| ADR-014 | Licensing: Apache-2.0 workspace; Slint licence decision; NVIDIA docs not redistributed in git |
| ADR-015 | Pinned versions & couplings (ort exact pin, reqwest majors, arrow via lancedb) |

---

## 3. Verified facts (premise check)

All facts below were checked on **2026-10-20** unless marked otherwise.

| # | Topic | Fact | Label | Source / evidence | Consequence |
|---|---|---|---|---|---|
| F1 | Token Factory API | OpenAI-compatible; base `https://api.tokenfactory.nebius.com/v1/`; Bearer `NEBIUS_API_KEY`. | VERIFIED | docs.tokenfactory.nebius.com/quickstart | S2 client = OpenAI-compatible; any compatible endpoint (incl. local Ollama) can exercise it. |
| F2 | NVIDIA models on Token Factory | `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B` (262K), `nvidia/Nemotron-3_5-Lightning` (1M, MTP), `nvidia/nemotron-3-super-120b-a12b` (256K), `nvidia/Nemotron-3-Ultra-550b-a55b` (1M), `nvidia/Nemotron-3.5-Super-VL` (1M, vision). | VERIFIED | `tokenfactory.nebius.com/api/public/models_info` (28 models) | Chatlog's Nemotron-70B/340B/Mini names are stale. |
| F3 | Embeddings on Token Factory | `Qwen/Qwen3-Embedding-8B` (type `embedding`). | VERIFIED | same catalog | D8. |
| F4 | Rate limits | Soft caps evaluated in 15-min windows; limit grows +20% when avg usage ≥ 80%, shrinks when ≤ 50%, up to 20× base. | VERIFIED (docs) | docs.tokenfactory.nebius.com/ai-models-inference/rate-limits | Ratelimiter tracks window utilization. |
| F5 | Prefix caching / batch API / rate-limit headers / reasoning controls | Not documented on the pages fetched. | UNVERIFIED | — | Spike S0-5; design degrades gracefully. |
| F6 | Token Factory Sandboxes (Contree) | VM-isolated sandboxes, 7,000+ preloaded SWE environments (SWE-bench Verified, SWE-rebench, SWE-rebench-V2), branching; CLI, Python-only SDK `contree-sdk`, MCP server. | VERIFIED (vendor docs) | docs.tokenfactory.nebius.com/sandboxes/swe-agents | Track fit; integrate via MCP. |
| F7 | Contree MCP | `pip install contree-mcp`; `contree-mcp --mode http --http-port 9452`; 18 tools: run, rsync, import_image, registry_token_obtain, registry_auth, list_images, get_image, set_tag, upload, download, list_files, read_file, grep, get_operation, list_operations, wait_operations, cancel_operation, get_guide. | VERIFIED | docs.tokenfactory.nebius.com/sandboxes/mcp(/tools) | Branching via `directory_state_id`; auth details UNVERIFIED (S0-13). |
| F8 | OpenShell | NVIDIA/OpenShell, Rust, Apache-2.0, alpha; latest stable **v0.1.4 (2026-10-13)**, tag rev `5a7728c304b5528e954e9ae900cf21f84e1a12b1`. | VERIFIED | GitHub releases API, `git ls-remote` | Pinned by rev. |
| F9 | OpenShell drivers | docker, podman, **vm** (libkrun; experimental; `/dev/kvm`; default image `nvcr.io/nvidia/base/ubuntu:24.04`; GPU via VFIO + root), kubernetes, **mxc** (Windows AppContainer via `wxc-exec`; no process policy, no interactive exec, no OCI images, no restart durability). | VERIFIED | crate READMEs at v0.1.4 | D2; tier E1w evaluated in S0-11. |
| F10 | OpenShell policies | Policy YAML top-level: `version`, `filesystem_policy`, `landlock`, `process`, `network_policies`; only network policies hot-reload; ≤ 256 fs paths; rejects bare-root rw. | VERIFIED (docs) | docs.nvidia.com/openshell policy schema | `polaris-openshell-policy`. |
| F11 | OpenShell inference | The managed `openshell inference set` / `inference.local` route **was removed**; providers are profiles (endpoint + allowed binaries) with proxy-side credential substitution; mismatches denied (`credential_endpoint_mismatch`). | VERIFIED | docs.nvidia.com/openshell/how-it-works/inference | Polaris ships a custom Token Factory provider profile. |
| F12 | OpenShell on Windows | Linux, macOS (Apple Silicon); Windows via WSL2 **experimental**. | VERIFIED | OpenShell README | D1. |
| F13 | openshell-sdk | Rust crate in-repo (`crates/openshell-sdk`), consumed via git; uses tonic 0.14, reqwest 0.12.28, rustls 0.23 (aws-lc-rs), axum 0.8, tokio-tungstenite 0.26. | VERIFIED | Cargo.toml at v0.1.4 | Version alignment (§10). Windows build UNVERIFIED (S0-1). |
| F14 | NemoClaw | Reference stack: versioned YAML **blueprint** (sandbox shape, policies, inference profiles), onboarding (`nemoclaw onboard`), lifecycle; base image `ghcr.io/nvidia/nemoclaw/sandbox-base`; agents OpenClaw (default), Hermes, DeepAgents; Windows via WSL2. **Bring-your-own-agent undocumented.** Docs expose an MCP server and `.md` pages. | VERIFIED / custom-agent UNVERIFIED | docs.nvidia.com/nemoclaw | Polaris blueprint + onboarding/doctor; S0-3. |
| F15 | System One | TypeSafe concept; flagship cloud model Jev; primitives **Choice**, **Score**, **Noul** (yes/no probability); typed answers with calibrated probabilities; `POST /v1/systemone`. | VERIFIED | docs.typesafe.ai/concepts/system-one | Polaris S1 API mirrors the primitives locally. |
| F16 | GLiNER2.5-Decide | `fastino/GLiNER2.5-Decide`; Apache-2.0; ships **`model.safetensors` 1.95 GB fp32, no ONNX**; config: GLiNER2 **span** checkpoint, DeBERTa-v3-large (24 layers, hidden 1024), max span width 8; Python `gliner2` (`AutoExtractor.from_pretrained`, `classify_text(text, schema)`; multi_label, cls_threshold, prompt yes/no, ordinal labels "0".."10", several tasks per call); English. Vendor CPU latency ≈ 167 ms on a 48-core CPU. | VERIFIED (files/config) / latency vendor-reported | huggingface.co/fastino/GLiNER2.5-Decide | Chatlog "<15 ms" is false for this hardware; ONNX export needed (S0-4). |
| F17 | Rust GLiNER2 | `gliner2-rs 0.9.6`: ONNX Runtime engine for GLiNER2 **span** checkpoints (entities, relations, classification with probabilities), requires ORT ≥ 1.23; `gliner-rs 0.3.1` (candle) targets GLiNER2.5 *boundary* checkpoints. Published ONNX exists for `fastino/gliner2-privacy-filter-PII-multi`. | VERIFIED (docs.rs/HF) | docs.rs | ADR-008. |
| F18 | Cactus Needle 3 | 121M params (engram memory), 8–29 MB `.cact` (2.125-bit quants), Apache-2.0; tool calling **and embeddings**; C++ engine (`--serve`), Python `cactus-needle`, community npm (WASM); no Rust bindings; Windows build unconfirmed. | VERIFIED (README) / Windows UNVERIFIED | github.com/cactus-compute/needle | Tier-0 gated by S0-6. |
| F19 | Karpathy autoresearch | Human-written `program.md`; agent edits code, runs a fixed time budget, measures one metric, keeps or reverts, repeats. | VERIFIED (secondary sources) | datacamp guide, write-ups | `/research` follows this loop; "epistemic graph / devil's advocate" are Polaris additions. |
| F20 | NVIDIA docs | `llms.txt` at docs.nvidia.com, /cuda, developer.nvidia.com, /openshell, /nemoclaw; append `.md` to a page for Markdown; section `/llms.txt`; docs MCP at `docs.nvidia.com/nemoclaw/_mcp/server`; no global sitemap (404). | VERIFIED | HTTP 200 probes + llms.txt contents | `polaris-nvidia-docs` ingestion order: MCP → `.md` via llms.txt → HTML→MD. |
| F21 | Local machine | Windows 11 LTSC 2024; GTX 1650 4 GB (sm_75, driver 617.42); 16 GB RAM; Ryzen 7 2700 (8C/16T); WSL2 only `docker-desktop` (no Ubuntu, no `.wslconfig`); Docker 29.6.1; rustc/cargo 1.99.0 + nightly (Miri); cmake 4.3.0-rc3; NASM 3.01; VS Build Tools 2026; ninja 1.13.2; Python 3.14.5; Node 24.12; cargo-deny 0.20.2, cargo-vet 0.10.2, cargo-audit 0.22.2 **already present** (correction: an earlier draft listed them as missing). **Missing at plan time:** protoc, clang, uv, nix, cargo-nextest/fuzz/mutants/leptos, kani — all except clang and nix installed in P0 (§0.1, PHASES.md §0.1). | VERIFIED | command output | P0 setup tasks. |
| F22 | Dependency resolution | All 153 active external deps resolve together (1,699 packages) after pinning `ort =2.0.0-rc.13` and aligning `rten 0.26.0`. | VERIFIED | `workspace.py probe` (`cargo generate-lockfile`) | §10. |
| F23 | Hackathon brief | Must run on Token Factory or AI Cloud and use ≥ 1 NVIDIA open model; track = coding agents that write, run and test code in Token Factory. | VERIFIED | `@prime/hackaton_content.md` | §1.1. |

---

## 4. Chatlog corrections (claims that were wrong, stale or unverifiable)

The chatlog is an AI-drafted design. Its *ideas* are preserved (see §14); its *facts* were checked.

| # | Chatlog claim (msg) | Reality | Label | Resolution |
|---|---|---|---|---|
| C1 | System 2 = `nvidia/llama-3.1-nemotron-70b-instruct`, `nemotron-4-340b`, Nemotron-Mini, Nemotron-70B (1, 3, 9, 29, 33) | Token Factory serves Nemotron 3 Nano/Super/Ultra, 3.5 Lightning, 3.5 Super VL | VERIFIED | Effort→model table §6.2 |
| C2 | Endpoint `api.tokenfactory.nebius.ai` (1) | `api.tokenfactory.nebius.com/v1/` | VERIFIED | F1 |
| C3 | GLiNER2.5-Decide decides in 15–40 ms / "<15 ms" (1, 3, 29) | Vendor ≈ 167 ms on 48 cores; this host is 8 cores + 4 GB GPU | VERIFIED (vendor) | S1 off the synchronous path where possible; batching; xxh3 cache; measure in S0-4 |
| C4 | GLiNER2.5 runs as "embedded ONNX/Candle in Rust" (1) | No ONNX shipped; candle crate targets boundary checkpoints; Decide is a span checkpoint | VERIFIED | ADR-008, S0-4 |
| C5 | "DeBERTa-v3-large 340M" (1, 33) | Checkpoint is 1.95 GB fp32 (~0.49 B params incl. heads); model card says 340M | VERIFIED (files) | Memory budget §6.13 |
| C6 | OpenShell is accessed via "C-FFI / CLI RPC bindings" and `openshell inference set` / `inference.local` (1, 25) | Rust `openshell-sdk` (gRPC); inference route removed → provider profiles | VERIFIED | ADR-009 |
| C7 | OpenShell = runtime, NemoClaw = "policy & guard layer / token vault" (1, 19) | OpenShell = sandbox runtime **and** policy/credential engine; NemoClaw = blueprint/onboarding stack | VERIFIED | §6.6 |
| C8 | Firecracker microVMs "boot in 5–10 ms / 7.4 ms", run on the user's machine, host kernel exploit "impossible" (25, 33) | Firecracker needs Linux KVM; D2 uses OpenShell vm driver (libkrun); boot time unmeasured; overclaim | VERIFIED / UNVERIFIED (timing) | D2, S0-2 |
| C9 | `catch_unwind` restarts actors even after a segfault in C bindings (21) | Unwinding cannot catch segfaults/aborts | VERIFIED (Rust semantics) | D3 / ADR-006 |
| C10 | xxH3 used as a perceptual screenshot hash (25) | xxH3 is not perceptual | VERIFIED | pHash via `image_hasher` |
| C11 | CockroachDB "serverless version" embeddable (10) | No embedded mode; needs a server process | LIKELY (vendor model) | Rejected (ADR-007) |
| C12 | `sled` for docs/catalog index (5, 11) | sled is unmaintained for production use; redb chosen elsewhere in chatlog | LIKELY | redb (ADR-007) |
| C13 | spdlog-rs is "the official port of C++ spdlog" (27) | Community crate | VERIFIED | ADR-010 |
| C14 | evalexpr gives "rational fractions / arbitrary precision" (27) | It does not; note `rust_decimal` is exact decimal with ~28 significant digits, also not arbitrary precision. Also: **evalexpr ≥ 12 is AGPL-3.0-only** (VERIFIED, P0-ENV-7) | LIKELY | `rust_decimal` for exact decimal arithmetic; float path via **exmex 0.21.0** (MIT OR Apache-2.0) instead of evalexpr; arbitrary precision deferred (e.g. `dashu`) if a tool needs it |
| C15 | `$nebius-speculative` enables speculative decoding on Token Factory (9) | Not client-controllable on a serverless API (3.5 Lightning has server-side MTP) | LIKELY | Redefined as cascade drafting (cheap draft → strong verify) |
| C16 | S1 (GLiNER) synthesizes `flake.nix` in 15 ms (41) | An encoder cannot generate files | VERIFIED (architecture) | S2 drafts, Nix evaluates |
| C17 | Biscuit gives a "mathematical guarantee" against prompt injection (13) | Biscuit **bounds the damage** (scope, churn, TTL); injection can still steer allowed actions | VERIFIED (semantics) | §6.11 |
| C18 | mlock'd ephemeral key protects the ciphertext (27) | Key stored next to ciphertext in the same struct; plaintext still in heap `Vec` | VERIFIED (code reading) | `polaris-security-secrets` keeps them apart |
| C19 | regex-automata features `dfa-dense`, `hybrid-search` (33, 35) | Not features of 0.4.18; correct: `dfa-build`, `dfa-search`, `hybrid` | VERIFIED (crates.io) | workspace.toml |
| C20 | Crate versions (tokio 1.41, axum 0.7, ratatui 0.29, slint 1.8, tonic 0.12, prost 0.13, biscuit-auth 4, secrecy 0.8, rand 0.8, redb 2.1, lancedb 0.14, rusqlite 0.32, iceoryx2 0.4, spdlog-rs 0.4, evalexpr 11, bollard 0.18, …) | All re-pinned (e.g. tokio 1.54.0, axum 0.8.9, ratatui 0.30.2, slint 1.18.1, tonic 0.14.6, prost 0.14.4, biscuit-auth 6.0.0, redb 4.4.0, lancedb 0.40.0) | VERIFIED | §10 |
| C21 | Rate limiter described as GCRA but implemented as a fixed window (19) | Fixed window ≠ GCRA | VERIFIED (code reading) | `governor` GCRA + token-weighted TPM |
| C22 | DAG engine respawns running nodes (23) | Bug: no started-set | VERIFIED (code reading) | `polaris-runtime-dag` tracks started nodes |
| C23 | `/hack` detects data races by string-matching "data race"/`SIGSEGV` and reports `tokens_used = 0` as a constant (33, 35) | Not detection; constant is not a measurement | VERIFIED | TSan/loom/proptest in sandbox; tokens measured |
| C24 | gRPC diagram RPC names ≠ proto RPC names; tonic impl in PascalCase (35) | Mismatch; tonic generates snake_case methods | VERIFIED | Corrected proto in `polaris-rpc` |
| C25 | Tonic over UDS on Windows (35) | tokio has no UDS on Windows; named pipes need a custom connector + auth | VERIFIED | ADR-013 |
| C26 | Model "mmap reload < 8 ms" for ~680 MB and `madvise` on Windows (39) | Unmeasured; `madvise` is POSIX only | UNVERIFIED / VERIFIED | Measure in S0-4; Windows uses process exit / `EmptyWorkingSet`-free design (worker restart) |
| C27 | "100% KV-cache hit", "TTFT 800 ms → <100 ms", "86.8 % savings", "~180 k tokens saved" (11, 23, 31) | Illustrative numbers, not measurements | UNVERIFIED | Telemetry measures; S0-5 for caching evidence |
| C28 | Epistemic scratchpad in the stable prefix while being updated every turn (23) | Contradiction: updates invalidate prefix caching | VERIFIED (logic) | Scratchpad placed after the stable prefix |
| C29 | `§ref:<blake3>§` costs ~13 tokens (23) | A 64-hex hash is ~40+ tokens | LIKELY | 12-hex collision-checked short refs |
| C30 | Nebius "Zero Data Retention SLA, SOC 2, HIPAA, ISO 27001, Finland DC" (9) | Not verified this session | UNVERIFIED | Not used in claims |
| C31 | Contree CLI `contree use / run / branch-A`, REST `/sandboxes/v1`, WebSocket client (9) | Documented surfaces are CLI, Python SDK and MCP; REST paths unverified | VERIFIED / UNVERIFIED | MCP integration |
| C32 | Anthropic "Mods" names `blast-radius`, `proof-of-done`, `reflect`, `token-weather` (31) | Mods exist in Claude Code (plugin system); these specific names unverified | UNVERIFIED | Re-implemented natively as Polaris features regardless |
| C33 | "Karpathy's deep research algorithm: hypothesis tree, evidence ledger, devil's advocate" (21) | Karpathy's autoresearch = program.md + edit/run/measure/keep-or-revert loop | VERIFIED (secondary) | `/research` = autoresearch loop + Polaris additions |
| C34 | Needle 3 "121M params, 2-bit, Monarch Hadamard MLP" flagged as fabricated by reviewers | Real (README) | VERIFIED | Tier 0 (gated S0-6) |
| C35 | `"noul"` serde rename looked like a typo (1) | Real System One primitive | VERIFIED | Kept |
| C36 | `madvise(MADV_DONTNEED)` frees model pages; Firecracker shares host `/dev/shm` with guests (27, 39) | POSIX-only; microVM guests cannot see host `/dev/shm` | VERIFIED | Worker-process eviction; vsock/gRPC to guests |
| C37 | `fts5 tokenize='trigram'` needs SQLite ≥ 3.34; rusqlite `bundled` includes FTS5 (19) | Trigram tokenizer exists since 3.34; FTS5 in bundled build | LIKELY | Verify in P1 test |
| C38 | `musl` single static binary for self-hosting (37) | Conflicts with ORT/CUDA/native deps | LIKELY | v2 decision; per-target packaging |

---

## 5. Risk register (failure modes first)

| # | Risk / failure mode | Likelihood | Impact | What must be true | Mitigation | Owner / spike |
|---|---|---|---|---|---|---|
| R1 | **No API keys** → no Nemotron, no Contree, no Tavily → hackathon requirement unmet | Certain today | Critical | Keys obtained before P3 exit | Request on day 1 (P0-ACC-*); P3 walking skeleton against local OpenAI-compatible server; keyless phases first | P0 |
| R2 | OpenShell on Windows is WSL2-experimental and alpha; API churn (0.0.x → 0.1.x removed inference route) | High | High | Gateway reachable from Windows host via openshell-sdk | Pin rev; adapter isolation; conformance tests; remote gateway (E4) fallback | S0-1 |
| R3 | `openshell-sdk` does not compile on `x86_64-pc-windows-msvc` | Medium | High | SDK builds or a proto client is generated | Proto-generated tonic client from OpenShell `proto/` at the pinned rev | S0-1 |
| R4 | Nested KVM unavailable in WSL2 on Ryzen 2700 → no local microVM tier | Medium | Medium | `/dev/kvm` inside Ubuntu WSL2 | E4 remote gateway on Nebius AI Cloud; E1 docker tier for builds | S0-2 |
| R5 | S1 too slow / too big for this hardware (1.95 GB fp32; CPU latency ≫ 15 ms) | High | Medium | Batched decisions fit the latency budgets in §6.2 | fp16/int8 ONNX, CUDA EP on GTX 1650, batching, caching, fail-open/closed matrix, off-path consumers | S0-4 |
| R6 | GLiNER2.5-Decide ONNX export/parity fails | Medium | High | ≥ 99 % top-1 parity vs Python on golden set | Supervised Python `gliner2` sidecar (Python is an allowed auxiliary language) | S0-4 |
| R7 | Contree auth/branching not usable from Rust | Medium | High | contree-mcp runs as sidecar and authenticates | Sidecar via uv; fallback CLI wrapper; local OpenShell for demo | S0-13 |
| R8 | Token Factory lacks prefix caching / batch API / usable rate-limit headers | Medium | Low–Medium | — | Local prefix-stability metric; client-side coalescing; 429-driven adaptation | S0-5 |
| R9 | Compile time/RAM blow-up (lancedb+datafusion, slint, leptos, ort, gix, tonic) on 16 GB | High | Medium | Daily builds stay < budget | Heavy deps in leaf crates/features; short target-dir; sccache; thin LTO daily; `[profile.dist]` only for submission | S0-9 |
| R10 | TLS provider ambiguity (ring + aws-lc-rs both compiled) → runtime panic | Certain (graph) | High | Process-level provider installed before TLS use | `rustls::crypto::aws_lc_rs::default_provider().install_default()` in every `main`; smoke test | ADR-005 |
| R11 | Workspace sync leaks secrets into sandboxes or corrupts bytes (CRLF, case) | Medium | Critical | `.polarisignore` deny-list; byte-exact mirror | Default deny-list; `.gitattributes`; case/reserved-name detection; tests | S0-7 |
| R12 | WSL2 VM idles out / Docker Desktop VM vs Ubuntu VM confusion / Hyper-V firewall | Medium | Medium | Gateway keepalive; host-initiated traffic | Keepalive/systemd; `host.docker.internal`; doctor checks | P2 |
| R13 | Scope explosion (93 packages, 2 people, ~1–2 months) | High | High | MVP cut line respected | Phase gates; MVP = end of P4; should/could tiers; v2 declared | All |
| R14 | Model licences / docs licensing (NVIDIA docs redistribution; OpenMDW for Ultra/VL; Slint GPL/RF) | Medium | Medium | Licence review before submission | ADR-014; docs seed is a local artifact, not in git; cargo-deny licences | P10 |
| R15 | S1 thresholds uncalibrated → false blocks or missed threats | High (initially) | Medium | Labelled fixtures exist | Conservative defaults; fail-closed only where deterministic fallback exists; calibration task | P4 |
| R16 | Unsafe needed for Windows console input (Shift+Enter) or named-pipe DACL | Medium | Low | Declared exceptions | Alt+Enter / Ctrl+J first; isolated exception crate with sign-off | S0-10 |
| R17 | Needle 3 unavailable on Windows/WSL2 | Medium | Low | — | Tier 0 deferred; Token Factory embeddings (D8) | S0-6 |
| R18 | iceoryx2 unreliable on Windows | Medium | Low | — | gRPC over named pipes only | S0-8 |
| R19 | CMake 4 rejects old `cmake_minimum_required` in vendored C deps | Medium | Medium | — | `CMAKE_POLICY_VERSION_MINIMUM=3.5` in `.cargo/config.toml [env]` | S0-9 |
| R20 | Hostile fixtures quarantined by Defender / secret scanners | Medium | Low | — | Generate at test time, never commit (tests/fixtures/README.md) | P1 |
| R21 | Demo machine without WSL2/keys at judging | Medium | High | Fallback path | E3 Contree or E4 remote gateway; recorded transcript | P10 |
| R22 | Small local models fail at tool calling → the keyless P3 walking skeleton cannot exercise the tool loop | High (nemotron-mini: 1 of 4 runs) | Medium | A ≤ 4 GB VRAM model with reliable OpenAI-format tool calls | Evaluate other small models (S0-style probe `verify-local-llm.py --require-tools`); allow retries in the walking skeleton; Token Factory once keyed | P3-X-1 |
| R23 | Per-member disk budget: WSL2 disk image, Docker images (sandbox image 1.24 GB), model weights (GLiNER fp32 1.8 GiB + HF cache), cargo targets (a LanceDB build alone used 4.8 GB) | High | Medium | ≥ 40 GB free per PC | LanceDB opt-in; one shared spike target; prune targets after spikes; models outside the repo | P0 §0.1 |
| R24 | Shared-memory IPC exposed to other local users (iceoryx2 default root `C:\Temp\iceoryx2`, `Authenticated Users:(M)`) | Certain if defaults used | High | Per-user root with owner-only ACL | `set_root_path` + ACL test before any iceoryx2 use (ADR-013) | S0-8 |

---

## 6. Architecture

### 6.1 System context

```
                         ┌──────────────────────────── Windows host (native) ────────────────────────────┐
  User ──► polaris (TUI) │  polaris-kernel (composition root, Backends bundle, session typestate)        │
        ──► polaris-desktop (Slint)   ──► polaris-web (Axum+Leptos SSR+PWA)   ──► polarisd (gRPC daemon)  │
                         │      │                                                                        │
                         │      ├─ bus (CQRS/ES) ── eventlog (redb) ── projections ── UI view models      │
                         │      ├─ runtime: frames · context · leases · guards · dag · research            │
                         │      ├─ S1 client ──named pipe──► polaris-worker --mode s1 (ort + GLiNER2)      │
                         │      ├─ ingest client ──────────► polaris-worker --mode ingest (parsers + OCR)  │
                         │      ├─ storage: CAS · registry.db · events.redb · vectors · caches             │
                         │      └─ security: Biscuit · PII vault · secrets (keyring) · DFA · rule packs    │
                         └──────┬──────────────────────────────┬──────────────────────────┬──────────────┘
                                │ HTTPS (OpenAI-compatible)     │ gRPC (openshell-sdk)      │ MCP (HTTP, local sidecar)
                                ▼                               ▼                           ▼
                 Nebius Token Factory            OpenShell gateway                contree-mcp ──► Token Factory
                 Nemotron 3/3.5 · Qwen3-Emb      ├─ WSL2 Ubuntu (E1 docker, E2 vm)            Sandboxes (E3)
                                                 └─ Nebius AI Cloud VM (E4, remote)
                                                      sandbox ⇢ polaris-guest (exec, sync, Xvfb+Chromium)
```

### 6.2 Cognitive tiers

| Tier | Component | Where | Used for | Tokens |
|---|---|---|---|---|
| T0 | Cactus Needle 3 (gated, S0-6) | sidecar (Windows or WSL2) | JSON-argument filling, local embeddings, chaos inputs | 0 |
| S1 | GLiNER2.5-Decide + GLiNER2 PII-NER | `polaris-worker --mode s1` (ort; CPU or CUDA EP) | typed decisions: routing, effort, tool pruning, triage, safety, verification, watchdog, PII | 0 |
| S2 | Nemotron on Token Factory | cloud | planning, code synthesis, reflection, research hypotheses, critique | billed |

**System One primitives** (mirroring TypeSafe; F15) and their GLiNER2 mapping:

| Primitive | Polaris type | GLiNER2 schema mapping |
|---|---|---|
| Choice | `Question::Choice { labels, multi }` → `Answer::Choice { label(s), probs }` | classification task (single/multi-label) |
| Score | `Question::Score { min, max }` → `Answer::Score { value, confidence }` | ordinal labels `"0".."10"` mapped to range |
| Noul | `Question::Noul { statement }` → `Answer::Noul { p_yes }` | yes/no prompt classification |

Several questions are sent as **one multi-task schema** per forward pass. Results are cached by
`xxh3(schema_id ‖ input)` in L1 (`quick_cache`) and L2 (redb).

**Effort → S2 model** (overridable by `$subagent-models`; numeric 1–5 accepted):

| Effort | Model id (VERIFIED) | Typical work |
|---|---|---|
| trivial (S1/tool only) | — | run linter, format, obvious command |
| low (1) | `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B` | small fixes, argument drafting |
| medium (2) | `nvidia/Nemotron-3_5-Lightning` | single-file logic, long-context reading (1M) |
| high (3) | `nvidia/nemotron-3-super-120b-a12b` | multi-file changes, tests |
| xhigh (4) | `nvidia/nemotron-3-super-120b-a12b` + higher reasoning budget (S0-5) | hard debugging |
| max (5) | `nvidia/Nemotron-3-Ultra-550b-a55b` | architecture, research synthesis |
| vision | `nvidia/Nemotron-3.5-Super-VL` | screenshots, `@img` when OCR is insufficient |
| embeddings | `Qwen/Qwen3-Embedding-8B` | `$rag`, docs, semantic cache, session memory |

**S1 failure matrix** (timeouts are per-consumer, set in `polaris-system-one-schemas`):

| Consumer | On timeout / error | Synchronous? |
|---|---|---|
| Safety gate (commands, diffs) | **Fail closed**: deterministic DFA + rule packs + approval prompt | yes |
| PII masking | **Fail closed**: DFA + entropy only | yes |
| Pre-flight verifier | **Fail closed** for deterministic checks; semantic check skipped with warning | yes |
| Triage / tool pruning / effort routing | **Fail open**: pass all tools / default effort | no (timeout) |
| Watchdog, sifter, arbiter | **Fail open** with logged degradation | no |

**Latency/memory targets** (to be measured in S0-4, not claims): single multi-task decision on CPU ≤ 400 ms
p95 (GUESSING until measured); with CUDA EP fp16 ≤ 80 ms p95 (GUESSING); resident S1 ≤ 2.2 GB fp32 or ≤ 1.2 GB fp16.

### 6.3 Agents are context frames

One S2 client (pooled HTTP/2) and one S1 worker serve all agents. An **AgentContextFrame** holds: role
(`Architect`, `KernelDeveloper`, `TestEngineer`, `SecurityAuditor`, `Researcher`, `DocVerifier`, …),
attenuated Biscuit token, sandbox quota profile, epistemic scratchpad, history CIDs, step count, parent id.
Child frames inherit **attenuated** rights (path scope, churn limit, TTL, no-unsafe) and **resource quotas**
(cpu, memory, network policy). The **watchdog** (S1) checks every proposed step for goal drift, semantic
loops (sliding window of xxh3 step fingerprints), role-boundary violations and context fatigue, and injects
corrective anchors or negative constraints into the scratchpad.

### 6.4 Event-driven core (CQRS / event sourcing)

Agents never mutate state directly; they submit **commands** carrying a Biscuit token. Approved effects are
**events** appended to `events.redb`, then projected.

| Commands | Events |
|---|---|
| `ProposeDiff { agent, token, path, diff, intent }` | `DiffApproved`, `DiffRejected { layer, reason }` |
| `AcquireScope { agent, token, path, scope }` | `LeaseGranted`, `LeaseRevoked`, `LeaseQueued` |
| `SpawnFrame { parent_token, role, scope, quotas }` | `FrameSpawned`, `FrameEnded` |
| `ExecuteTool { agent, token, call }` | `ToolStarted`, `ToolCompleted { exit, cid }` |
| `RunInSandbox { tier, cmd }` | `SandboxCreated`, `SandboxBranchCreated`, `SandboxExecCompleted` |
| `SetDirective`, `SlashCommand` | `DirectiveApplied`, `SessionStateChanged` |
| `RollbackBranch`, `Undo` | `RolledBack` |

**HybridMutationVerifier** (three layers, cheapest first):
1. **Formal** — Biscuit signature chain + Datalog checks (`resource`, `churn ≤ limit`, `time < ttl`,
   `touches_unsafe == false`, `diff_blake3 == planned`).
2. **Metric** — churn, Δcyclomatic complexity, AST tree-edit distance (deterministic, no model).
3. **Semantic** — S1 Noul: "does this diff serve the task intent?" (threshold calibrated; fail closed only
   for high-risk paths).

### 6.5 Request lifecycle (functional operator pipeline)

```
input ─► SanitizePii ─► ParseDirectives ─► S1Triage(intent, effort, tools) ─► ContextAssemble
      ─► RateLimit(GCRA RPM + TPM, priority) ─► S2 (stream) ─► tool calls ─► Leases ─► Verifier
      ─► Sandbox exec (tier per $sandbox) ─► RTK filter / §ref fold ─► Post-flight ─► ProofOfDone
      ─► events ─► UIs / telemetry / registry
```

- Trivial intents short-circuit after S1Triage (no S2 call).
- `fork_branch` operators split work into frames or sandbox branches; `assert_guard(ProofOfDone)` closes tasks.
- Every operator implements the sdk `FunctionalOperator` trait and declares its fail-open/closed policy.

### 6.6 Execution plane

| Tier | Backend | Where | Use | Notes |
|---|---|---|---|---|
| E0 | in-process read-only | host | read, grep, glob, tree-sitter | no exec |
| E1 | OpenShell **docker** driver | WSL2 Ubuntu 24.04 | builds, tests, lint | default local tier |
| E2 | OpenShell **vm** driver (libkrun) | WSL2 (nested KVM) | untrusted code, GUI/browser tests, `/hack` dynamic | S0-2; else E4 |
| E3 | Token Factory Sandboxes (Contree) | Nebius | SWE-bench envs, branching, heavy runs | MCP sidecar; keys |
| E4 | remote OpenShell gateway | Nebius AI Cloud VM | KVM microVMs, cloud bursting | mTLS |
| E1w | OpenShell **mxc** (AppContainer) | Windows | Windows-native builds | S0-11 (evaluation only) |

`$sandbox(auto)` routing (S1 router): trivial checks → E1; untrusted/GUI/browser → E2 (or E4); benchmark
envs and branch search → E3. **Workspace sync** (`polaris-storage-sync`): manifest of (path, size, blake3),
only changed blobs move, byte-exact (no EOL translation), `.polarisignore` deny-list (`.env`, `*.pem`,
`*.key`, `.git/config` by default). **Credentials**: OpenShell provider profile for Token Factory
(endpoint + allowed binaries) → sandboxed code never sees keys. **Guest**: `polaris-guest` built in WSL2,
baked into the Polaris sandbox image, `protocol_version` handshake. **Topology rules**: Windows host
initiates all connections; WSL2 gateway kept alive (keepalive/systemd); Docker Desktop containers live in
the `docker-desktop` VM (reach host via `host.docker.internal`).

### 6.7 Concurrency mesh

Named semaphore pools (defaults from chatlog msg 17, adjusted for this host; `$concurrency(...)` overrides):

| Pool | Default permits | Guards |
|---|---|---|
| `inference` (Token Factory) | 6 | concurrent S2 streams |
| `web` (Tavily/SearXNG/fetch) | 10 | outbound research |
| `docs` (NVIDIA docs index) | 16 | local index reads |
| `disk` (fs + tree-sitter) | 64 | EMFILE protection |
| `s1` (worker batches) | 2 | CPU/GPU saturation |
| `sandbox` (create/exec) | 4 | sandbox/microVM count |

Speculative races (`JoinSet` + first-valid-wins + `abort_all`), hierarchical `CancellationToken`s, and
RAII permits. `/concurrency-stats` shows usage/queues; `/parallel-dag` shows the task graph.

### 6.8 Rate limiting & token economy

GCRA (`governor`) for RPM; token-weighted cells for TPM; reservations before sending, reconciliation from
`usage` after; priorities `InteractiveUser > Verifier > SubAgent > Background`; background shedding near
limits; `Retry-After` with jitter; 15-min window utilization tracking (F4). Token economy: S1/T0/DFA do
routine work; stable prompt prefix; folding; coalescing of compatible requests; `$budget` warnings;
`/usage`, `/quota`, token-weather HUD.

### 6.9 Storage layout

```
~/.polaris/                       (global; $storage-level(global))
├── config.toml                   layered config (secret *references* only)
├── registry.db                   SQLite: sessions, FTS5 trigram history search
├── events.redb                   event journal, L2 exact cache, locks, invariants, research checkpoints
├── cas/ab/cd/<blake3>            content-addressed blobs (zstd)
├── vectors/                      usearch HNSW indexes + SQLite metadata (docs, RAG, session memory, L3 semantic cache)
├── models/                       ONNX fragments, tokenizers, ORT DLL, Needle .cact (downloaded)
├── docs/nvidia/                  NVIDIA docs seed + incremental sync state (local artifact, not in git)
└── logs/                         rotating spdlog-style sinks
<repo>/.polaris/                  (repo-local; $storage-level(isolated))
├── config.toml  settings.toml    repo config; session settings snapshots
├── RULES.md  invariants (redb)   Reflect/Rule Distiller output
├── issues.md                     RTK ledger
├── screenshots/                  /screenshot, /ui-diff baselines
├── skills/                       $skills
└── .polarisignore                sync deny-list
```

| Store | Engine | Holds | Why |
|---|---|---|---|
| Registry | SQLite (`rusqlite` bundled) | sessions, FTS5 history | relational + full-text |
| Event log | redb | events, locks, L2 cache, invariants | pure-Rust ACID KV, append-only |
| CAS | files + blake3 + zstd | blobs, snapshots, folded outputs | dedup, byte-exact recall |
| Vectors | usearch + SQLite (S0-9; LanceDB opt-in feature) | embeddings | embedded, no server |
| L1 cache | quick_cache | hot decisions/completions | bounded RAM |

Rejected for v1 (ADR-007): CockroachDB (no embedded mode), sled (maintenance), LMDB/heed (redb covers),
DuckDB (heavy C++; SQLite covers analytics) — revisit if analytics needs grow.

### 6.10 Lossless context virtualization

Prompt layout (byte-stable prefix first): `[system + tool schemas + versioned repo map]` →
`[epistemic scratchpad: target, invariants, proven facts, disproven hypotheses, blockers, touched symbols]`
→ `[sliding window]`. Tool outputs above a size threshold are stored in CAS and replaced by
`§ref:<12hex>` + a deterministic summary (compiler diagnostics via RTK, files via tree-sitter skeleton,
logs head/tail). The model restores bytes with `polaris_expand(ref)`; `polaris_recall(query)` searches
history (FTS5 + vectors). `/context-map` shows the budget per region; `/context-expand <ref>` for humans.

### 6.11 Security model

| Threat | Controls |
|---|---|
| Host damage by agent code | All exec in OpenShell/Contree sandboxes; policies per role/tier; blast-radius approval |
| Credential theft | Keys in keyring/secret vault; provider profiles inject at egress only; `.polarisignore` |
| Prompt injection (web, docs, repo) | Biscuit scope bounds, S1 safety gate, DFA rule packs, approvals for destructive ops |
| PII leakage to cloud | PII vault masks before every S2/embedding call; unmask only at sandbox boundary |
| Test evasion / fake completion | Pre/post-flight verifier; Proof-of-Done gate; assertion-tamper detection |
| Edit wars between agents | Leases (file/AST scope), OCC on base hash, cycle breaker |
| Supply chain | cargo-deny/vet, pinned revs, licence checks; typosquat rules in packs |
| Local RPC hijack | Named pipes with token auth; mTLS remote |
| Secrets in RAM / logs | secrecy/zeroize, sealed vault, `Redactor` before every sink |

### 6.12 Fault tolerance & deallocation

| Resource | Mechanism | Windows | Linux/WSL2 |
|---|---|---|---|
| Rust tasks | supervisor tree (one-for-one, intensity limits, circuit breaker, DLQ) | `JoinError::Panic` restart | same |
| Native workers | one process per mode, supervised, restart on exit | Job Object kill-tree | process group |
| Child processes | `process-wrap` kill-on-drop trees | CTRL_BREAK (if possible) → TerminateJobObject | SIGTERM → SIGKILL |
| Sandboxes | Reaper destroys on session end/crash | via gateway | via gateway |
| Temp dirs | `tempfile` RAII | | |
| Secrets | zeroize on drop | | |
| Models | lease + idle TTL + force-evict before builds; worker restart frees pages | worker exit | worker exit |

### 6.13 Model lifecycle & memory budget (16 GB host)

| Consumer | Budget (target) |
|---|---|
| Windows + apps | ~4 GB |
| WSL2 VM (`.wslconfig` cap) | ~6 GB (builds run here) |
| S1 worker (fp16 preferred) | ≤ 1.5 GB |
| Polaris kernel + UI | ≤ 0.8 GB |
| Headroom | ≥ 2 GB |

Models load lazily, are leased per operation, evicted after idle TTL (default 45 s, configurable) or on
memory pressure (`sysinfo`), and force-evicted before heavy builds.

### 6.14 NVIDIA knowledge engine

Ingestion priority: docs MCP endpoints → `.md` pages listed in `llms.txt` → HTML→Markdown fallback. Topics:
CUDA C++ guide & best practices, PTX ISA, cooperative groups, TensorRT-LLM, Triton, NCCL/NVLink/GPUDirect,
Hopper/Blackwell tuning guides, OpenShell, NemoClaw, Nemotron prompt/tool formats. `$nvidia(auto)` probes
the workspace (`.cu/.cuh/.ptx`, `cudarc`/`cust`/`candle` CUDA, `tensorrt_llm`, `nvcr.io` images,
OpenShell configs), local hardware (GTX 1650 = sm_75) and target hardware (default Nebius H100 = sm_90),
and injects only the relevant invariants. `/nv-profile` and `/nv-verify` run CUDA advisor rule packs.
An offline seed is built by `xtask` as a local artifact (not committed; licensing).

### 6.15 Nebius integration map

| Nebius service | Polaris use | Crate | Phase |
|---|---|---|---|
| Token Factory inference | S2 chat, tools, Responses API, embeddings | `polaris-nebius-cloud-token-factory` | P3 |
| Token Factory Sandboxes (Contree) | E3 tier, SWE env resolver, branching | `polaris-nebius-cloud-sandboxes` | P3/P6 |
| Data Lab / post-training | trajectory export, fine-tune trigger | `polaris-nebius-cloud-data-lab` | P6 |
| AI Cloud | remote OpenShell gateway, cloud bursting | `polaris-nebius-cloud-ai-cloud` | P6 |

### 6.16 Interfaces & design system

TUI first (`polaris-ui-cli`): demo **design** reused (not code): canvas `#000000`, text `#E8E8E8`,
accent NVIDIA green `#76B900`, gold `#F9E2AF` (mentions/tools/plan), blue `#89B4FA` (settings/goal),
pink `#F5C2E7` (auto mode), diff `#91CD66/#07230C` and `#F59696/#320709`, error `#AC4242`, warning
`#E5B454`; chip logo with `✦`; composer (paste chips, history recall, Shift+Enter/Alt+Enter/Ctrl+J
newline, Shift+Up/Down effort, Shift+Tab plan/auto), completion menus for `$ / @`, `--dump WxH` snapshots.
Web (`polaris-ui-web`): approved welcome page (Space Grotesk / Inter / JetBrains Mono; aurora/stars
atmosphere; composer pill; model popover). Desktop: Slint (femtovg/software renderers). RPC:
`polaris-rpc` (named pipe + token auth; TCP + mTLS).

### 6.17 Platform seam

`PlatformOps` (sdk) abstracts process trees, PTY, fs watching, memory pressure, keyring, paths. Windows is
implemented first; a Linux `cargo check` leg in WSL2 runs from P1 so non-Windows code cannot rot; macOS
follows in v2 (Slint/HVF/keychain).

---

## 7. Architecture Decision Records

**ADR-001 — Stack vs prompt.md §3.2 allowlist.** *Context:* prompt.md lists six "authoritative" crates;
initial.md mandates Slint, ratatui, axum, Leptos (via D6), OpenShell, NemoClaw, GLiNER, etc.
*Decision:* initial.md's stack and this plan's pinned list override §3.2; every dependency is vetted with
cargo-deny (licences, advisories, bans) and cargo-vet before its phase exits. *Consequence:* P0 installs
the tools; `deny.toml` lands with the first real dependency.

**ADR-002 — Composition root.** *Decision:* `trait Backends { type Chat: ChatModel; type Embed:
EmbeddingProvider; type Decide: DecisionEngine; type Sandbox: SandboxBackend; type Search: WebSearch;
type Docs: DocParser; type Vectors: VectorStore; type Events: EventStore; }`; `Kernel<B: Backends>`.
Runtime-selected variants use enums (`enum SandboxKind { OpenShell(..), Contree(..), Remote(..) }`).
`dyn` only on cold paths with a written reason. *Consequence:* monomorphized hot paths; one place to wire.

**ADR-003 — Zero-allocation hot path.** *Decision:* the hot path is exactly: SSE frame parser, bus
publish/consume, GCRA check, DFA/PII scan, `§ref` hash/fold, lease lookup, log enqueue. Each has a
zero-allocation test using the vetted **`dhat` 0.3.3** testing mode (`dhat::Alloc` as `#[global_allocator]` plus
`HeapStats` assertions) — no own `unsafe` (a hand-written counting allocator would need `unsafe impl GlobalAlloc`).
Each such check lives in its own integration-test binary because dhat allows one profiler per process (LIKELY,
per dhat docs). *Consequence:* the rest of the async code allocates
normally; Valgrind/Massif (unavailable on Windows) are not required.

**ADR-004 — Unsafe policy.** *Decision:* own `unsafe` = 0 except declared, signed-off exceptions, counted
in CI: (a) ConPTY Shift+Enter input only if S0-10 fails and Alt+Enter/Ctrl+J are rejected; (b) named-pipe
security descriptor; (c) memory locking for the secret vault; (d) `fuzz_target!` expansion. Vetted deps
containing unsafe: ort, iceoryx2, slint, portable-pty, process-wrap, mimalloc, aws-lc-rs, rusqlite
(bundled SQLite), pdfium-render, gix (max-performance), etc.

**ADR-005 — TLS.** *Fact:* the probe shows rustls 0.23 compiled with both aws-lc-rs and ring (via reqwest
0.12 in lance and openshell-sdk) plus rustls 0.21/ring via dagger-sdk (VERIFIED). *Decision:* every binary
calls `rustls::crypto::aws_lc_rs::default_provider().install_default()` before any TLS; rcgen/tonic/
tokio-rustls configured for aws-lc-rs; dagger-sdk feature-gated. *Consequence:* no runtime provider
ambiguity; smoke test asserts the provider.

**ADR-006 — Fault tolerance (D3).** In-process supervisor for Rust tasks; one OS process per native mode;
circuit breakers + DLQ; release `panic = "unwind"`.

**ADR-007 — Storage.** See §6.9.

**ADR-008 — S1 inference.** `gliner2-rs` on ONNX Runtime (`load-dynamic`, DLL beside the exe, never
System32; `api-23` ⇒ ORT ≥ 1.23) inside `polaris-worker --mode s1`; ONNX fragments exported by a Python
script in xtask; parity ≥ 99 % top-1 against Python `gliner2` on a golden set (S0-4). Fallback: supervised
Python sidecar. `ort` pinned `=2.0.0-rc.13` (oar-ocr-core requirement; ort-sys `links`). **P0 result (S0-4, VERIFIED):** parity 100 % only when the text gets a trailing `.` the way Python `gliner2` 2.1.0 does (`_normalize_text`); the client must replicate it, with a regression test. fp32 ONNX fragments 1.81 GiB; Rust p50 363 ms per 1-task call on 4 CPU threads; RSS peak 2.35 GiB; fp16 is slower and larger on CPU; the export needs ≈ 8 GB free RAM. The Python sidecar is not needed.

**ADR-009 — Execution plane.** OpenShell gateway (WSL2 or remote) via openshell-sdk (or proto client);
Contree via MCP sidecar; sync byte-exact and secret-free; provider profiles for credentials. **P0 results (S0-1/S0-7/S0-12, VERIFIED):** Polaris builds the mTLS tonic channel itself (`OpenShellClient::from_parts`), dials `localhost`, retries connect after WSL2 cold starts, chunks payloads below the gateway's 1 MiB request limit; sandbox images install under `/usr` (the default policy denies `/opt`), artifacts live under `/sandbox`.

**ADR-010 — Logging.** `tracing` is the only logging facade (prompt.md §4); spdlog-rs provides async,
rotating sinks behind a tracing layer (user asked for "something like spdlog"); `Redactor` runs first.

**ADR-011 — Context.** See §6.10. No destructive summarization; summaries are additive pointers.

**ADR-012 — Web & GUI.** Leptos SSR (out-of-order streaming, per-route lazy loading) + PWA; Slint for
desktop and later mobile (Android Rust support; iOS initial support since Slint 1.12 — LIKELY for 1.18).

**ADR-013 — IPC.** tonic over Windows named pipes (custom connector; pre-created next instance;
token auth) and TCP+mTLS; iceoryx2 for bulk zero-copy only if S0-8 passes. **P0 result (S0-8, VERIFIED):** iceoryx2 works (RTT p50 2.0 µs, survives peer kill) but its default Windows root `C:\Temp\iceoryx2` is writable by all authenticated users — adoption requires a per-user `set_root_path` with an owner-only ACL (R24).

**ADR-014 — Licensing.** Workspace Apache-2.0. Slint: choose royalty-free (attribution) or GPLv3/commercial
before P9; NVIDIA docs content stays local (not in git, not embedded); model licences recorded
(GLiNER Apache-2.0, Needle Apache-2.0, Nemotron 3 Ultra / 3.5 Super VL under OpenMDW-1.1 per catalog).

**ADR-015 — Pins & couplings.** ort `=2.0.0-rc.13`; rten 0.26.0 (ocrs); reqwest 0.13.5 own code (0.12 via
OpenShell/lance, 0.11 via dagger, feature-gated); tower-http 0.6.11 and tokio-tungstenite 0.26.2 aligned
with OpenShell; arrow only via lancedb re-exports; regex-automata 0.4.18 / regex-syntax 0.8.11 as the user
pinned; leptos_meta 0.8.7.

---

## 8. Workspace layout

Rules: one nesting level (`crates/*/crates/*`); a parent is a **pure facade** (re-exports children);
**children never depend on their parent**; nested package name = `<parent>-<dir>`; every package is a
doc-only `lib` until its phase (binaries get `main` then). Layer rule: `dep.layer < crate.layer`, except
facade→child and sibling→sibling (`≤`). Enforced by `python -I @prime/tools/workspace.py check-layers`.

```
Polaris/
├── Cargo.toml                 generated (workspace.toml) · resolver 3 · edition 2024 · rust 1.99
├── Cargo.lock                 tracked (D5)
├── rust-toolchain.toml        1.99.0 + rustfmt + clippy
├── .gitignore  .gitattributes
├── apps/                      L7 (lib-only skeletons until their phase)
│   ├── polaris-cli            bin `polaris` (TUI + headless) ................................ P1
│   ├── polaris-desktop        Slint desktop .................................................. P9
│   ├── polaris-web            Axum + Leptos SSR + PWA ........................................ P9
│   ├── polaris-daemon         bin `polarisd` (gRPC) .......................................... P9
│   ├── polaris-worker         s1 | ingest | nano worker processes ............................ P4
│   └── polaris-guest          Linux in-sandbox agent ......................................... P7
├── crates/
│   ├── polaris-sdk                         L0 ........................................... P1
│   ├── polaris-proto                       L1 ........................................... P1
│   ├── polaris-config                      L1 ........................................... P1
│   ├── polaris-log                         L1 ........................................... P1
│   ├── polaris-telemetry                   L1 ........................................... P1
│   ├── polaris-ratelimit                   L1 ........................................... P3
│   ├── polaris-code-intel                  L1 ........................................... P1
│   ├── polaris-storage            (facade) L1 ........................................... P1
│   │   └── crates/ cas(P1) eventlog(P1) registry(P1) cache(P1) vector(P5) sync(P2)        L1
│   ├── polaris-security           (facade) L3 ........................................... P2
│   │   └── crates/ capability(L1,P3) automata(L1,P2) secrets(L1,P3) pii(L3,P4) rules(L3,P4)
│   ├── polaris-platform           (facade) L2 ........................................... P2
│   │   └── crates/ wsl(P2) process(P2)                                                    L2
│   ├── polaris-vcs                         L2 ........................................... P2
│   ├── polaris-kernel             (facade + composition root) L5 ........................ P1
│   │   └── crates/ bus(L1,P1) directives(L1,P1) supervisor(L2,P1) reaper(L2,P1)
│   │               operators(L4,P3) verifier(L4,P3)
│   ├── polaris-nebius-cloud       (facade) L2 ........................................... P3
│   │   └── crates/ token-factory(P3) sandboxes(P3) data-lab(P6) ai-cloud(P6)              L2
│   ├── polaris-openshell          (facade) L2 ........................................... P2
│   │   └── crates/ client(P2) policy(P2) nemoclaw(P2)                                     L2
│   ├── polaris-ingest                      L2 ........................................... P5
│   ├── polaris-search             (facade) L3 ........................................... P5
│   │   └── crates/ providers(L2,P5) sift(L3,P5)
│   ├── polaris-ext                (facade) L2 ........................................... P7
│   │   └── crates/ mcp(P7) skills(P7)                                                     L2
│   ├── polaris-browser                     L2 ........................................... P7
│   ├── polaris-devenv             (facade) L2 ........................................... P8
│   │   └── crates/ nix(P8) docker(P8) cicd(P8)                                            L2
│   ├── polaris-system-one         (facade) L3 ........................................... P4
│   │   └── crates/ engine(L2,P4) client(L2,P4) nano(L2,P8) schemas(L3,P4) router(L3,P4)
│   │               verifier(L3,P4) watchdog(L3,P4)
│   ├── polaris-models                      L3 ........................................... P3
│   ├── polaris-tools              (facade) L3 ........................................... P1
│   │   └── crates/ builtin(P1) math(P3) rtk(P1)                                           L3
│   ├── polaris-nvidia-docs                 L3 ........................................... P5
│   ├── polaris-hacker                      L3 ........................................... P7
│   ├── polaris-runtime            (facade) L4 ........................................... P2
│   │   └── crates/ frames(P3) context(P3) dag(P7) leases(P2) guards(P2) research(P7)     L4
│   ├── polaris-rpc                         L5 ........................................... P9
│   └── polaris-ui                 (facade) L6 ........................................... P1
│       └── crates/ model(P1) cli(P1) gui(P9) web(P9)                                      L6
├── tests/                     polaris-tests (L8, P0): common helpers lib; suites in tests/tests/; fixtures/
│   ├── spike/ ratelimit(P3) bus(P1) s1(P4) sandbox(P2) web(P9)      polaris-spike-* (L8)
│   ├── fixtures/README.md     fixture policy
│   └── fuzz/README.md         excluded; cargo-fuzz in WSL2 nightly
├── xtask/                     L8, P0 (Rust port of @prime/tools/workspace.py)
└── @prime/
    ├── PLAN.md  PHASES.md  PHASES_STATUS.md
    └── tools/ workspace.toml  workspace.py  chatlog_inventory.json   (dep-probe/ is regenerated, ignored)
```

Counts (VERIFIED by `workspace.py summary` and `cargo metadata`): **93 packages** = 6 apps + 27 top-level
crates + 53 nested crates + 1 tests + 5 spikes + 1 xtask; 13 facades; **252 internal edges**; layers
L0:1 · L1:18 · L2:28 · L3:17 · L4:9 · L5:2 · L6:5 · L7:6 · L8:7.

---

## 9. Commands, directives and mentions

Grammar owner: `polaris-kernel-directives` (recursive descent; see its module plan in §13 for EBNF).
Canonical spelling follows initial.md (`$subagent-models(model(lo-hi), …)`); the demo's `$subagents-model=[…]`
/ `=`-list spelling is accepted as an alias. `$` directives are **session-scoped** and persisted for resume (D11); a bare name means `on`; unknown
`$name(...)` is kept as custom metadata; directives inside code spans/attachments are ignored.

### 9.1 `$` directives

| Directive | Arguments | Semantics | Owner crate | Phase | Source |
|---|---|---|---|---|---|
| `$websearch` | `on`\|`off` (bare = on) | enable Tavily/SearXNG research | search | P5 | init, msg 0/1 |
| `$subagents` | `on`\|`off` | allow child frames | runtime-frames | P3 | init |
| `$subagent-models` | `model(lo-hi), …` names `low..max` or `1..5` | effort→model mapping per model | models | P3 | init, msg 1/3 |
| `$links` | `url, …` | fetch, sift and add to RAG | search, nvidia-docs | P5 | init |
| `$skills` | `skill, …` | mount procedural skills | ext-skills | P7 | init |
| `$mcp` | `server, …` | connect MCP servers; S1 prunes tools | ext-mcp | P7 | init |
| `$rag` | `source, …` | local vector sources | storage-vector, search | P5 | init |
| `$nvidia` | bare/`auto` \| `cuda, hopper, openshell, nemoclaw, …` \| `off` | NVIDIA profile (auto-detect or manual) | nvidia-docs | P5 | msg 5–7 |
| `$nvidia-docs` | `sync` | background docs refresh | nvidia-docs | P5 | msg 5/6 |
| `$sandbox` (alias `$nebius-sandbox`) | `auto`\|`openshell`\|`microvm`\|`contree`\|`remote` | execution tier | openshell, sandboxes | P2/P3 | msg 9, 25 |
| `$nebius-branching` | `on, max_branches=N` | parallel branch search in Contree | sandboxes | P6 | msg 9 |
| `$nebius-datalab` | `on`\|`off` | opt-in trajectory export | data-lab | P6 | msg 9 |
| `$nebius-speculative` | `on`\|`off` | cascade drafting (C15) | models | P6 | msg 9 |
| `$concurrency` | `web=N, inference=N, io=N, …` | pool sizes | runtime-dag | P7 | msg 17 |
| `$ratelimit` | `rpm=N, tpm=N` | declare plan quotas | ratelimit | P3 | msg 19 |
| `$budget` | `usd=N` and/or `tokens=N` | FinOps warnings | telemetry | P6 | msg 9 |
| `$storage-level` | `global`\|`isolated` | cache scope | storage-cache | P5 | msg 19 |
| `$browser` | `on`\|`off` | interactive browser tools | browser | P7 | msg 25 |
| `$needle` | `on`\|`off` | Tier-0 argument filling | system-one-nano | P8 | msg 33 |
| `$env` | `nix`\|`docker`\|`host` | toolchain environment | devenv | P8 | msg 41 |
| `$ci` | `dagger` | built-in CI backend | devenv-cicd | P8 | msg 41 |

### 9.2 `/` commands

| Command | Semantics | Owner | Phase | Source |
|---|---|---|---|---|
| `/model` | pick model/provider/effort | models, ui | P3 | init |
| `/usage` | tokens/cost/latency by model | telemetry | P3 | init |
| `/stats` | S1 vs S2 time, cache hits, sandbox load, activity | telemetry | P1 | init |
| `/status` | workspace/session, settings with source layer | kernel | P1 | demo |
| `/quit` (alias `/exit`) | graceful stop, teardown, persist | kernel, reaper | P1 | init |
| `/plan` | show/rebuild plan tree; plan mode | runtime | P3 | init |
| `/goal` | set/modify root objective | runtime | P3 | init |
| `/diff` | diff of agent changes | vcs | P2 | msg 1 |
| `/rollback` | roll back agent actions | vcs | P2 | msg 1 |
| `/undo` | undo last change | vcs | P2 | msg 37 |
| `/sessions` | list sessions | storage-registry | P1 | msg 19 |
| `/session resume <id>` | resume incl. leases, settings, sandbox state | kernel | P1 | msg 19 |
| `/session replay [step]` | time-travel replay | storage-eventlog | P3 | msg 19 |
| `/settings` `/help` `/clear` | settings view, help, clear conversation | ui | P1 | demo |
| `/providers` `/onboarding` | provider keys; WSL2/OpenShell/keys setup wizard | security-secrets, openshell-nemoclaw | P2 | demo |
| `/nv-profile` | CUDA best-practice analysis | nvidia-docs | P5 | msg 5/6 |
| `/nv-verify` | strict CUDA audit (cudaGetLastError, grid bounds, smem OOB) | nvidia-docs, system-one-verifier | P5 | msg 5/6 |
| `/nebius-status` | Token Factory/Contree status, quota | nebius-cloud | P6 | msg 9 |
| `/contree-branches` | branch tree | sandboxes | P6 | msg 9 |
| `/datalab-sync` `/datalab-finetune` | export trajectories; trigger fine-tune | data-lab | P6 | msg 9 |
| `/concurrency-stats` | pools, queues, aborts | runtime-dag | P7 | msg 17 |
| `/cache stats` `/cache purge` | cache tiers | storage-cache | P5 | msg 19 |
| `/quota` | RPM/TPM headroom, leases, sandboxes | ratelimit | P3 | msg 19 |
| `/research <goal>` `status` `report` `resume` | autoresearch loop | runtime-research | P7 | msg 20/21 |
| `/context-map` `/context-expand <ref>` | context budget; expand folded ref | runtime-context | P5 | msg 23 |
| `/parallel-dag` | task graph view | runtime-dag | P7 | msg 23 |
| `/screenshot` `/browser goto <url>` `/browser tree` `/ui-diff` | visual tools | browser | P7 | msg 25 |
| `/hack` (alias `/redteam`) | zero-token adversarial audit | hacker | P7 | msg 33/35 |
| `/search-local "<q>"` | local semantic search | search, system-one-nano | P5/P8 | msg 33 |
| `/ci` `/ci-run` | built-in CI pipeline | devenv-cicd | P8 | msg 41 |
| `/nix-flake` | draft + evaluate flake.nix | devenv-nix | P8 | msg 41 |
| `/proof-of-done` | run the completion gate explicitly | runtime-guards | P2 | msg 37 |

### 9.3 Mentions and model tools

`@file`, `@docx(path)`, `@pdf(path)`, `@xlsx(path)`, `@img(path)`, `@scans` (force OCR) — owner
`polaris-ingest` (P5) and `polaris-kernel-directives` (P1 parsing). Model-facing tools: `polaris_expand(ref)`,
`polaris_recall(query)` (runtime-context, P3), plus the builtin tool catalog (§13, `polaris-tools-builtin`).

---

## 10. Dependencies & versions

All versions VERIFIED on crates.io on 2026-10-20 (sparse index audit: 156/156 OK, every `rust_version`
≤ 1.99). The authoritative list with features is `@prime/tools/workspace.toml` → root `Cargo.toml`
`[workspace.dependencies]`.

| Group | Crates (pinned) |
|---|---|
| Async | tokio 1.54.0 · tokio-util 0.7.20 · tokio-stream 0.1.19 · futures 0.3.34 · bytes 1.12.1 |
| Serde/zero-copy | thiserror 2.0.21 · serde 1.0.229 · serde_json 1.0.154 · toml 1.1.8 · serde-saphyr 2.0.0 · postcard 1.1.3 · zerocopy 0.8.63 · bytemuck 1.25.2 |
| Net | reqwest 0.13.5 · hyper 1.12.0 · hyper-util 0.1.21 · http 1.5.0 · axum 0.8.9 · axum-extra 0.12.6 · tower 0.5.3 · tower-http 0.6.11 · tokio-tungstenite 0.26.2 · rustls 0.23.46 · tokio-rustls 0.26.6 · rcgen 0.14.10 · eventsource-stream 0.2.3 · url 2.5.8 |
| gRPC | tonic / tonic-prost / tonic-prost-build 0.14.6 · prost / prost-types 0.14.4 · protoc-bin-vendored 3.3.0 · openshell-sdk (git rev 5a7728c3) |
| IPC/log | iceoryx2 0.10.0 · tracing 0.1.44 · tracing-subscriber 0.3.23 · spdlog-rs 0.5.3 |
| Regex | regex 1.13.1 · regex-automata 0.4.18 · regex-syntax 0.8.11 · aho-corasick 1.1.5 · memchr 2.8.3 |
| Hash/cache | blake3 1.8.7 · xxhash-rust 0.8.19 · foldhash 0.2.0 · hashbrown 0.17.1 · dashmap 6.2.1 · quick_cache 0.7.0 |
| Security | secrecy 0.10.3 · zeroize 1.9.1 · chacha20poly1305 0.11.0 · biscuit-auth 6.0.0 · rand 0.10.3 · getrandom 0.4.3 · keyring 4.2.0 |
| Storage | redb 4.4.0 · rusqlite 0.40.2 (bundled) · usearch 2.26.5 (default vector store, S0-9) · lancedb 0.40.0 (opt-in, `features = ["remote"]`) · zstd 0.14.3 |
| ML | ort =2.0.0-rc.13 · tokenizers 0.23.2 · gliner2-rs 0.9.6 · hf-hub 1.0.0 · ndarray 0.17.2 |
| Documents | docx-rs 0.4.22 · calamine 0.37.0 · pdf-extract 0.12.1 · pdfium-render 0.9.4 · ocrs 0.13.1 · rten 0.26.0 · oar-ocr 0.10.0 · image 0.25.10 · image_hasher 3.1.1 · pulldown-cmark 0.13.4 · scraper 0.27.0 · htmd 0.5.6 · encoding_rs 0.8.42 · infer 0.22.0 |
| Code | tree-sitter 0.27.1 · tree-sitter-{rust 0.24.2, python 0.25.0, cpp 0.23.4, cuda 0.21.2, typescript 0.23.2} · ast-grep-core 0.51.0 · ast-grep-config 0.51.0 · gix 0.89.0 · imara-diff 0.2.0 · similar 3.2.0 |
| OS | ignore 0.4.33 · globset 0.4.20 · walkdir 2.5.0 · notify 8.2.0 · notify-debouncer-full 0.7.0 · tempfile 3.27.0 · dunce 1.0.5 · etcetera 0.11.0 · fs4 1.1.0 · sysinfo 0.39.6 · process-wrap 10.0.1 · portable-pty 0.9.0 · which 8.0.6 · shlex 2.0.1 |
| TUI | ratatui 0.30.2 · crossterm 0.29.0 · ratatui-textarea 0.9.3 · syntect 5.3.0 (fancy-regex) · two-face 0.5.2 · ansi-to-tui 8.0.1 · arboard 3.7.0 · unicode-width 0.2.2 · unicode-segmentation 1.13.3 |
| GUI/Web | slint / slint-build 1.18.1 · leptos 0.8.22 · leptos_axum 0.8.10 · leptos_router 0.8.17 · leptos_meta 0.8.7 · wasm-bindgen 0.2.129 |
| Agents | chromiumoxide 0.9.1 · rmcp 3.5.1 · bollard 0.21.1 · dagger-sdk 0.21.10 (feature-gated) |
| Math | exmex 0.21.0 (replaces evalexpr: ≥ 12 is AGPL-3.0-only) · rust_decimal 1.43.0 · splines 5.0.0 · statrs 0.19.1 |
| Resilience | governor 0.10.4 · backon 1.6.0 · arc-swap 1.9.3 · rayon 1.12.0 · parking_lot 0.12.5 |
| CLI/schema | clap 4.6.7 · schemars 1.2.3 · jsonschema 0.59.0 · miette 7.6.0 |
| IDs/time/mem | uuid 1.28.0 · jiff 0.2.38 · mimalloc 0.1.52 · bumpalo 3.20.3 |
| Test | proptest 1.12.0 · insta 1.49.0 · criterion 0.8.2 · divan 0.1.21 · rstest 0.28.0 · wiremock 0.6.5 · assert_cmd 2.2.2 · predicates 3.1.4 · loom 0.7.2 · hdrhistogram 7.6.0 · dhat 0.3.3 (zero-alloc tests, ADR-003) |
| v2 (commented) | whisper-rs 0.16.0 · cpal 0.18.2 · boringtun 0.7.1 |

**Resolution probe** (`python -I @prime/tools/workspace.py probe`, VERIFIED): first run failed —
`oar-ocr-core 0.10.0` requires `ort =2.0.0-rc.13` while we pinned rc.15; `ort-sys` declares `links`, so
one version per graph → pinned `=2.0.0-rc.13` (has `load-dynamic`, `api-23`). Second run: **1,691
packages locked**; after adding `usearch`, `ast-grep-config` and `dhat`: **1,699 packages locked**. Duplicates reviewed: reqwest ×3 (0.11 dagger, 0.12 lance/openshell, 0.13 own),
rustls ×2, hyper 0.14 (dagger), ndarray 0.16 (lance-index), quick_cache 0.6 (lance-core), redb 3.1 and
statrs 0.18 (hf-xet), similar 2.7 (insta), rand 0.8/0.9 (biscuit-auth and others), windows-sys ×4 —
all third-party; our own `rten` was aligned to 0.26.0 to remove a duplicate. `ring` present (ADR-005).

**Dropped vs. chatlog/draft** (with reason): sled (C12), CockroachDB (C11), figment (0.10.19 has no TOML
provider feature; config uses `toml` + serde), lopdf direct (pdf-extract wraps it), tracing-appender
(spdlog-rs sinks), libfuzzer-sys/arbitrary (live in the excluded fuzz manifest), Tauri (D6),
ed25519-dalek direct (biscuit-auth includes it), `nix` crate (POSIX-only; process-wrap/rustix cover needs),
tesseract bindings (native libs on Windows; OCR pair is ocrs + oar-ocr), Firecracker SDK crates (D2).

---

## 11. Test strategy

### 11.1 Categories (user-requested) and where they live

| Category | Location | Runner | Gate |
|---|---|---|---|
| common | `tests/src/lib.rs` (`polaris-tests` helpers) | — | — |
| fixtures | `tests/fixtures/` (+ MANIFEST.toml provenance) | — | policy in `tests/fixtures/README.md` |
| unit / proptest | each crate (`src` + `crates/*/tests/`) | `cargo test` / nextest | every phase exit |
| smoke | `tests/tests/smoke.rs` | nextest `default` | every PR |
| e2e / contract / security | `tests/tests/{e2e,contract,security}.rs` | nextest | phase exits; contract = live re-record when keys exist |
| load | `tests/tests/load.rs` (feature `heavy`) | nextest `heavy` profile | P10 + perf-sensitive phases |
| scalability | `tests/tests/scalability.rs` (feature `heavy`) | nextest `heavy` | P5, P7, P10 |
| spike | `tests/spike/*` packages | nextest `heavy` | owning phase |
| stress | `tests/tests/stress.rs` (feature `heavy`) | nextest `heavy` | P10 |
| fuzz | `tests/fuzz` (excluded; WSL2 nightly) | cargo-fuzz | parsers' phases; nightly runs |
| verification | Kani (WSL2), Miri (pure crates), loom (bus, leases, caches) | per crate | owning phase |
| mutation | cargo-mutants on core logic | WSL2/Windows | P10 |
| benchmarks | criterion/divan per crate | — | perf regressions |

### 11.2 Test-first process (prompt.md §6.3)

1. Write the JSON descriptor in `specs/descriptors/<logic_name_snake>.json` (schema below).
2. Generate the failing test file (external utility "provided separately"; if absent by P1, `xtask
   scaffold-tests` implements it — declared). Unit-logic tests go to the owning crate's `tests/`;
   cross-crate suites to `tests/tests/`. New failing tests carry `#[ignore = "red: <descriptor>"]` until
   implemented, so CI stays honest and green.
3. Run and **observe** the failure for the right reason; record `TestsFailing` evidence.
4. Implement; run; record `PASS` evidence (command + raw output + conditions) in PHASES_STATUS.md.

Example descriptor (the canonical shape; every module plan lists its descriptors in short form):

```json
{
  "type_of": "function",
  "name": "parse",
  "file_path": "crates/polaris-kernel/crates/directives/src/lib.rs",
  "logic_name": "Directive Grammar",
  "test_list": [
    { "test_name": "bare_directive_means_on", "description": "`$websearch` yields WebSearch(On)." },
    { "test_name": "nested_model_effort_ranges", "description": "`$subagent-models(super(high-max), nano(1-2))` parses names and numbers." },
    { "test_name": "unknown_directive_kept_as_metadata", "description": "`$foo(bar)` is preserved as custom metadata." },
    { "test_name": "directive_inside_code_span_ignored", "description": "Backticked `$websearch` is not applied." },
    { "test_name": "invalid_range_rejected_draft_untouched", "description": "`(max-low)` returns an error and leaves state unchanged." }
  ],
  "dependencies": ["polaris-sdk"],
  "libraries": ["thiserror"],
  "inputs": [{ "name": "input", "type": "&str", "description": "Raw composer text." }],
  "output_type": "Result<ParsedInput, DirectiveError>",
  "output_description": "Typed directives, commands, mentions and remaining prompt text."
}
```

### 11.3 Shared rules

- No panics in tests either (prompt.md §7.1): tests return `Result` and use explicit assertions with messages.
- Keyed tests report **SKIPPED: no key** (never PASS) until keys exist; synthetic fixtures are labelled.
- Hostile fixtures are generated at test time (tests/fixtures/README.md).
- Every PASS needs pasted evidence (command, raw output, hardware/driver/versions).
- Zero-allocation tests (dhat testing mode, one test binary per check) guard the ADR-003 hot path.
- A Linux `cargo check` leg in WSL2 runs from P1.

### 11.4 Budgets (targets, not claims; measured and recorded when first run)

| Area | Target |
|---|---|
| Bus publish→subscribe p99 | < 50 µs in-process |
| GCRA check | < 1 µs, zero alloc |
| DFA scan | ≥ 500 MB/s single core |
| SSE frame parse | zero alloc per frame |
| S1 multi-task decision p95 | CPU ≤ 400 ms; CUDA EP ≤ 80 ms (S0-4) |
| Sync 10k-file repo, no changes | < 2 s manifest diff |
| TUI frame render | < 8 ms at 200×60 |

### 11.5 Fixtures catalogue (planned)

Sample repos (Rust/Python/CUDA), cargo JSON + text diagnostics (RTK), nvcc logs, synthetic Token Factory
SSE streams (labelled), Contree MCP transcripts (synthetic until live), NVIDIA doc pages (fetched,
provenance), PII corpora (generated), prompt-injection corpora, malicious diffs (generated), docx/xlsx/
pdf/pptx/scanned-pdf samples (self-made), S1 golden outputs (Python reference), screenshots for pHash.

### 11.6 Per-crate test matrices

Each module plan in §13 has a **Tests** table and **Test-first descriptors** table.

### 11.7 MVP acceptance (end of P4)

A scripted demo, recorded: three fixed tasks (Rust bug fix, Python feature with tests, CUDA-flavoured
kernel check). Nemotron on Token Factory edits; tests run in an OpenShell sandbox (and/or Contree);
ProofOfDone **blocks** a deliberately falsified completion (deleted assertion); the TUI streams;
transcript saved. Fallback path for judges without WSL2: E3/E4.

---

## 12. Windows pitfalls checklist

| # | Pitfall | Handling | Phase |
|---|---|---|---|
| W1 | MAX_PATH in deep `target\…\build\aws-lc-sys-*` paths | LongPathsEnabled=1 + `git config core.longpaths true` (per PC, §0.1); a short `target-dir` was **not needed** on the reference PC (P0-ENV-4) — set one in the user-level Cargo config only if a build fails | P0 (done) |
| W2 | CRLF breaking scripts/YAML/Nix in WSL | `.gitattributes` (done); byte-exact sync; git `autocrlf=false` in sandboxes | P0 (done) / P2 |
| W3 | Case-insensitive NTFS vs case-sensitive sandbox; reserved names (`con`, `nul`, `aux`, `prn`) | sync detects and reports; fixtures generated at runtime | P2 |
| W4 | Defender slowing `target/` and quarantining fixtures | exclusions documented; hostile fixtures generated | P0 |
| W5 | Stale `onnxruntime.dll` in System32 | ort `load-dynamic` with absolute path; DLL beside exe; doctor check | P4 |
| W6 | CUDA EP needs CUDA 12.x + cuDNN 9 + VC++ redist on PATH | doctor check; CPU fallback | P4 |
| W7 | protoc absent; lance build scripts need `PROTOC` (VERIFIED, S0-9) | `protoc-bin-vendored` for own crates; protoc on PATH per PC (`win-protoc`), only for the opt-in `lancedb` feature | P0 (done) |
| W8 | CMake 4 rejects old minimum versions | `CMAKE_POLICY_VERSION_MINIMUM=3.5` in `[env]` | P0 |
| W9 | tonic over named pipes needs custom connector; default DACL allows other local users | connector + pre-created instance + Biscuit auth | P4/P9 |
| W10 | No SIGTERM | CTRL_BREAK to process groups where possible, then Job Object kill | P2 |
| W11 | ConPTY input (Shift+Enter, Alt-codes, C0 controls) | Alt+Enter/Ctrl+J first; S0-10 | P1 |
| W12 | `notify` buffer overflow on checkout floods | treat as forced rescan | P2 |
| W13 | `keyring` blob limit (~2.5 KB) | store keys only; sealed vault on disk for larger data | P3 |
| W14 | WSL2 idle shutdown; Docker Desktop VM ≠ Ubuntu VM; Hyper-V firewall blocks WSL→host | keepalive/systemd; `host.docker.internal`; host-initiated traffic only | P2 |
| W15 | Mirrored vs NAT networking; VPN subnet clashes | doctor detects mode; documented | P2 |
| W16 | Atomic rename fails while AV/indexer holds the file | temp + fsync + rename with a bounded retry on OS errors 5/32/33 (`polaris-storage-cas::atomic`, VERIFIED with a real share-mode-0 handle) | P1 (done) |
| W17 | 16 GB RAM shared by Docker VM, WSL VM, S1, compilers | `.wslconfig` memory cap ≈ 6 GB; model eviction | P0/P4 |
| W18 | Kani/fuzz unsupported or weak on Windows | run in WSL2 | P0+ |
| W19 | TLS provider ambiguity | install aws-lc-rs provider at start (ADR-005) | P1 |
| W20 | cargo-fuzz defaults `--target` to its own build triple; the cargo-binstall build is musl, so fuzz builds fail with `can't find crate for core` (VERIFIED) | always pass `--target x86_64-unknown-linux-gnu` (`verify-proofs.sh`) | P1 (done) |
| W21 | Restoring a source file with its old mtime (backup copy/move) leaves cargo linking the stale build — a mutation check then "fails" on correct code (VERIFIED) | touch restored files; prefer cargo-mutants, which handles this | P1 |
| W22 | OpenShell gateway: 1 MiB request limit, ~3 s listen delay after a WSL boot, `/opt` not readable/executable in sandboxes, downloads only from `/sandbox` (VERIFIED, S0-1/S0-7/S0-12) | chunk ≤ 896 KiB; connect retry; images under `/usr`; artifacts under `/sandbox` | P2 |
| W23 | iceoryx2 creates `C:\Temp\iceoryx2` writable by all authenticated users (VERIFIED, S0-8) | per-user `set_root_path` + owner-only ACL before any use | P4+ |
| W24 | A whole-parser Kani harness exhausts WSL memory (CBMC > 3.9 GB after 11 min on a 4-byte input) | keep harnesses small (3 bytes, 6-char alphabet) and prove components separately | P1 |

---

## 13. Module plans

Each section below is the per-crate plan: purpose, API sketch, dependencies (pinned), TODO checklist,
test matrix, test-first descriptors, risks, and chatlog sources. Package metadata (path, layer, phase,
edges) is generated from `@prime/tools/workspace.toml`; if this text and the table disagree, the table wins.

## 13.1 Group G1 — Foundations

Scope: 20 packages (L0 sdk; L1 foundations; L3 security domain). All labels follow the truth kernel: **VERIFIED** (checked against `workspace.toml`, the PLAN core (§0–§12) or `prompt.md` while drafting this section), **LIKELY**, **UNVERIFIED**. No API of a third-party crate below was compiled or run; every third-party call is marked "API to confirm at implementation". All numeric budgets are **targets to be measured**, never results. D10 applies: no lint attributes or lint config anywhere; the `prompt.md` rules (no `unwrap`/`expect`/`panic`/indexing/`as`, `Result` + `thiserror`, typestate, generics over `dyn`) are followed by discipline and review.

Cross-cutting conventions used by every crate in this group:
- Hot-path set is closed (plan): SSE frame parser, bus publish/consume, GCRA check, DFA/PII scan, `§ref` hash/fold, lease lookup, log enqueue. Only functions in that set carry `// hot path: zero-alloc`; they are verified by the counting `#[global_allocator]` wrapper in `polaris-tests`.
- Blocking I/O crates (cas, eventlog, registry, vector, sync) expose **synchronous** APIs; async callers wrap them in `spawn_blocking` or a dedicated owner thread. They never run SQLite/redb/zstd work on a tokio worker.
- Async traits in the registry use return-position `impl Future<Output = ...> + Send` (no `dyn`, no boxed futures), per `prompt.md` §6.2 and the composition-root ADR.
- Test-first JSON descriptors (§6.3) live in `specs/descriptors/<crate>/*.json`; generated tests land in the owning crate's `tests/` directory and stay `#[ignore]`d until implemented.
- Descriptor tables below list `logic_name | type_of | name | file_path | tests`; the full JSON (inputs, outputs, dependencies, libraries) is produced when the crate's phase starts.

---

### `polaris-sdk` — Domain newtypes, error kinds, API-registry traits, Backends bundle, PlatformOps, command/event contracts

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-sdk` | L0 | lib | — | P1 |

**Purpose.** The only crate every other crate may depend on. It owns the vocabulary (newtypes, error taxonomy), the internal API registry (`prompt.md` §6.2: one trait per external concern, business logic depends only on the trait), the `Backends` associated-type bundle that the composition root instantiates, and the CQRS `Command`/`Event` contracts. It contains no I/O and no concrete third-party adapters, so it is Miri-friendly and Kani-friendly.

**API sketch.**
```rust
// ids.rs — newtypes, no primitive obsession; small ones are Copy
pub struct SessionId(Uuid);                 // uuid v7: time-ordered
pub struct FrameId(Uuid);
pub struct Seq(u64);                        // event-log sequence number
pub struct Cid([u8; 32]);                   // BLAKE3 content id (CAS key)
pub struct ShortRef([u8; 6]);               // first 48 bits of a Cid; renders as 12 lowercase hex in `§ref:<12hex>`
pub struct FastFingerprint(u64);            // xxh3-64; NOT collision-resistant: dirty-checks only, never a security key
pub struct ModelId(Box<str>);               // validated token-factory model id
pub struct MicroUsd(u64);                   // integer money, checked arithmetic only
pub struct TokenCount(u32);
pub struct Probability(f32);                // validated in 0.0..=1.0, NaN rejected; compare via partial_cmp only
pub enum Effort { Low, Medium, High, XHigh, Max }   // FromStr accepts names and "1".."5"; TryFrom<u8>
pub enum AgentRole { Orchestrator, SubAgent, Verifier, Researcher, Hacker, Watchdog } // provisional set
pub enum ErrorKind { Transient, Permanent, Policy, Fatal }  // supervisor/circuit-breaker input
pub trait HasKind { fn kind(&self) -> ErrorKind; }          // implemented by every crate error enum

// system_one.rs — System-One primitives (Choice / Score / Noul), tagged serde (NOT untagged)
pub enum QuestionKind { Choice { options: Vec<Label> }, Score { min: i32, max: i32 }, Noul { statement: String } }
pub struct Question { pub id: QuestionId, pub kind: QuestionKind }
pub enum Answer { Choice { selected: Label, probs: Vec<(Label, Probability)> }, Score { value: i32, confidence: Probability }, Noul { yes: Probability } }
pub struct Decision { pub answers: Vec<(QuestionId, Answer)>, pub latency: core::time::Duration }

// registry.rs — API-registry traits; RPITIT futures, all fallible ops return Result<_, XError>
pub trait ChatModel: Send + Sync { fn stream(&self, req: ChatRequest) -> impl Future<Output = Result<ChatStream, ModelError>> + Send; }
pub trait EmbeddingProvider: Send + Sync { fn embed(&self, batch: &[&str]) -> impl Future<Output = Result<Vec<Vec<f32>>, ModelError>> + Send; }
pub trait DecisionEngine: Send + Sync { fn decide(&self, schema: &[Question], state: &DecisionInput) -> impl Future<Output = Result<Decision, DecisionError>> + Send; }
pub trait SandboxBackend: Send + Sync { /* create / exec / upload / download / list_files / read_file / destroy */ }
pub trait WebSearch: Send + Sync { fn search(&self, q: &SearchQuery) -> impl Future<Output = Result<Vec<SearchHit>, SearchError>> + Send; }
pub trait DocParser: Send + Sync { fn parse(&self, src: DocSource<'_>) -> Result<Document, IngestError>; }
pub trait VectorStore: Send + Sync { /* upsert(chunks) / search(vector, k) / delete / count */ }
pub trait EventStore: Send + Sync { fn append(&self, e: &EventRecord) -> Result<Seq, EventStoreError>; fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventStoreError>; }
pub trait KernelApi: Send + Sync { /* submit(Command<Verified>) / subscribe() -> EventStream / attach_terminal */ }
pub trait PlatformOps: Send + Sync { /* spawn_tree, graceful_stop (CTRL_BREAK then kill), pty, watch, memory_pressure */ }
pub trait Redactor: Send + Sync { fn redact_into(&self, input: &[u8], out: &mut Vec<u8>) -> RedactStats; } // caller-owned buffer
pub trait FunctionalOperator { type In; type Out; type Err; fn apply(&self, i: Self::In) -> impl Future<Output = Result<Self::Out, Self::Err>> + Send; }

// backends.rs — composition-root bundle (ADR) and enum-dispatch without dyn
pub trait Backends { type Chat: ChatModel; type Embed: EmbeddingProvider; type Decide: DecisionEngine; type Sandbox: SandboxBackend; type Search: WebSearch; type Store: EventStore; type Vectors: VectorStore; type Platform: PlatformOps; type Redact: Redactor; }
pub enum OneOf3<A, B, C> { A(A), B(B), C(C) }       // generic enum dispatch; impls forward every registry trait
// composition root: type Sandbox = OneOf3<OpenShellBackend, ContreeBackend, RemoteBackend>;

// contracts.rs — CQRS contracts (provisional variant set, versioned envelope)
pub struct TokenBytes(Bytes);                        // opaque capability token; sdk does not depend on biscuit
pub enum Unverified {} pub enum Verified {}          // typestate markers
pub struct Command<S> { /* id, frame, token: TokenBytes, body: CommandBody, _s: PhantomData<S> */ }
pub enum CommandBody { SubmitUserInput, ProposeDiff, AcquireAstScope, ReleaseAstScope, SpawnSubagent, ExecuteTool, RollbackTo, CancelFrame, ApplyDirective }
pub enum Event { SessionStarted, DirectiveApplied, DecisionMade, ModelRequestCompleted, ToolExecutionStarted, ToolExecutionCompleted,
                 DiffApproved, DiffRejected, FileLeaseGranted, FileLeaseRevoked, SandboxSnapshotBranchCreated, SandboxExecutionCompleted,
                 ProofOfDonePassed, ProofOfDoneBlocked, WorkerRestarted, CircuitTripped, DeadLettered, SessionClosed,
                 SessionConfigured, SessionPaused, SessionResumed } // schema 2; schema-1 envelopes remain readable
pub struct EventRecord { pub seq: Seq, pub at: jiff::Timestamp, pub schema: u16, pub session: SessionId, pub event: Event }
```

**Dependencies.** Internal: none (L0). External: `thiserror = 2.0.21`, `serde = 1.0.229` (derive), `bytes = 1.12.1`, `uuid = 1.28.0` (v7, serde), `jiff = 0.2.38`, `secrecy = 0.10.3`. Dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/sdk_{content_identifiers,effort_parsing,probability_validation,command_typestate,event_envelope,registry_dispatch}.json` (tests written first; red = unresolved API, then green).
- [x] Implement newtypes with validated constructors (`TryFrom`, `FromStr`), `Debug` impls that never print secrets, and `HasKind` for the error taxonomy.
- [x] Implement `Effort` (names low/medium/high/xhigh/max and numeric 1-5, as the chatlog's 1-5 form must still parse; names in JSON, `u8` in postcard) and `Probability` (NaN/out-of-range rejected; `-0.0` normalized; total order).
- [x] Define the System-One types with `#[serde(tag = "type")]` (internally tagged); reject the chatlog's `untagged` `DecisionValue` (ambiguous decoding).
- [x] Define the 12 registry traits with RPITIT async methods; one error enum per concern, all `thiserror` — plus `ExactCacheStore` and `Tool`/`ToolSpec` (cross-crate flags). Streams are pull traits (`ChatChunkSource`, `EventSource`), so sdk needs no futures/tokio dependency.
- [x] Define `Backends` and `OneOf3`/`OneOf2` enum-dispatch wrappers with forwarding impls for every registry trait; no `dyn` anywhere (cold-path exceptions need written justification).
- [x] Define `Command<Unverified|Verified>` typestate: sealed `VerificationState` with an associated proof (`()` / `ApprovalWitness`); `verify(witness)` is the only path to `Command<Verified>`; only `Command<Unverified>` implements `Deserialize`.
- [x] Workspace check restricting `ApprovalWitness::new` call sites to `polaris-sdk` and `polaris-kernel-verifier`: `cargo xtask check-witness` (test-first, descriptor `xtask_check_witness.json`).
- [x] Define `Event` v1 envelope (`schema: u16`, unknown/zero schema rejected on decode) and the insta snapshot that freezes its JSON shape. Postcard snapshot: added with the eventlog codec (postcard is not an sdk dependency).
- [x] Define `PlatformOps` with Windows semantics in the doc comments: graceful stop is CTRL_BREAK where available, then kill-tree via Job Object; no SIGTERM. (`pty`/`watch` join when `polaris-platform-process` starts in P2.)
- [x] Add `compile_fail` doctests: `Command<Unverified>` is not accepted as `Command<Verified>`; `Command<Verified>` is not `Deserialize`; `FastFingerprint` does not convert to `Cid` (each with a compiling positive twin). Token redaction is a runtime test (`token_bytes_debug_is_redacted`).
- [x] Run Miri over the crate (27 integration tests; `event_record_json_snapshot` skipped under Miri because insta calls `socketpair`, unsupported by Miri) and Kani harnesses for `Effort::try_from(u8)` and `Probability::new` (`src/kani_proofs.rs`, feature `kani`): both VERIFIED. Runner: `@prime/tools/setup/verify-proofs.sh`.

**Tests.**

| Category | Cases / properties (budgets are targets to be measured) |
|---|---|
| unit | `Cid`/`ShortRef` hex round-trip; `Effort` accepts `low`, `XHIGH`, `3`, rejects `0`, `6`, `""`; `Probability::new(f32::NAN)` is `Err`; `ModelId` rejects empty/oversized/control chars |
| proptest | any `Cid` -> `ShortRef` -> hex -> parse is stable; `Effort` order is monotone with its numeric form; serde round-trip of `EventRecord` for generated events |
| fuzz | `fuzz_cid_from_hex`, `fuzz_shortref_parse`, `fuzz_effort_parse` (parsers must return `Err`, never panic) |
| kani / miri | Kani: `Effort::try_from` total over `u8`; `Probability::new` never returns a value outside 0..=1. Miri: full crate |
| fixtures | `insta` snapshots of `Command`/`Event` JSON and postcard bytes for schema v1 (wire-stability gate) |
| security | `compile_fail` doctests: `SecretString`-wrapping types have no `Debug`/`Display` leak; `Command<Verified>` cannot be built from outside the sanctioned constructor |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Content Identifiers | struct | `Cid` / `ShortRef` | `crates/polaris-sdk/src/ids.rs` | `cid_hex_roundtrip`, `short_ref_is_first_48_bits`, `short_ref_renders_12_lowercase_hex`, `cid_rejects_wrong_length_hex`, `fast_fingerprint_has_no_cid_conversion` |
| Effort Parsing | function | `Effort::from_str` | `crates/polaris-sdk/src/effort.rs` | `parses_names_case_insensitively`, `parses_numeric_1_to_5`, `rejects_zero_and_six`, `display_roundtrips_name` |
| Probability Validation | struct | `Probability` | `crates/polaris-sdk/src/system_one.rs` | `rejects_nan`, `rejects_above_one`, `accepts_bounds`, `ordering_is_total_for_valid_values` |
| Command Typestate | struct | `Command<S>` | `crates/polaris-sdk/src/contracts.rs` | `verified_requires_approval_witness`, `command_serde_roundtrip`, `token_bytes_debug_is_redacted`, `handler_requires_verified_command` (compile_fail), `verified_command_cannot_be_deserialized` (compile_fail) |
| Event Envelope | struct | `EventRecord` / `Event` | `crates/polaris-sdk/src/contracts.rs` | `event_record_json_roundtrip`, `schema_version_is_one`, `event_is_fixed_size_copy`, `event_record_json_snapshot` |
| Registry Dispatch | trait | `OneOf2` / `OneOf3` | `crates/polaris-sdk/src/backends.rs` | `one_of2_forwards_event_store_to_active_variant`, `one_of3_forwards_redactor`, `one_of2_forwards_async_chat_stream`, `error_kinds_cover_taxonomy` |

**Risks & open questions.**
- Cross-crate typestate: `pub(crate)` constructors cannot protect `Command<Verified>` from other crates. **Decided (P1):** sealed state trait + `ApprovalWitness` + `verify()` as the only promotion path + no `Deserialize` for verified commands (VERIFIED by compile_fail doctests); `ApprovalWitness::new` is public, so `cargo xtask check-witness` restricts its call sites.
- `secrecy` 0.10 API: `prompt.md` still says `Secret<T>`; 0.10 renamed to `SecretBox<T>`/`SecretString`. **LIKELY**; API to confirm at implementation.
- `AgentRole` set and the `Command`/`Event` variant list are provisional (derived from chatlog msgs 12-13 and 31); the owning semantics live in kernel/runtime crates. Changing a variant after P1 breaks stored event logs (postcard is not self-describing).
- RPITIT in public traits cannot be used as `dyn`; that is intended (VERIFIED against the plan's composition-root ADR), but it forces generics through the whole kernel and may inflate compile times. **LIKELY** acceptable.
- `workspace.toml` lists `trait ChatModel, DecisionEngine, SandboxBackend, Backends, enum Command, enum Event, struct Decision` as the headline API; this section expands it (e.g. `Command<S>` is generic over typestate rather than a plain enum). Flag for the toml owner.

**Chatlog sources.** Msg 1/3 (Choice/Score/`noul` primitives, `DecisionSchema`; corrected: `untagged` serde and `todo!()` bodies rejected), 12-13 (Command/Event names, `PolarisCommand`/`PolarisEvent`, Biscuit token inside commands), 14/15 (`FastFingerprint`, `CryptoContentId` -> `Cid`; corrected: xxh3 is non-cryptographic), 21 (`DocumentContent`), 27 (Redactor/Reaper contracts). Corrections from the plan: stale model names removed (`ModelId` is validated, not hard-coded), effort levels now map low..max to Nano-30B..Ultra-550B at the models layer, `PlatformOps` seam added for the later macOS/Linux port.

---

### `polaris-proto` — Protobuf contracts and generated tonic/prost code for kernel, worker and guest services

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-proto` | L1 | lib | — | P1 |

**Purpose.** Owns `proto/polaris/v1/{kernel,worker,guest}.proto`, generates Rust in `build.rs` with a vendored `protoc` (no system protoc needed; protoc is VERIFIED missing on this machine), and keeps generated code isolated from hand-written crates. It also holds the sdk <-> proto conversions and the `PROTOCOL_VERSION` constant used in the `polaris-guest` handshake. It contains **definitions only**; transports (named pipe, TCP+mTLS) belong to `polaris-rpc` and the worker/daemon apps.

**API sketch.**
```rust
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };
pub struct ProtocolVersion { pub major: u16, pub minor: u16 }
pub fn negotiate(local: ProtocolVersion, remote: ProtocolVersion) -> Result<ProtocolVersion, ProtoError>; // major must match

// generated (prost/tonic), re-exported under `generated::polaris::v1`
// service PolarisKernelService { SubmitCommand; Subscribe(stream Event); AttachTerminal(bidi); Health }
// service WorkerService       { Handshake; Decide(batch); Embed; Ingest; EvictModels; Health; Shutdown }
// service GuestService        { Handshake(protocol_version); Exec(stream); ApplySyncDelta; Screenshot; A11yTree; Health }

// conversions: fallible, never panicking
impl TryFrom<pb::Event> for polaris_sdk::Event { type Error = ProtoError; /* ... */ }
impl From<&polaris_sdk::Event> for pb::Event { /* ... */ }
pub enum ProtoError { MissingField(&'static str), UnknownVariant { field: &'static str, value: i32 }, VersionMismatch { local: ProtocolVersion, remote: ProtocolVersion }, Oversize { limit: usize } }
```

**Dependencies.** Internal: `polaris-sdk`. External: `prost = 0.14.4`, `prost-types = 0.14.4`, `tonic = 0.14.6` (root: `default-features = false`; member opts into `codegen` plus `transport` as needed), `tonic-prost = 0.14.6`; build-deps `tonic-prost-build = 0.14.6`, `protoc-bin-vendored = 3.3.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: version negotiation, sdk<->pb conversions (Event, Command, Decision), oversize-message guard.
- [ ] Draft the three `.proto` files; reserve field numbers; use `oneof` for `Event`/`Command` bodies; blobs are passed as CAS `Cid` references, not inline bytes.
- [ ] Write `build.rs`: point prost/tonic builders at the vendored protoc binary; emit to `OUT_DIR`; `cargo:rerun-if-changed=proto`; no network, no env mutation.
- [ ] Re-export generated modules under one `generated` module; confirm `cargo doc --workspace --no-deps` passes with generated code.
- [ ] Implement `ProtocolVersion` + `negotiate`; the guest handshake rejects a differing major version.
- [ ] Implement `TryFrom`/`From` conversions with exhaustive matching (no wildcard arms) and typed `ProtoError`.
- [ ] Configure explicit max encode/decode message sizes for client/server stubs and document them (large payloads travel as CAS refs).
- [ ] Commit a golden `FileDescriptorSet` snapshot and a wire-compat check (field removal/renumber fails CI) in xtask.
- [ ] Windows: verify the build on `x86_64-pc-windows-msvc` with a short `target-dir`; confirm the vendored protoc binary runs without a system protoc; add the Linux check leg in WSL2.

**Tests.**

| Category | Cases / properties |
|---|---|
| unit | version negotiation matrix (same major ok, different major rejected); conversion of every `Event` variant both ways |
| proptest | generated sdk `Event`/`Decision` -> pb -> bytes -> pb -> sdk equals original |
| fuzz | `fuzz_proto_kernel_decode`, `fuzz_proto_worker_decode`, `fuzz_proto_guest_decode` (arbitrary bytes into prost decode + `TryFrom`; must return `Err`, never panic) |
| smoke | clean-clone `cargo check -p polaris-proto` on Windows with no protoc installed |
| fixtures | `insta` snapshot of the descriptor set; encoded golden bytes per message (synthetic, labelled so) |
| security | decoding an over-limit message is rejected; deeply nested message does not overflow the stack (prost recursion limit, **LIKELY** 100 — to confirm) |
| stress | 10k-message stream decode under a 64 MiB memory ceiling (target, to be measured) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Protocol Version Negotiation | function | `negotiate` | `crates/polaris-proto/src/version.rs` | `same_major_negotiates_to_min_minor`, `different_major_is_rejected`, `zero_major_is_rejected`, `negotiation_is_symmetric_on_success` |
| Event Conversion | impl_block | `TryFrom<pb::Event> for Event` | `crates/polaris-proto/src/convert.rs` | `every_event_variant_roundtrips`, `missing_oneof_body_is_error`, `unknown_enum_value_is_error`, `conversion_never_panics_on_default_message` |
| Message Size Guard | function | `check_message_len` | `crates/polaris-proto/src/limits.rs` | `at_limit_is_accepted`, `over_limit_is_rejected`, `limit_is_configurable`, `zero_length_is_accepted` |

**Risks & open questions.**
- tonic's default decode limit is 4 MiB (**LIKELY**; to confirm) — screenshots and AST dumps exceed it, so payloads must be CAS refs or chunked streams.
- The chatlog's iceoryx2 zero-copy path (msg 27) is **not** part of this crate: iceoryx2 is a later optimisation behind spike S0-8; gRPC is the baseline and the fallback.
- Generated code ignores house style (indexing etc.); D10 means no allow-attributes, so generated code is reviewed as a vendored artefact and kept in `OUT_DIR`.
- `tonic` feature set `tls-aws-lc` must stay ring-free (gate: `cargo tree -i ring` empty). VERIFIED as the plan's gate; the result is still pending.

**Chatlog sources.** Msg 27 (protobuf `.proto` contracts for host / guest / background workers; `prost`; iceoryx2 deferred to S0-8), msg 1 (`IPC / Local RPC` between frontends and kernel). Corrected: `prost 0.13` -> 0.14.4, `iceoryx2 0.4` -> 0.10.0 (not used here), guest `protocol_version` handshake added from the plan.

---

### `polaris-config` — Layered configuration: defaults < ~/.polaris < .polaris < env < session

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-config` | L1 | lib | — | P1 |

**Purpose.** Resolves the global (`~/.polaris`, i.e. `%USERPROFILE%\.polaris` on Windows) and repo (`.polaris/`) directories, merges config layers deterministically, and records which layer set each value so `/status` can show it. It holds secret *references* only (keyring entry names, env var names), never values. It also hosts the typed defaults for directives that map to config keys: `$ratelimit`, `$concurrency`, `$storage-level`, `$budget`, effort->model map.

**API sketch.**
```rust
pub enum Layer { Defaults, Global, Repo, Env, Session }       // ascending precedence
pub struct PolarisConfig { /* typed sections: nebius, ratelimit, concurrency, storage, sandbox, search, log, budget */ }
pub struct Provenance { /* key path -> Layer (and source file path) */ }
pub struct LayerInput { pub layer: Layer, pub table: toml::Table, pub origin: Option<PathBuf> }
pub fn load(layers: &[LayerInput], grant: &TrustGrant) -> Result<Loaded, ConfigError>; // pure merge + validation, no I/O (P1: takes the trust grant)
pub fn discover(cwd: &Path) -> Result<Dirs, ConfigError>;              // ~/.polaris and nearest .polaris/
pub fn read_layers(dirs: &Dirs, env: &EnvSnapshot, session: &SessionOverrides) -> Result<Vec<LayerInput>, ConfigError>;
pub struct Loaded { pub config: PolarisConfig, pub provenance: Provenance }
pub enum SecretRef { Keyring { service: String, user: String }, Env(String) }  // parsed from "keyring:..." / "env:..." only
pub struct SessionOverrides(BTreeMap<ConfigKey, toml::Value>);         // typed by kernel-directives; D11: persisted per session
pub enum ConfigError { Io(..), Parse { file: PathBuf, line: u32 }, UnknownKey(String), SecretLiteral(String), UntrustedRepoKey(String), Invalid { key: String, reason: String } }
// P1 additions: ConfigError::NoHome, ConfigError::Encode(String); Parse also carries `reason`.
```

**Dependencies.** Internal: `polaris-sdk`. External: `serde = 1.0.229` (derive), `toml = 1.1.8`, `etcetera = 0.11.0`, `dunce = 1.0.5`, `thiserror = 2.0.21`. (`figment` is in the root table but **not** in this crate's planned list; merge is implemented on `toml::Table` to keep provenance exact.)

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/config_{layer_merge,repo_layer_trust_filter,secret_reference_parsing,directory_discovery,golden_provenance}.json` (tests written first; red = unresolved API).
- [x] Define the typed config schema: an explicit `KNOWN_KEYS` list (unknown key -> `UnknownKey`, never ignored), typed readers (`Invalid { key, reason }` for wrong type/range) and documented defaults in `DEFAULTS_TOML` (model ids VERIFIED from the Token Factory catalog; rpm/tpm and permit counts are **unverified starting points**, see Risks).
- [x] Implement deterministic deep merge on `toml::Table` (tables merge, scalars replace; precedence from `Layer`, not input order; stable for equal layers) with per-key `Provenance` (`layer_of`, `origin_of`, `iter`).
- [x] Implement the **repo-layer trust filter** (`src/trust.rs`): a repo layer may set only `REPO_ALLOWED` = `ui.*`, `log.level`, `concurrency.*`, `ratelimit.*` without a `TrustGrant`; endpoints, secret refs, storage level and sandbox tier need the global layer or an explicit grant. Check order per layer: unknown key -> literal secret -> trust, so a leaked literal is always reported as `SecretLiteral`.
- [x] Implement `SecretRef` parsing (`keyring:service/user`, `env:[A-Z0-9_]+`); any credential-like key (`*api_key*`, `*token*`, `*secret*`, `*password*`) holding a literal -> `SecretLiteral(key)`; errors name the key, never the value.
- [x] Implement `discover`/`discover_from(cwd, home)`: `etcetera::home_dir`, `dunce::canonicalize`, walk up to the nearest `.polaris/`. **P1 decision:** the walk stops at home (exclusive) — `~/.polaris` is the global layer and directories above home are never projects (found when the developer's real `~/.polaris` was picked up as a "repo" dir from a temp dir under home).
- [x] Implement env layer: `POLARIS_` prefix, `__` nesting, lower-cased keys, integers/booleans typed, read with `std::env::vars_os` only; conflicting paths (`POLARIS_A` + `POLARIS_A__B`) -> `Invalid`; `EnvSnapshot`'s `Debug` prints names only.
- [x] Implement `SessionOverrides` (`set/get/remove`, `to_toml`/`from_toml`, `to_layer`) for `/session resume` (D11). Keys are dotted strings; typing happens at `load` (no separate `ConfigKey` type).
- [x] Windows: UTF-8 BOM and CRLF accepted; a file held without sharing (editor/AV) is retried (OS errors 32/33, 10 ms doubling to 200 ms, 8 attempts) — VERIFIED with a real share-mode-0 handle; non-ASCII + space paths and a >300-char path tested.
- [x] `budget.usd` keeps exact decimal text (string or integer; TOML floats rejected because they are already binary-rounded).

**Tests.**

| Category | Cases / properties |
|---|---|
| unit | precedence order across all five layers; missing files are not errors; unknown key -> `UnknownKey`; provenance names the winning layer |
| proptest | merge is associative for disjoint tables; merging a layer with itself is idempotent; provenance always points at an existing layer |
| fuzz | `config_toml_layer` (arbitrary bytes as a repo config: no panic; a layer that loads without a grant changed only allow-listed keys) — exists, 64,562 runs / 61 s clean (§E16) |
| security | hostile repo config tries `nebius.base_url = "https://evil"`, `sandbox.tier = "host"`, a literal `api_key` -> all rejected with distinct errors |
| fixtures | golden config files (global/repo/env) with provenance snapshots (`insta`); Windows path cases generated at test time |
| smoke | `load` on an empty machine (no `~/.polaris`) returns defaults in under 5 ms (target) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Config Layer Merge | function | `load` | `crates/polaris-config/src/merge.rs` | `later_layer_overrides_earlier`, `tables_merge_scalars_replace`, `provenance_reports_winning_layer`, `unknown_key_is_rejected` |
| Repo Layer Trust Filter | function | `filter_repo_layer` | `crates/polaris-config/src/trust.rs` | `repo_cannot_set_endpoint`, `repo_cannot_downgrade_sandbox_tier`, `repo_may_set_allowlisted_style_keys`, `trust_grant_unlocks_listed_key` |
| Secret Reference Parsing | enum | `SecretRef` | `crates/polaris-config/src/secret_ref.rs` | `parses_keyring_scheme`, `parses_env_scheme`, `rejects_bare_literal`, `rejects_empty_name` |
| Directory Discovery | function | `discover` / `read_layers` | `crates/polaris-config/src/sources.rs` (P1: discovery, file, env and session sources in one module) | 10 tests incl. `home_global_dir_is_not_a_repo_dir`, `windows_locked_file_is_retried`, `long_path_repo_dir_is_found` |
| Golden Provenance | function | `discover_from` / `read_layers` / `load` | `crates/polaris-config/src/merge.rs` | `golden_provenance_snapshot` (insta, reviewed) |

**Risks & open questions.**
- Repo-supplied config is attacker-controlled input (an untrusted clone could redirect the Bearer key to another host). The trust filter is a **design requirement of this plan**, not something the chatlog covered. LIKELY necessary; UNVERIFIED which key allow-list is right.
- `toml 1.1.8` / `etcetera 0.11.0` API specifics: API to confirm at implementation (including how `etcetera` resolves home on Windows vs `%USERPROFILE%`).
- Where `events.redb` and `invariants` live (global vs `.polaris/`) is stated differently in chatlog msgs 18 and 31; the config key `storage.level` must make this explicit (`$storage-level(global|isolated)`). Open: ADR.
- Default semaphore permits (Token Factory 4-8, web 8-12, docs 16, disk 64, S1 tensors 2-4, Contree 3-5) come from chatlog msg 17 and are **GUESSING** until measured on the GTX 1650 / 16 GB machine.

**Chatlog sources.** Msg 18 (`$ratelimit(rpm=,tpm=)`, `$storage-level(global|isolated)`, `~/.polaris` vs `.polaris/`), msg 17 (`$concurrency(web=,inference=,io=)` and default permit pools), msg 9 (budget -> `$budget`), msg 1 (directives are layered config). Corrected: D11 (directives are session-scoped and persisted for resume); secrets are references only.

---

### `polaris-log` — tracing facade with spdlog-style async sinks and a redaction layer

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-log` | L1 | lib | — | P1 |

**Purpose.** Installs the `tracing` subscriber and routes records to non-blocking sinks: rotating file, a TUI ring buffer and a telemetry ring. Every record passes the sdk `Redactor` before any sink sees it, so secrets/PII never reach disk or the TUI. The application thread must never wait on disk or terminal I/O.

**API sketch.**
```rust
pub struct LogConfig { pub dir: PathBuf, pub level: String, pub rotate_bytes: u64, pub keep_files: u16, pub ring_capacity: usize, pub json: bool }
pub struct LogGuard { /* flushes and joins the drain thread on Drop */ }
pub fn init<R: Redactor + 'static>(cfg: LogConfig, redactor: R) -> Result<LogGuard, LogError>;
pub struct RingSink { /* fixed-capacity, overwrite-oldest, drop counter */ }
impl RingSink {
    pub fn new(capacity: usize) -> Self;
    pub fn push(&self, rec: &LogRecordRef<'_>);                 // hot path: zero-alloc (log enqueue)
    pub fn snapshot(&self) -> Vec<LogLine>;                      // cold path, for the TUI log pane
    pub fn dropped(&self) -> u64;
}
pub struct RedactionLayer<R> { /* tracing_subscriber::Layer; formats into a reused buffer, redacts, enqueues */ }
pub enum LogError { AlreadyInitialised, Io(..), BadFilter(String), SinkInit(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `tracing = 0.1.44`, `tracing-subscriber = 0.3.23` (env-filter, json, registry, fmt, ansi, std), `spdlog-rs = 0.5.3`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: ring sink, redaction layer, file rotation, init/guard lifecycle.
- [ ] Implement `RingSink` as a fixed-capacity overwrite-oldest buffer with a drop counter; never blocks, never allocates in `push` after construction.
- [ ] Implement `RedactionLayer`: format the event into a reusable thread-local buffer, call `Redactor::redact_into`, then enqueue; confirm it adds no per-record allocation (counting allocator).
- [ ] Wire spdlog-rs-backed sinks (rotating file, async drain) behind the layer; **if the allocation test shows spdlog-rs allocates per record**, keep the zero-alloc enqueue in our ring and let spdlog-rs run on the drain thread only.
- [ ] Implement `init`/`LogGuard`: single-init guard, flush on drop, flush on panic hook and Ctrl+C path (coordinated with `kernel-reaper`).
- [ ] TUI mode: when the TUI owns the terminal, no sink writes to stdout/stderr; only the ring and files are active.
- [ ] File sink on Windows: daily + size rotation with retry when AV/indexers hold the file (`backon` is not in this crate's list; use a bounded manual retry or add the dependency via the toml owner).
- [ ] Add a `JSON` formatter option and an `EnvFilter` from config (`RUST_LOG`-style), read via `var_os`.

**Tests.**

| Category | Cases / properties |
|---|---|
| unit | ring overwrites oldest and counts drops; init twice returns `AlreadyInitialised`; guard drop flushes pending records |
| proptest | for any record sequence, ring contents equal the last `capacity` records in order; redaction never lengthens a secret-bearing span's visible text |
| smoke | init writes a rotated file in a temp dir on Windows; 3 rotations keep exactly `keep_files` files |
| load | 1M records from 8 threads: producers never block > 1 ms p99 (target); drop counter reported |
| stress | disk-full / read-only log dir: producers keep running, `LogError`/drop counter surfaces, no panic |
| security | seeded fake secrets (generated at test time) never appear in file, ring or JSON output; redactor failure fails closed (record dropped, counter incremented) |
| fuzz | `fuzz_log_redaction_layer` (arbitrary field text through the layer) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Ring Sink | struct | `RingSink` | `crates/polaris-log/src/ring.rs` | `push_beyond_capacity_overwrites_oldest`, `dropped_counter_increments`, `snapshot_preserves_order`, `push_does_not_allocate` |
| Redaction Layer | struct | `RedactionLayer` | `crates/polaris-log/src/redact.rs` | `secret_never_reaches_sink`, `redactor_error_drops_record`, `non_secret_text_unchanged`, `layer_reuses_buffer` |
| Log Init Lifecycle | function | `init` | `crates/polaris-log/src/init.rs` | `second_init_is_error`, `guard_drop_flushes`, `bad_filter_is_error`, `tui_mode_never_writes_stdout` |

**Risks & open questions.**
- `spdlog-rs` is a community crate, not an official port of the C++ library (plan correction, VERIFIED). The chatlog's "<150 ns per record" and "zero-alloc formatting" claims are **UNVERIFIED** for this crate; the plan treats them as hypotheses to measure.
- `tracing-spdlog`-style adapters: the exact `spdlog-rs` sink/async-pool API is to be confirmed at implementation. Fallback: the in-house ring plus a plain file writer thread.
- Redacting every record costs a DFA pass; budget (target, to be measured): < 1 µs per short record on the dev machine.
- `tracing-appender` was dropped on purpose (plan); do not reintroduce.

**Chatlog sources.** Msg 27 (spdlog-style async logging, lock-free ring, daily file / TUI / memory-ring sinks, tracing adapter). Corrected: "official port" -> community crate; `spdlog-rs 0.4` -> 0.5.3; performance numbers demoted to targets. Msg 31 (Token-weather HUD consumes the telemetry ring); plan (redaction via the sdk `Redactor` before any sink).

---

### `polaris-telemetry` — Usage, cost, latency and cache metrics fed by bus events

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-telemetry` | L1 | lib | — | P1 |

**Purpose.** Aggregates bus events into snapshots that back `/usage`, `/stats`, `/quota`, `/context-map` and the token-weather HUD: tokens per model, S1-vs-S2 time, cache hit rates per tier, sandbox load, latency histograms. Cost is computed only from a configured price table (Token Factory prices are not in the public catalog, VERIFIED in the plan). One owner task mutates aggregates; readers get immutable snapshots without locking the producers.

**API sketch.**
```rust
pub struct Telemetry { /* owns aggregates; single writer */ }
pub struct TelemetrySnapshot { pub models: Vec<ModelUsage>, pub s1: StageStats, pub s2: StageStats, pub cache: CacheStats, pub sandbox: SandboxLoad, pub cost: CostReport, pub at: jiff::Timestamp }
pub struct ModelUsage { pub model: ModelId, pub prompt: TokenCount, pub completion: TokenCount, pub cached: Option<TokenCount>, pub requests: u64 }
pub struct StageStats { pub count: u64, pub p50_us: u64, pub p99_us: u64, pub total_us: u64 }
pub struct PriceTable { /* ModelId -> (prompt, completion, cached) in MicroUsd per 1M tokens; loaded from config */ }
pub struct CostReport { pub total: MicroUsd, pub per_turn_avg: MicroUsd, pub priced: bool }   // priced=false when no table entry
pub enum BudgetStatus { Ok, Warn { used_pct: u8 }, Exceeded }                                 // warn-only; enforcement is the kernel's job
impl Telemetry {
    pub fn record(&mut self, e: &Event);                         // fold one bus event into aggregates
    pub fn snapshot(&self) -> Arc<TelemetrySnapshot>;            // lock-free read (snapshot swap)
    pub fn budget(&self, limit: &Budget) -> BudgetStatus;
}
pub struct HudView { pub context_load_pct: u8, pub cache_hit_pct: Option<u8>, pub burn_tokens_per_min: u32, pub s1_avg_us: u64, pub s2_ttft_ms: Option<u32> }
pub enum TelemetryError { Histogram(String), PriceMissing(ModelId), Overflow }
```

**Dependencies.** Internal: `polaris-sdk`. External: `hdrhistogram = 7.6.0` (group "test" in `workspace.toml` but listed here as runtime; see risks), `serde = 1.0.229` (derive). `arc-swap 1.9.3` is named in the crate's responsibilities but missing from its planned `ext` list.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: event folding, price-table cost, budget status, snapshot publication, HUD view.
- [ ] Define `ModelUsage`/`StageStats`/`CacheStats` and make all counters saturating or checked (no overflow panic).
- [ ] Implement `record(&Event)` as an exhaustive `match` over `Event` (no wildcard arm) so a new event variant forces a telemetry decision.
- [ ] Implement `PriceTable` loading from config and integer `MicroUsd` arithmetic; unknown model -> `priced = false`, never a made-up price.
- [ ] Implement latency histograms (hdrhistogram) for S1 and S2 stages plus TTFT; record with saturating semantics.
- [ ] Implement snapshot publication: a single consumer task publishes `Arc<TelemetrySnapshot>` every N ms; readers never block the bus.
- [ ] Implement `BudgetStatus` for `$budget(usd/tokens)` with warn thresholds (default 80% and 100%, configurable).
- [ ] Implement `HudView`: cache hit % is `None` unless the provider reports cached tokens (spike S0-5); otherwise show a local prefix-stability metric.
- [ ] Add `serde` serialisation of snapshots for `polaris --dump` and the Leptos/Slint UIs.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | usage sums per model; unknown-model cost is `priced=false`; budget crosses Warn then Exceeded at the configured percentages |
| proptest | folding any event sequence in two chunk orders (same total) yields identical totals; counters never decrease; cost equals sum of per-model costs |
| load | 1M events folded in under 1 s on the dev machine (target); snapshot read from 4 threads never blocks the writer |
| fixtures | recorded synthetic event streams with expected snapshot (`insta`), labelled synthetic until re-recorded live |
| stress | counter saturation at `u64::MAX`; histogram value beyond range is clamped, not an error loop |
| smoke | `/usage` view renders from an empty snapshot without panic |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Usage Aggregation | method | `Telemetry::record` | `crates/polaris-telemetry/src/aggregate.rs` | `model_request_completed_adds_tokens`, `cached_tokens_tracked_only_when_reported`, `s1_and_s2_time_split`, `counters_saturate_not_wrap` |
| Cost Calculation | function | `cost_of` | `crates/polaris-telemetry/src/cost.rs` | `priced_model_cost_matches_table`, `missing_price_marks_unpriced`, `zero_tokens_cost_zero`, `micro_usd_overflow_is_error` |
| Budget Monitor | method | `Telemetry::budget` | `crates/polaris-telemetry/src/budget.rs` | `below_threshold_is_ok`, `at_80_percent_warns`, `at_100_percent_exceeded`, `token_and_usd_limits_checked_independently` |

**Risks & open questions.**
- `hdrhistogram` is grouped as "test" and annotated with a v2 comment in `workspace.toml`, yet this crate needs it at runtime. **VERIFIED** inconsistency in the toml; owner should reclassify the dependency group.
- `arc-swap` is required by the stated design but absent from `ext`. **VERIFIED**; add it to this crate's list (version `1.9.3` is already pinned in the `[[ext]]` table).
- Provider cached-token reporting and rate-limit headers are **UNVERIFIED** until spike S0-5 (needs a key; BLOCKED-ON-ACCESS). The chatlog HUD sample (94.2% KV hit, $0.04/turn) is illustrative, not measured.
- hdrhistogram API (`record`, `saturating_record`, bounds constructor) to confirm at implementation.

**Chatlog sources.** Msg 31 (Token Weather and KV-cache HUD; keep-warm ping only if S0-5 allows), msg 9 (FinOps and token budgeting), msg 1 (`/usage`, `/stats`), msg 19 (`/cache stats`, `/quota`), msg 23 (`/context-map`). Corrected: Nebius pricing is configured, not assumed; prefix-cache reuse numbers (80%, TTFT 800 ms -> <100 ms) are unverified claims.

---

### `polaris-ratelimit` — GCRA rate limiting: RPM plus token-weighted TPM, priority shedding, Retry-After, 15-minute window tracking

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ratelimit` | L1 | lib | — | P3 |

**Purpose.** Prevents HTTP 429 from Token Factory and search providers by reserving request and token budget *before* a call and reconciling with actual usage afterwards. It replaces the chatlog's fixed-window counter (msg 19: a `Mutex<Instant>` plus per-minute atomics, which allows a 2x burst at window boundaries and is not GCRA) with a real GCRA for RPM and a token-weighted GCRA for TPM. Under pressure it sheds low-priority work first and honours `Retry-After`.

**API sketch.**
```rust
pub enum Priority { Background, SubAgent, Verifier, InteractiveUser }          // ascending; shed from the bottom
pub struct Quota { pub rpm: NonZeroU32, pub tpm: NonZeroU32, pub burst_requests: u32, pub burst_tokens: u32 }
pub struct DualLimiter<C: Clock> { /* RPM cell + TPM cell + shed thresholds + window tracker */ }
pub struct Estimate { pub prompt: TokenCount, pub max_output: TokenCount }
pub struct Permit { /* RAII: holds the reservation; drop without commit refunds it */ }
impl<C: Clock> DualLimiter<C> {
    pub fn new(q: Quota, clock: C, shed: ShedPolicy) -> Result<Self, RateLimitError>;
    pub fn try_acquire(&self, est: Estimate, p: Priority) -> Result<Permit, Denied>;       // hot path: zero-alloc (GCRA check)
    pub async fn acquire(&self, est: Estimate, p: Priority) -> Result<Permit, RateLimitError>; // cancel-safe; may allocate (timer)
    pub fn on_response(&self, h: &RateLimitHeaders, status: u16);                           // parse hints, apply Retry-After penalty
    pub fn utilization(&self) -> Utilization;                                               // feeds /quota and telemetry
}
impl Permit { pub fn commit(self, actual: TokenCount); }                                    // reconcile reserved vs actual
pub struct Denied { pub retry_after: core::time::Duration, pub reason: DenyReason }
pub enum DenyReason { RpmExhausted, TpmExhausted, Shed(Priority), PenaltyActive }
pub struct ShedPolicy { pub background_above_pct: u8, pub subagent_above_pct: u8, pub verifier_above_pct: u8 } // interactive is never shed
pub struct WindowTracker { /* rolling 15-minute utilization for soft-cap reporting */ }
pub fn jittered_backoff(attempt: u32, retry_after: Option<core::time::Duration>) -> core::time::Duration;
pub enum RateLimitError { InvalidQuota, Closed, Cancelled }
```

**Dependencies.** Internal: `polaris-sdk`. External: `governor = 0.10.4`, `backon = 1.6.0`, `tokio = 1.54.0` (member opts into `time`, `sync`, `macros`).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: GCRA check, token reservation/reconcile, priority shedding, Retry-After penalty, window tracking.
- [ ] Decide governor vs a hand-rolled single-atomic GCRA: governor offers `check_n`-style cells (API to confirm) but I have not verified a *refund* operation, and reconcile-after-response needs one. Resolve with a criterion bench plus the spike `polaris-spike-ratelimit`.
- [ ] Define the `Clock` abstraction so tests use a fake clock (no sleeping tests).
- [ ] Implement RPM GCRA (one cell per request) and TPM GCRA (cell cost = tokens); both lock-free on the fast path.
- [ ] Implement `Permit` RAII: drop without `commit` refunds the whole reservation; `commit(actual)` refunds the difference or charges the overshoot; never panics on underflow.
- [ ] Implement priority shedding (default: Background above 70%, SubAgent above 85%, Verifier above 95%; Interactive never) -- thresholds are **GUESSING**, configurable via `$ratelimit`.
- [ ] Implement `on_response`: parse `Retry-After` (seconds or HTTP-date) and, if present, `x-ratelimit-*` hints; apply a penalty window that all priorities respect. Header names are UNVERIFIED until S0-5.
- [ ] Implement `acquire` as cancel-safe: dropping the future while waiting must not leak a reservation (test with `tokio::select!` cancellation).
- [ ] Implement `WindowTracker` for the 15-minute soft-cap window and expose `/quota` data.
- [ ] Implement `jittered_backoff` using backon (API to confirm) with the server hint as a floor.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | N requests inside burst pass, N+1 denied with correct `retry_after`; commit(actual < reserved) refunds; penalty blocks all priorities |
| proptest | over any request sequence, admitted tokens in any 60 s window never exceed `tpm + burst`; refund never makes the cell exceed its burst; shedding is monotone in priority |
| loom | not applicable if the limiter uses only atomics with a CAS loop; add loom on the CAS loop if hand-rolled |
| kani | arithmetic of cell advance: no overflow for `tpm` up to `u32::MAX` and cost up to `burst` |
| load | `try_acquire` > 5M ops/s single thread, > 20M ops/s across 8 threads (targets); latency via hdrhistogram |
| spike | `polaris-spike-ratelimit`: 429 storm (server returns 429 for 30 s) with 50 sub-agent bursts; zero requests sent during the penalty window |
| stress | 10k cancelled `acquire` futures leave utilization unchanged |
| scalability | 1 -> 32 concurrent frames, fairness: Interactive p99 wait < 100 ms while Background saturates (target) |
| fixtures | recorded **synthetic** 429/Retry-After responses; labelled synthetic until live S0-5 |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| GCRA Check | method | `DualLimiter::try_acquire` | `crates/polaris-ratelimit/src/gcra.rs` | `burst_then_deny_with_retry_after`, `no_boundary_double_burst`, `tpm_cost_weights_cell`, `try_acquire_does_not_allocate`, `fake_clock_advance_restores_capacity` |
| Reserve And Reconcile | method | `Permit::commit` | `crates/polaris-ratelimit/src/permit.rs` | `commit_less_than_reserved_refunds`, `commit_more_than_reserved_charges`, `drop_without_commit_refunds_all`, `refund_never_exceeds_burst` |
| Priority Shedding | function | `shed_decision` | `crates/polaris-ratelimit/src/shed.rs` | `background_shed_first`, `interactive_never_shed`, `thresholds_monotone`, `zero_utilization_admits_all` |
| Retry-After Penalty | method | `DualLimiter::on_response` | `crates/polaris-ratelimit/src/penalty.rs` | `retry_after_seconds_applies_penalty`, `retry_after_http_date_parsed`, `garbage_header_ignored`, `penalty_expires_with_clock` |

**Risks & open questions.**
- Chatlog presents the limiter as GCRA but the code is a fixed window (**VERIFIED** by reading msg 19). Real GCRA is a plan correction.
- Token Factory limits are soft caps in 15-minute windows (**VERIFIED**, plan); whether RPM/TPM headers and `Retry-After` exist is **UNVERIFIED** (S0-5, BLOCKED-ON-ACCESS). Until then the limiter runs on configured `$ratelimit(rpm,tpm)` plus local 429 learning.
- Token estimates: S1 cannot count tokens (msg 19 claims it can); use a conservative bytes/4 heuristic plus reconcile (**GUESSING** accuracy, measure per model).
- governor's exact API (`check_n`, quota types, clock trait) and whether it can model refunds: API to confirm at implementation.
- Background-shedding thresholds are tuning guesses; calibrate in `polaris-spike-ratelimit`.

**Chatlog sources.** Msg 19 (dual RPM/TPM metering, priority order InteractiveUser > Verifier > SubAgent > Background, shed below 15% TPM headroom, jittered backoff with Retry-After, `/quota`, `$ratelimit`), msg 17 (429 prevention rationale, semaphore pools handled in kernel). Corrected: fixed window -> GCRA; header names and token estimation by S1 flagged unverified; 15-minute soft-cap window added from the verified Token Factory facts.

---

### `polaris-code-intel` — tree-sitter parsing, PageRank repo map, skeletons, AST diff/merge, code metrics

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-code-intel` | L1 | lib | — | P1 |

**Purpose.** Pure code-understanding engine over bytes: parse supported languages (Rust, Python, C++, CUDA, TypeScript), emit signature-only skeletons for context folding, rank symbols with personalised PageRank (Aider-style) for the repo map, and compute the mutation metrics (line churn, cyclomatic delta, tree-edit distance) consumed by the kernel's `HybridMutationVerifier`. It does no file discovery or I/O: callers pass `&[u8]`, which keeps it deterministic and testable.

**API sketch.**
```rust
pub enum Lang { Rust, Python, Cpp, Cuda, TypeScript }
pub fn detect_lang(path: &Path, head: &[u8]) -> Option<Lang>;
pub struct Parsed { /* tree + source handle; incremental reparse supported */ }
pub struct ParserPool { /* one tree-sitter Parser per thread; Parser is not Sync */ }
impl ParserPool { pub fn parse(&self, lang: Lang, src: &[u8], prev: Option<&Parsed>) -> Result<Parsed, CodeIntelError>; }

pub struct Skeleton { pub items: Vec<SkeletonItem> }               // signatures only, bodies elided
pub struct SkeletonItem { pub kind: SymbolKind, pub name: String, pub signature: String, pub span: ByteSpan }
pub fn skeleton(p: &Parsed) -> Result<Skeleton, CodeIntelError>;

pub struct SymbolGraph { /* defs, refs, edges per file; deterministic ordering */ }
pub struct RepoMap { pub entries: Vec<RankedFile>, pub token_estimate: TokenCount, pub version: RepoMapVersion }
pub struct RepoMapBudget { pub max_tokens: TokenCount, pub focus_files: Vec<PathBuf> }
pub fn repo_map(g: &SymbolGraph, b: &RepoMapBudget) -> Result<RepoMap, CodeIntelError>;   // byte-identical for identical input

pub struct MutationMetrics { pub churn_lines: u32, pub cyclomatic_delta: i32, pub tree_edit_distance: u32, pub changed_symbols: Vec<SymbolRef> }
pub fn metrics(old: &Parsed, new: &Parsed) -> Result<MutationMetrics, CodeIntelError>;
pub fn subtree_fingerprints(p: &Parsed) -> Vec<(ByteSpan, FastFingerprint)>;                // xxh3 per function/struct node
pub enum MergeOutcome { Merged(Vec<u8>), Conflict { symbols: Vec<SymbolRef> } }
pub fn merge3(base: &Parsed, ours: &Parsed, theirs: &Parsed) -> Result<MergeOutcome, CodeIntelError>; // symbol-granular; never overwrites on conflict
pub enum CodeIntelError { UnsupportedLanguage, ParseTimeout, ParseFailed, TooLarge { limit: usize }, GrammarVersionMismatch(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `tree-sitter = 0.27.1`, `tree-sitter-rust = 0.24.2`, `tree-sitter-python = 0.25.0`, `tree-sitter-cpp = 0.23.4`, `tree-sitter-cuda = 0.21.2`, `tree-sitter-typescript = 0.23.2`, `ast-grep-core = 0.51.0` (requires tree-sitter ^0.27, VERIFIED in the plan), `imara-diff = 0.2.0`, `xxhash-rust = 0.8.19` (xxh3).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: parser pool, skeleton extraction, repo-map ranking, mutation metrics, symbol-granular merge.
- [ ] Spike inside the crate: confirm every grammar crate loads against `tree-sitter 0.27.1` (language-ABI compatibility) on `x86_64-pc-windows-msvc`; record results in PHASES_STATUS.
- [ ] Implement `ParserPool` (per-thread parsers, parse size cap, timeout/progress cancellation; API to confirm in 0.27) and a language detector from extension plus shebang.
- [ ] Implement `skeleton` per language with an **iterative** `TreeCursor` walk (no recursion on untrusted depth).
- [ ] Implement the symbol graph and personalised PageRank (damping 0.85, fixed iteration cap, deterministic tie-break by path then name); output must be byte-stable so the repo map can sit in the cached prompt prefix.
- [ ] Implement token-budget fitting for the repo map with a documented bytes/4 estimate; the real tokenizer is unavailable, so mark the estimate as approximate in the output.
- [ ] Implement churn (imara-diff line diff), cyclomatic delta (per-language decision-node tables), and tree-edit distance with a node cap and an approximate fallback above it.
- [ ] Implement xxh3 subtree fingerprints and use them to skip unchanged functions on re-analysis; never use them as a security identity.
- [ ] Implement `merge3` at function granularity via `ast-grep-core` positions (API to confirm); report conflicts instead of writing.
- [ ] Windows: keep spans as byte offsets of the original text (never rewrite EOLs), handle UTF-8 BOM, test paths with spaces/non-ASCII; add the WSL2 Linux check leg.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | skeleton of a known Rust/Python/C++/CUDA/TS sample equals expected signatures; cyclomatic of `if`/`match`/`&&` fixtures; churn of known diffs |
| proptest | `metrics(x, x)` is all zeros; churn is symmetric; PageRank scores sum to ~1 and ranking is deterministic across runs and thread counts |
| fuzz | `fuzz_code_intel_parse` (per language, arbitrary bytes), `fuzz_code_intel_skeleton`, `fuzz_code_intel_merge3` |
| kani / miri | Kani: bounded harness on token-budget fitting (no overflow). Miri: pure Rust parts only; tree-sitter is C FFI and not Miri-runnable |
| scalability | 1k / 10k / 100k-file repos: full repo-map under 2 s / 15 s / 120 s (targets) on the Ryzen 7 2700, RSS under 2 GB |
| load | skeleton throughput > 20 MB/s of source single-threaded (target) |
| stress | 50 MB single file, 10k-deep nested expressions, binary garbage: bounded time and memory, `TooLarge`/`ParseTimeout` returned |
| fixtures | per-language samples with provenance and licence; hostile/degenerate sources generated at test time, not committed |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Skeleton Extraction | function | `skeleton` | `crates/polaris-code-intel/src/skeleton.rs` | `rust_fn_signature_kept_body_elided`, `python_class_methods_listed`, `cuda_kernel_qualifiers_preserved`, `syntax_error_file_still_yields_partial_skeleton` |
| Repo Map Ranking | function | `repo_map` | `crates/polaris-code-intel/src/repomap.rs` | `output_is_byte_identical_across_runs`, `focus_file_boosts_rank`, `budget_is_respected`, `empty_graph_yields_empty_map` |
| Mutation Metrics | function | `metrics` | `crates/polaris-code-intel/src/metrics.rs` | `identical_trees_zero_metrics`, `added_branch_raises_cyclomatic`, `churn_counts_added_plus_deleted`, `ted_fallback_above_node_cap` |
| Symbol Merge | function | `merge3` | `crates/polaris-code-intel/src/merge.rs` | `orthogonal_functions_merge_cleanly`, `same_function_edited_conflicts`, `conflict_never_returns_merged_bytes`, `eol_style_preserved` |

**Risks & open questions.**
- tree-sitter and grammars are C code; a segfault is not catchable by `catch_unwind` (plan correction, VERIFIED). code-intel runs in-process at P1; the risk of a malicious repo crashing the kernel is **LIKELY low but real**. Mitigation: fuzzing plus size/depth caps; fallback: move parsing into a worker mode if fuzzing finds crashes.
- `ast-grep-core 0.51` and the five grammar crates must agree on the tree-sitter language ABI; the plan verified only that ast-grep-core needs tree-sitter ^0.27. Grammar load compatibility is **UNVERIFIED** until the in-crate spike.
- Exact tree-sitter 0.27 API (parse cancellation, timeouts, `TreeCursor`) and the ast-grep-core pattern API: API to confirm at implementation.
- Tree-edit distance (Zhang-Shasha style) is super-quadratic; the node cap and approximate-fallback thresholds are **GUESSING**.
- The chatlog's savings claims (85-90% tokens, "10x fewer tokens") are **UNVERIFIED**; measure on real repos.

**Chatlog sources.** Msg 13 (churn, cyclomatic delta, tree-edit distance, limits as Datalog facts), msg 15 (AST subtree fingerprinting with xxh3), msg 23 (tree-sitter skeleton, repo map with PageRank importance, stable prefix), msg 3 (file triage with tree-sitter candidates), msg 31 (AST blast radius via dependency graph). Corrected: hard-coded `churn > 150` ceiling becomes a token-fact limit; grammar versions re-pinned; `todo!()` skeletonizer replaced by a real iterative walker.

---

### `polaris-storage` — Facade re-exporting the storage children (cas, eventlog, registry, cache, vector, sync)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage` | L1 | facade | — | P1 |

**Purpose.** A pure facade: it re-exports the six storage children and owns the documentation of the on-disk layout (`~/.polaris` global, `.polaris/` per repo). It contains no logic; children never depend on it.
**Re-exports.** `pub use polaris_storage_cas as cas; pub use polaris_storage_eventlog as eventlog; pub use polaris_storage_registry as registry; pub use polaris_storage_cache as cache; pub use polaris_storage_vector as vector; pub use polaris_storage_sync as sync;`
**Dependencies.** Internal: the six storage children. External: none.
**TODO.**
- [ ] Document the layout table: `registry.db` (SQLite FTS5 trigram), `events.redb` (events, locks, L2 cache, invariants, `RULES.md` mirror), `cas/<ab>/<cd>/<blake3>` (zstd), `vectors/` (LanceDB), L1 in RAM (`quick_cache`), `$storage-level(global|isolated)` scoping.
- [ ] Resolve and document where `events.redb` and invariants live (global vs repo): chatlog msgs 18 and 31 disagree (ADR).
- [ ] Add a facade smoke test that every re-export resolves (`cargo check` plus a doc-test per alias).

---

### `polaris-storage-cas` — Content-addressable blob store: BLAKE3 ids, zstd compression, xxh3 dirty-checks, short `§ref` ids

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/cas` | L1 | lib | `polaris-storage` | P1 |

**Purpose.** Immutable blob storage under `~/.polaris/cas/<ab>/<cd>/<blake3-hex>`: file versions, diffs, tool outputs, logs. It supplies the byte-exact backing for `§ref:<12hex>` folding (`polaris_expand` returns the original bytes) and the xxh3 fast fingerprints used for dirty-checks. Writes are atomic (temp + rename) and survive Windows AV locks. Blobs are zstd-compressed and verified by re-hashing.

**API sketch.**
```rust
pub struct Cas { /* root dir, compression level, size caps */ }
pub struct CasConfig { pub root: PathBuf, pub zstd_level: i32, pub min_compress_bytes: usize, pub max_blob_bytes: u64, pub verify_on_get: bool }
impl Cas {
    pub fn open(cfg: CasConfig) -> Result<Self, CasError>;
    pub fn put(&self, bytes: &[u8]) -> Result<Cid, CasError>;                   // idempotent; existing blob is a success
    pub fn put_file(&self, path: &Path) -> Result<Cid, CasError>;               // streaming hash; mmap only above a threshold
    pub fn get(&self, cid: &Cid) -> Result<Bytes, CasError>;                    // decompress, bound output size, optional re-hash
    pub fn contains(&self, cid: &Cid) -> Result<bool, CasError>;
    pub fn short_ref(&self, cid: &Cid) -> Result<ShortRef, CasError>;           // hot path: zero-alloc (§ref hash/fold)
    pub fn resolve(&self, r: &ShortRef) -> Result<Cid, CasError>;               // lists `<ab>/<cd>/` by prefix; Ambiguous on 2 hits
    pub fn gc(&self, live: impl Iterator<Item = Cid>, policy: GcPolicy) -> Result<GcReport, CasError>; // caller supplies live set
}
pub fn fast_fingerprint(bytes: &[u8]) -> FastFingerprint;                       // hot path: zero-alloc (xxh3-64)
pub fn content_id(bytes: &[u8]) -> Cid;                                         // hot path: zero-alloc (BLAKE3, in-memory)
pub fn fold_ref(cid: &Cid, out: &mut [u8; 18]) -> usize;                        // writes "§ref:" + 12 hex into a caller buffer
pub enum CasError { Io(..), NotFound(Cid), Corrupt { cid: Cid }, Ambiguous(ShortRef), ShortRefCollision(ShortRef), TooLarge { limit: u64 }, Decompress(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `blake3 = 1.8.7`, `xxhash-rust = 0.8.19` (xxh3), `zstd = 0.14.3`, `tempfile = 3.27.0`, `bytes = 1.12.1`; dev: `proptest`, `dhat = 0.3.3`. **P1 decision:** the AV-lock retry is a small bounded loop in `atomic.rs` (`RetryPolicy`, tested), not `backon`.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/cas_{blob_store_round_trip,atomic_write_with_retry,short_reference_folding,decompression_bound,zero_alloc_hot_path}.json`.
- [x] Define the on-disk blob format: 1 format-version byte + 1 codec byte + payload (`raw` or `zstd`); skip compression below `min_compress_bytes` or when it does not shrink.
- [x] Implement `put`: hash first, return early if the path exists, otherwise write to a temp file in the same directory, fsync and rename (replace); concurrent writers of the same blob all succeed (tested with 8 threads).
- [x] Implement bounded retry on Windows sharing violations / AV scans (OS errors 5/32/33; exponential 10 ms → 500 ms, 8 attempts ≈ 1.8 s) and surface `CasError::Io` after the budget — VERIFIED with a real share-mode-0 handle (a single attempt fails, the retry policy succeeds).
- [x] Implement `get` with a hard cap on decompressed size (read the zstd frame's declared content size, reject over-limit or undeclared size before allocating) and re-hash verification (`verify_on_get`, default on).
- [x] Implement `resolve`/`fold_ref` and `short_ref_checked` (returns `ShortRefCollision` when another blob shares the 12-hex prefix, so the fold layer falls back to the full 64-hex ref). P1 change: the check is a separate call, not a `put` error — a `put` that stored the bytes must not report failure.
- [x] Implement `put_file` with streaming BLAKE3 (`update_reader`) and no mmap (mmap risk avoided entirely); an existing blob is never re-read.
- [x] Implement `gc` driven by a live-Cid iterator, with a grace period and a dry-run mode; used by `/cache purge`.
- [x] Windows: lower-case hex only, no partial blob on failure (temp file removed; tested). Root-path length and `dunce` canonicalisation are the caller's (config) concern — open item for `polaris-config`. Disk-full: not simulated (needs a full volume).
- [x] Counting-allocator tests (dhat testing mode, ADR-003) for `ShortRef::of`, `fold_ref`, `fast_fingerprint`, `content_id`: 0 blocks.
- [x] Fuzz target `cas_blob_decode` (decode bound + encode→decode roundtrip; 3,962,414 runs / 61 s, no crash).
- [ ] Kani harness for `fold_ref` (first attempt grew CBMC past 6 GB and was removed; needs a smaller formulation), `cas_shortref_parse` fuzz target, load/stress tests (10k blobs, disk full). Miri: not applicable to the zstd FFI paths.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | put/get byte-exact for empty, 1 B, 1 MiB, incompressible blobs; second `put` of same bytes is a no-op; `get` of missing -> `NotFound` |
| proptest | for any bytes: `get(put(b)) == b`; `short_ref(cid)` is a prefix of `cid` hex; `resolve(short_ref(cid)) == cid` when unique; `content_id` equals `blake3::hash` |
| fuzz | `fuzz_cas_blob_decode` (arbitrary on-disk blob bytes must give `Err`, never panic or over-allocate), `fuzz_cas_shortref_parse` |
| kani | `fold_ref` writes exactly 18 bytes for every `Cid`; no out-of-bounds |
| miri | pure helpers (`fold_ref`, hex); FFI-backed paths (zstd) excluded |
| load | 10k small blobs: p99 put < 5 ms (target; AV may inflate); 1 GiB streaming hash >= 1 GB/s with rayon (target, to be measured on this CPU) |
| stress | disk full mid-write leaves no partial file; 8 threads putting the same blob; AV-lock simulation (held handle) retries then errors |
| security | zstd bomb (small frame, declared 10 GiB) rejected before allocation; corrupted blob detected by re-hash; path traversal impossible (only hex names) |
| fixtures | blob corpora generated at test time; golden blob bytes for format v1 |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Blob Store Round Trip | struct | `Cas` | `crates/polaris-storage/crates/cas/src/store.rs` | `put_get_roundtrip_byte_exact`, `put_is_idempotent`, `get_missing_is_not_found`, `incompressible_stored_raw`, `corrupt_blob_detected_on_verify` |
| Atomic Write With Retry | function | `write_atomic` | `crates/polaris-storage/crates/cas/src/atomic.rs` | `rename_race_is_success`, `retry_on_sharing_violation`, `gives_up_after_budget`, `no_partial_file_on_failure` |
| Short Reference Folding | function | `short_ref` / `resolve` | `crates/polaris-storage/crates/cas/src/shortref.rs` | `short_ref_is_12_hex_prefix`, `resolve_unique_ok`, `resolve_ambiguous_errors`, `fold_ref_does_not_allocate`, `collision_returns_error` |
| Decompression Bound | function | `read_blob` | `crates/polaris-storage/crates/cas/src/codec.rs` | `declared_size_over_limit_rejected`, `truncated_frame_is_error`, `unknown_codec_byte_is_error`, `empty_payload_ok` |

**Risks & open questions.**
- Chatlog stores blobs in `redb` (msg 15, 18); the toml/plan store them as files under `cas/` (blake3 + zstd). The file layout is the adopted one: **VERIFIED** against `workspace.toml`.
- A 12-hex (48-bit) short ref has a birthday bound around 2^24 (~16.8 M) refs for a 50% collision chance (arithmetic, VERIFIED); the collision check plus full-hash fallback makes it safe, but the fold layer must handle `ShortRefCollision`.
- mmap on Windows: truncating a mapped file from another process can raise an in-page access violation that cannot be caught (**LIKELY**). Use buffered streaming by default and mmap only for large files that are quiescent; blake3 `update_mmap` availability: API to confirm.
- xxh3 is non-cryptographic: it must never key a *safety verdict* cache (an attacker could craft collisions). Use `Cid` there (**LIKELY**; xxh3 is not designed for adversarial collision resistance).
- Chatlog throughput claims (xxh3 35-50 GB/s, BLAKE3 6-7 GB/s, lookup <300 ns/80 ns) are **UNVERIFIED** on a Ryzen 7 2700 (AVX2, no AVX-512); treat as targets.
- `backon` missing from `ext`; either add it via the toml owner or implement a bounded manual retry.

**Chatlog sources.** Msg 15 (two-tier xxh3 + BLAKE3, CAS, dedup, `FastFingerprint`/`CryptoContentId`, `hash_file_mmap`), msg 23 (`§ref:` byte-exact folding with `polaris_expand`), msg 18 (blob store `~/.polaris/cas/blobs`). Corrected: `§ref:CID§` -> `§ref:<12hex>`; `dashmap` L1 replaced by `quick_cache` (cache crate); redb blob table replaced by files; `.expect()` on write failure removed.

---

### `polaris-storage-eventlog` — Append-only event journal on redb (events.redb): commands, decisions, tool calls, responses; replay and checkpoints

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/eventlog` | L1 | lib | `polaris-storage` | P1 |

**Purpose.** Implements the sdk `EventStore` trait over an embedded `redb` database. Besides the ordered event journal it persists file locks, the L2 exact-cache entries, project invariants (with the `RULES.md` mirror) and research checkpoints in separate tables. `/session replay [step]` replays events up to a `Seq` and pairs them with CAS snapshots (time-travel debugging). sled and CockroachDB were rejected (unmaintained / no embedded mode, plan).

**API sketch.**
```rust
pub struct EventLog { /* redb Database handle; single-writer discipline */ }
pub struct EventLogConfig { pub path: PathBuf, pub batch_commit_max: usize, pub durability: Durability }
pub enum Durability { Immediate, Eventual }                                         // maps to redb durability (API to confirm)
impl EventLog {
    pub fn open(cfg: EventLogConfig) -> Result<Self, EventLogError>;
    pub fn append(&self, e: &EventRecordDraft) -> Result<Seq, EventLogError>;       // implements EventStore::append
    pub fn append_batch(&self, es: &[EventRecordDraft]) -> Result<Seq, EventLogError>; // one transaction
    pub fn read_range(&self, from: Seq, to: Seq) -> Result<Vec<EventRecord>, EventLogError>;
    pub fn replay(&self, until: Seq) -> Result<ReplayIter<'_>, EventLogError>;       // lazy iterator of Result<EventRecord, _>
    pub fn head(&self) -> Result<Seq, EventLogError>;
    pub fn checkpoint(&self, at: Seq, snapshot: Cid) -> Result<(), EventLogError>;
    pub fn latest_checkpoint(&self, at_or_before: Seq) -> Result<Option<Checkpoint>, EventLogError>;
}
// auxiliary tables (same database file)
pub struct LockTable;    // path/AST-scope lease records, crash recovery on open
pub struct ExactCache;   // [u8;32] key -> value bytes; backs the L2 tier of storage-cache
pub struct Invariants;   // project rules (+ RULES.md mirror content hash)
pub struct ResearchCkpt; // /research resume state
pub enum EventLogError { Open(String), Locked, Codec(String), SchemaTooNew { found: u16, supported: u16 }, Gap { expected: Seq, found: Seq }, Io(..) }
```

**Dependencies.** Internal: `polaris-sdk` (P1: the `polaris-storage-cas` edge was dropped — checkpoints store the sdk `Cid` only). External: `redb = 4.4.0`, `serde = 1.0.229`, `serde_json = 1.0.154`. **P1 decision:** postcard was replaced by JSON because postcard cannot decode sdk's internally tagged `Event` (VERIFIED: `Err(WontImplement)`, probe in `@prime/tmp/p1/postcard-probe`); a binary codec needs an externally tagged mirror type — revisit if log size matters.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/eventlog_{event_append_and_read,replay_to_step,lock_table_recovery,exact_cache_store,event_codec}.json` (schema-version gate is covered in `event_codec` and `replay_to_step`).
- [x] Table layout (P1 subset): `events` (u64 -> `[schema u16 LE][JSON]`), `meta` (layout version, next lease), `checkpoints`, `locks` (JSON `LockRecord`), `l2_cache` (`[scope tag][id][payload]`). `INVARIANTS`, `RULES_MIRROR`, `RESEARCH_CKPT` arrive with their owning features (P4/P7).
- [x] Event envelope: sdk `EventRecord` (`schema: u16` v2, backwards-readable v1, `at`, `session`, `event`); the codec rejects `schema > SUPPORTED_SCHEMA` **before** parsing the body and rejects a prefix/envelope mismatch. Schema 2 adds immutable session configuration, pause and resume; old envelopes retain their version on encode/decode (§E24).
- [x] `append`/`append_batch` assign gap-free `Seq` inside one write transaction (`batch_is_atomic`). Group commit across callers — open (needs the async owner thread below).
- [x] `read_range`/`replay` are redb read transactions (MVCC, do not block the writer); newer schema rejected. `replay` returns a `Vec` — a lazy iterator is open.
- [x] Checkpoints store `(Seq, Cid)` (`checkpoint_beyond_head_is_rejected`); `replay_from_checkpoint_matches_full_replay`.
- [x] Lock-table recovery: on open, leases held by a different (dead) owner pid are listed by `stale_locks()` for the Reaper, never silently dropped; locks survive reopen.
- [ ] Async owner thread plus channel for tokio callers — open (callers use `spawn_blocking` meanwhile; the registry crate shows the owner-thread pattern).
- [x] `EventStore` and `ExactCacheStore` (sdk traits) implemented by `EventLog`; oversized cache values rejected (`max_cache_value_bytes`).
- [x] Windows: a second open maps redb's `DatabaseAlreadyOpen` to `EventLogError::Locked` (tested); path with spaces + non-ASCII tested. OneDrive-synced folders: not tested. Power-loss flush semantics: documented (`Durability`), not claimed.
- [x] Fuzz target `eventlog_decode` (arbitrary stored bytes: no panic; decode→encode→decode stable): 309,107 runs / 62 s clean (§E15). Miri: not applicable (redb file I/O). Load/stress: open.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | seq starts at 1 and is gap-free; read_range bounds; replay stops at `until`; reopening preserves head |
| proptest | any append sequence reads back identical and ordered; batch append equals sequential append; postcard round-trip of every `Event` variant |
| fuzz | `fuzz_eventlog_record_decode` (arbitrary postcard bytes must give `Err`), `fuzz_eventlog_open_corrupt_file` (mutated database file) |
| loom | not applicable (redb handles concurrency); loom on our owner-thread channel if hand-rolled |
| load | append p99 < 2 ms with `Eventual` durability and < 20 ms with `Immediate` on the dev SSD (targets, to be measured); 100k events replay < 2 s |
| stress | kill -9 / process abort during a write loop: reopen yields a consistent prefix, no gap; disk full returns an error, not a panic |
| scalability | 1M events: file size and replay-until-midpoint time recorded per 100k (no claim until measured) |
| fixtures | golden postcard bytes for schema v1 (frozen); an event log fixture generated at test time |
| security | tampered record rejected by decode, not trusted; lock records cannot be forged by a Biscuit-less caller (enforced by the kernel, tested at integration level) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Event Append And Read | struct | `EventLog` | `crates/polaris-storage/crates/eventlog/src/log.rs` | `append_assigns_gap_free_seq`, `read_range_is_ordered`, `batch_is_atomic`, `reopen_preserves_head` |
| Replay To Step | method | `EventLog::replay` | `crates/polaris-storage/crates/eventlog/src/replay.rs` | `replay_stops_at_until`, `replay_from_checkpoint_matches_full_replay`, `replay_empty_log_is_empty`, `replay_rejects_newer_schema` |
| Lock Table Recovery | struct | `LockTable` | `crates/polaris-storage/crates/eventlog/src/locks.rs` | `stale_locks_listed_on_open`, `acquire_conflict_is_error`, `release_removes_row`, `lock_survives_reopen` |

**Risks & open questions.**
- redb takes an exclusive database lock per process (**LIKELY**; API to confirm), so only one process can own `events.redb`: with the daemon running, an embedded CLI must connect instead (to be settled with the P9 rpc work).
- postcard is not self-describing: renaming/reordering fields or enum variants silently breaks old logs. Mitigation: `schema` version gate, append-only variants, golden fixtures (**VERIFIED** as a property of postcard's format design; severity LIKELY high).
- The L2 exact cache lives in this database but `polaris-storage-cache` depends only on sdk and cas. A trait (`ExactCacheStore`) is needed; it is not in the sdk registry list in `workspace.toml`. Open: add to sdk or to the cache crate (toml owner).
- The chatlog's `.polaris/invariants.redb` (msg 31) conflicts with the plan's single `events.redb` holding invariants; the plan wins (VERIFIED), flagged for the layout ADR.
- redb 4.x API (transactions, durability, compaction): API to confirm at implementation.

**Chatlog sources.** Msg 13 (transactional append-only log, immutable domain events, only approved events persisted), msg 18 (`events.redb`, time-travel debugging, L2 exact KV, `/session replay [step]`), msg 11 (redb for file locks, state-machine state, response cache), msg 21 and 23 (research checkpoints, CAS snapshots), msg 31 (invariants + `RULES.md` mirror). Corrected: sled mentions replaced by redb; CockroachDB rejected; blobs live in CAS files, not redb.

---

### `polaris-storage-registry` — Global session registry on SQLite (registry.db) with FTS5 trigram search over history

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/registry` | L1 | lib | `polaris-storage` | P1 |

**Purpose.** The machine-wide catalogue of sessions: id, name, workspace, branch, base commit, token counters, sandbox checkpoint ids, status. It also holds an FTS5 trigram index over prompts and responses for `/sessions` and `polaris_recall`. SQLite is used (not redb) because several processes (CLI, daemon, desktop) must read it concurrently via WAL.

**API sketch.**
```rust
pub struct Registry { /* owns a dedicated DB thread; requests via channel */ }
pub struct SessionMeta { pub id: SessionId, pub name: String, pub workspace: PathBuf, pub branch: String, pub base_commit: String,
                         pub prompt_tokens: u64, pub completion_tokens: u64, pub sandbox_checkpoint: Option<String>,
                         pub status: SessionStatus, pub created_at: jiff::Timestamp, pub updated_at: jiff::Timestamp }
pub enum SessionStatus { Active, Paused, Archived }
pub struct Hit { pub session: SessionId, pub role: Role, pub snippet: String, pub at: jiff::Timestamp }
impl Registry {
    pub fn open(dir: &Path) -> Result<Self, RegistryError>;                         // WAL, busy_timeout, migrations via user_version
    pub fn upsert_session(&self, m: &SessionMeta) -> Result<(), RegistryError>;
    pub fn get_session(&self, id: SessionId) -> Result<Option<SessionMeta>, RegistryError>;
    pub fn list_sessions(&self, f: &SessionFilter) -> Result<Vec<SessionMeta>, RegistryError>;
    pub fn index_message(&self, s: SessionId, role: Role, text: &str, at: jiff::Timestamp) -> Result<(), RegistryError>;
    pub fn search_history(&self, query: &str, limit: u32) -> Result<Vec<Hit>, RegistryError>;
}
pub enum RegistryError { Open(String), Busy, Migration { from: u32, to: u32 }, BadQuery, Sql(String), Closed }
// P1 as built: no BadQuery (any query string is valid; blank/NUL-only or limit 0 -> empty Vec); added SchemaTooNew,
// Corrupt, BadValue; Migration carries a reason. Role = sdk ChatRole, timestamps = sdk Timestamp (no direct jiff dep).
// Also: index_message_redacted<R: Redactor>, archive/unarchive/set_status(.., at) -> bool, RegistryConfig + open_with.
```

**Dependencies.** Internal: `polaris-sdk`. External: `rusqlite = 0.40.2` (bundled), `serde = 1.0.229`.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/registry_{session_registry_crud,history_indexing,history_search,fts_query_sanitising,schema_migration,database_open}.json` (implemented by a delegated subagent, test-first; independently re-run: 47 tests green, §E17).
- [x] Schema v1 (`sessions`, `session_history_fts` with `tokenize='trigram'`, `PRAGMA user_version`); forward-only migrations, all pending steps in one `IMMEDIATE` transaction (a failing step rolls back layout, version and rows; a newer schema is refused).
- [x] Gate test: FTS5 trigram works in the bundled build on Windows (`bundled_sqlite_has_fts5_trigram`; bundled SQLite **3.53.2**, VERIFIED from the libsqlite3-sys 0.38.2 header).
- [x] DB-thread actor: `Registry` is a sync `Send + Sync` handle over a channel of enum requests to one owner thread (no `Mutex<Connection>`, no `dyn`); async callers use `spawn_blocking`.
- [x] WAL (checked; a non-WAL result is refused), `busy_timeout` 5 s, `foreign_keys=ON`, `synchronous=NORMAL`; a blocked writer returns `Busy` after the bounded wait.
- [x] `search_history`: the query is one FTS5 phrase with doubled quotes, bound as a parameter (`MATCH ?1`); queries under 3 characters use an escaped `LIKE ?1 ESCAPE ''`; results capped (`MAX_SEARCH_LIMIT` 1000), newest first, 160-char snippet window.
- [x] Masked-only indexing: `index_message_redacted<R: Redactor>` (sdk trait, no dependency on `polaris-security`); a seeded secret is absent from `registry.db`, `-wal` and `-shm` while open and after close, with a negative control proving the scan can fail. Sign-off with `polaris-security-pii` on the masked-only decision: open (P4).
- [x] Windows: path with spaces + non-ASCII, two handles share WAL data, 4 readers + 1 writer. Network drives: refused when WAL cannot be enabled (untested). Antivirus lock tolerance beyond `busy_timeout`: open.
- [x] `/sessions` ordering (updated_at desc, id desc), status/workspace/limit filters, archive/unarchive.
- [ ] Fuzz `fuzz_registry_fts_query`, `fuzz_registry_migration_open`; load/scalability (100k/1M messages); two-process writers and kill-during-write — open. Known imprecision: the snippet window is located on the lower-cased text, so characters whose lower case is longer (e.g. `İ`) can shift the window by a few characters (no panic; char-based slicing).

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | upsert is idempotent and updates counters; get of unknown id -> `None`; migration from v0 fixture to v1 |
| proptest | any text indexed is findable by any >= 3-char substring (trigram property); `search_history` never errors on arbitrary query strings |
| fuzz | `fuzz_registry_fts_query` (arbitrary bytes as query: no panic, no SQL error leakage), `fuzz_registry_migration_open` (mutated db header) |
| load | 100k indexed messages: search p99 < 50 ms (target); concurrent readers x4 while one writer inserts |
| scalability | 1M messages: index size and query time recorded (no claim until measured) |
| stress | two processes writing simultaneously: no corruption, `Busy` handled; kill during write leaves a consistent DB |
| security | seeded secrets never appear in the db file (strings scan); query `"foo\"bar" OR *` does not change SQL semantics |
| fixtures | db files at schema v0/v1 generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Session Registry CRUD | struct | `Registry` | `crates/polaris-storage/crates/registry/src/sessions.rs` | `upsert_then_get_roundtrip`, `upsert_updates_token_counters`, `list_orders_by_updated_desc`, `archive_changes_status` |
| History Search | method | `Registry::search_history` | `crates/polaris-storage/crates/registry/src/search.rs` | `finds_substring_of_three_chars`, `short_query_uses_like_fallback`, `quote_in_query_is_escaped`, `limit_is_enforced` |
| Schema Migration | function | `migrate` | `crates/polaris-storage/crates/registry/src/migrate.rs` | `fresh_db_gets_latest_version`, `v0_upgrades_to_v1_keeping_rows`, `newer_version_is_refused`, `failed_migration_rolls_back` |

**Risks & open questions.**
- FTS5 availability in rusqlite's bundled SQLite is **LIKELY** but not verified here; the first TODO is a go/no-go test. Trigram tokenizer needs SQLite >= 3.34 (**LIKELY**; to confirm in the bundled version).
- The chatlog's `Arc<Mutex<Connection>>` with `unwrap()` is rejected (zero-panic, blocking in async).
- Persisting raw prompts/responses locally can store secrets and PII; the masked-only decision is a **proposal** needing sign-off with `polaris-security-pii`.
- Bundled SQLite needs a C compiler; the MSVC Build Tools are VERIFIED present on this machine, but CI images and the Linux leg are not.
- rusqlite 0.40.2 API (`Connection::open`, `execute_batch`, params): API to confirm at implementation.

**Chatlog sources.** Msg 18 (`GlobalSessionRegistry`, `registry.db`, `SessionMetadata` fields, FTS5 trigram, `/sessions`, `/session resume <id>`), msg 21 (`polaris_recall` search), msg 15 (ConTree checkpoint id column). Corrected: `Mutex<Connection>` + `unwrap` -> DB thread; `rusqlite 0.32` -> 0.40.2; `vtab` feature not needed for FTS5.

---

### `polaris-storage-cache` — Tiered caches: L1 quick_cache (RAM), L2 exact redb cache, L3 semantic cache via the VectorStore trait

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/cache` | L1 | lib | `polaris-storage` | P1 |

**Purpose.** Caches S1 decisions and S2 completions keyed by stable content hashes. L1 is an in-RAM `quick_cache` (replacing the chatlog's `DashMap`, decision D8); L2 is an exact-match persistent tier supplied through a trait (physically the `ExactCache` table in `events.redb`); L3 is a fuzzy semantic tier backed by the sdk `VectorStore`. It honours `$storage-level(global|isolated)` scoping and exposes hit/miss counters to telemetry for `/cache stats` and `/cache purge`.

**API sketch.**
```rust
pub enum Scope { Global, Workspace(WorkspaceId) }                            // $storage-level(global|isolated)
pub struct CacheKey { /* scope + kind + 32-byte blake3 digest */ }
pub enum KeyKind { S1Decision, S2Completion, Embedding }
pub struct TieredCache<L2: ExactCacheStore, L3: VectorStore> { /* L1 quick_cache + injected L2/L3 */ }
impl<L2: ExactCacheStore, L3: VectorStore> TieredCache<L2, L3> {
    pub fn new(cfg: CacheConfig, l2: L2, l3: Option<L3>) -> Self;
    pub fn get(&self, k: &CacheKey) -> Result<Option<CachedValue>, CacheError>;       // L1 -> L2 -> L3 (L3 only if enabled and threshold met)
    pub fn put(&self, k: CacheKey, v: CachedValue, ttl: Option<core::time::Duration>) -> Result<(), CacheError>;
    pub fn get_or_compute<F, Fut>(&self, k: CacheKey, f: F) -> impl Future<Output = Result<CachedValue, CacheError>> + Send
        where F: FnOnce() -> Fut + Send, Fut: Future<Output = Result<CachedValue, CacheError>> + Send;   // single-flight per key
    pub fn purge(&self, filter: PurgeFilter) -> Result<PurgeReport, CacheError>;
    pub fn stats(&self) -> CacheStats;                                          // per-tier hits, misses, evictions, bytes
}
pub trait ExactCacheStore: Send + Sync { fn get(&self, k: &[u8; 32]) -> Result<Option<Vec<u8>>, CacheError>; fn put(&self, k: [u8; 32], v: &[u8]) -> Result<(), CacheError>; fn clear_scope(&self, s: &Scope) -> Result<u64, CacheError>; }
pub fn key_for_completion(model: &ModelId, params_digest: &[u8; 32], prefix: &Cid, tail: &[u8]) -> CacheKey;   // blake3
pub fn key_for_s1(schema_digest: &[u8; 32], input: &[u8]) -> CacheKey;      // blake3 for safety decisions; see risks
pub enum CacheError { L2(String), L3(String), Codec(String), Poisoned }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-cas`. External: `quick_cache = 0.7.0`, `xxhash-rust = 0.8.19` (xxh3), `blake3 = 1.8.7`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: L1 get/put with weights, key derivation, scope isolation, single-flight compute, purge.
- [ ] Define the `ExactCacheStore` trait and decide its home (see risks); implement an in-memory test double only for unit tests, never shipped.
- [ ] Implement key derivation: S2 completion keys hash `(model, sampling params, stable prefix Cid, tail)`; S1 keys hash `(schema digest, input)`.
- [ ] Implement L1 with `quick_cache` weighting by bytes and a configurable capacity; no global static.
- [ ] Implement `Scope` namespacing: keys from different workspaces never collide under `isolated`; `global` shares L2/L3.
- [ ] Implement single-flight `get_or_compute` (concurrent identical requests run `f` once), cancel-safe.
- [ ] Implement the optional L3 semantic lookup with a similarity threshold (chatlog 0.98) and **only** for non-safety kinds; thresholds are configuration, labelled GUESSING until calibrated.
- [ ] Implement `purge` (`/cache purge`) and `stats` (`/cache stats`), and publish counters for telemetry.
- [ ] Windows: nothing platform-specific except L2 file locking, which is the eventlog crate's concern; add a note and a cross-crate integration test in `polaris-tests`.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | L1 hit/miss; TTL expiry with fake clock; L2 promoted into L1 on hit; L3 skipped for safety kinds |
| proptest | `get(put(k, v)) == v` until eviction; keys for different scopes never equal; purge(scope) removes exactly that scope |
| loom | single-flight state machine (N waiters, one computation, cancellation of the leader) |
| load | L1 hit < 200 ns, miss-to-L2 hit < 1 ms (targets); 8 threads x 1M ops |
| stress | capacity pressure with eviction storms; `f` panics (tokio join error) does not poison the key |
| security | collision test: two inputs with equal xxh3 but different blake3 never share a safety-decision entry |
| fixtures | recorded synthetic S1/S2 request/response pairs with expected keys (`insta`) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Cache Key Derivation | function | `key_for_completion` | `crates/polaris-storage/crates/cache/src/keys.rs` | `same_inputs_same_key`, `param_change_changes_key`, `scope_isolates_keys`, `prefix_cid_changes_key` |
| Single Flight Compute | method | `TieredCache::get_or_compute` | `crates/polaris-storage/crates/cache/src/singleflight.rs` | `concurrent_callers_run_f_once`, `leader_cancel_wakes_waiters`, `error_not_cached`, `ttl_expiry_recomputes` |
| Tier Promotion | method | `TieredCache::get` | `crates/polaris-storage/crates/cache/src/tiers.rs` | `l2_hit_promotes_to_l1`, `l3_never_used_for_safety_kind`, `miss_returns_none`, `stats_count_each_tier` |

**Risks & open questions.**
- Safety-verdict cache poisoning: keying S1 safety decisions by xxh3 (as the chatlog's fingerprint table does) lets an adversary craft collisions (**LIKELY**; xxh3 is non-cryptographic). Design rule: blake3 for anything security-relevant; xxh3 only for in-RAM dirty-checks.
- `ExactCacheStore` is not in the sdk trait list in `workspace.toml`, and this crate does not depend on eventlog; the trait must live in sdk or here, then be wired at the composition root. Open question for the toml owner.
- Semantic L3 correctness: a >0.98 embedding similarity does not guarantee the same answer for code tasks; restrict to low-risk kinds. **UNVERIFIED** benefit.
- quick_cache 0.7.0 API (weighter, placeholder/guard API for single-flight): API to confirm at implementation.
- Prefix-cache hit rates quoted in the chatlog (80% RTT discount, 94% HUD) are provider-side claims, **UNVERIFIED** until S0-5.

**Chatlog sources.** Msg 18 (multi-tier cache: L1 RAM DashMap+xxh3, L2 exact redb+BLAKE3, L3 semantic LanceDB >0.98, `/cache stats|purge`, `$storage-level`), msg 15 (L1 fingerprint table, `CachedAnalysis`), msg 29 (S1 result cache). Corrected: `dashmap` -> `quick_cache` (D8); xxh3 never used as a safety key; semantic tier restricted.

---

### `polaris-storage-vector` — Embedded vector store (usearch + SQLite; LanceDB opt-in) for RAG, NVIDIA docs, session memory and the L3 semantic cache

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/vector` | L1 | lib | `polaris-storage` | P5 |

**Purpose.** Implements the sdk `VectorStore` trait over a usearch HNSW index with SQLite metadata under `~/.polaris/vectors` and `.polaris/vectors` (S0-9 decision, §0.1); LanceDB is an opt-in `lancedb` feature. It is a leaf crate so any heavy dependency tree stays out of early phases. Embeddings come from Token Factory `Qwen/Qwen3-Embedding-8B` now (D8) and Needle 3 later; the store itself is embedding-agnostic and records the embedding model id and dimension per table.

**API sketch.**
```rust
pub struct LanceStore { /* lancedb connection + runtime handle */ }
pub struct TableSpec { pub name: String, pub dim: u32, pub embedder: ModelId, pub metric: Metric }
pub enum Metric { Cosine, L2, Dot }
pub struct Chunk { pub id: ChunkId, pub vector: Vec<f32>, pub text: String, pub source: SourceRef, pub meta: ChunkMeta }
pub struct Match { pub id: ChunkId, pub score: f32, pub text: String, pub source: SourceRef }
impl LanceStore {
    pub fn open(root: &Path) -> impl Future<Output = Result<Self, VectorError>> + Send;
    pub fn ensure_table(&self, spec: &TableSpec) -> impl Future<Output = Result<(), VectorError>> + Send;
    pub fn upsert(&self, table: &str, chunks: &[Chunk]) -> impl Future<Output = Result<u64, VectorError>> + Send;   // dim mismatch -> error
    pub fn search(&self, table: &str, vector: &[f32], k: u32, filter: Option<&Filter>) -> impl Future<Output = Result<Vec<Match>, VectorError>> + Send;
    pub fn delete_source(&self, table: &str, source: &SourceRef) -> impl Future<Output = Result<u64, VectorError>> + Send;
    pub fn build_index(&self, table: &str) -> impl Future<Output = Result<(), VectorError>> + Send;
}
pub enum VectorError { Open(String), DimMismatch { expected: u32, found: u32 }, EmbedderMismatch { table: ModelId, given: ModelId }, NoSuchTable(String), Engine(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `usearch = 2.26.5` + `rusqlite = 0.40.2` (default); optional `lancedb = 0.40.0` with `features = ["remote"]` (job.rs gating bug) behind the `lancedb` feature (requires arrow ^58 and datafusion ^54 -- use lancedb's re-exports, never pin arrow directly; needs protoc).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: table creation with dimension/embedder metadata, upsert, filtered search, source deletion, fallback trigger.
- [x] Spike S0-9: LanceDB 36 min / 4.8 GB target / 249 MB exe → no-go as default; usearch + SQLite 1 min 41 s / 4.3 MB / kNN p50 0.475 ms → default (`@prime/spikes/s0-9-lancedb/RESULT.md`).
- [ ] HNSW persistence (save to temp + atomic rename), parallel insert, recall@10 vs exact search on a labelled fixture.
- [ ] Implement `ensure_table` storing `(dim, embedder, metric)` in table metadata; refuse mixing embedders in one table.
- [ ] Implement `upsert` by `ChunkId` (idempotent) with batch limits and dimension validation.
- [ ] Implement `search` with a metadata filter and score normalisation; define behaviour on empty table (empty result).
- [ ] Implement ANN index build thresholds (flat scan below N rows, index above) -- N chosen from S0-9.
- [ ] Bridge the async LanceDB API to the sdk trait without nesting runtimes (use the caller's tokio runtime).
- [ ] Implement the `usearch` fallback behind a Cargo feature only if S0-9 fails; keep the trait identical.
- [ ] Windows: handle file locks on index files, `target-dir` length, and path normalisation (`dunce`); the Linux leg builds in WSL2.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | dimension mismatch rejected; upsert same id replaces; delete_source removes only that source; empty-table search is empty |
| proptest | exact top-1 retrieval: any inserted vector queried returns itself with the best score (flat scan) |
| spike | S0-9: build cost/RAM/binary size; 10k and 100k vector search latency; pass/fail recorded with raw output |
| load | 100k upserts/min and p99 search < 50 ms at 100k vectors (targets) |
| scalability | 10k / 100k / 1M vectors: on-disk size and latency table (no claim until measured) |
| stress | crash during upsert leaves a readable table; low-RAM (6 GB WSL cap) index build |
| fixtures | small synthetic embeddings generated at test time (deterministic seed); no model calls |
| security | embedder-id mismatch cannot silently mix vector spaces |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Vector Table Metadata | method | `LanceStore::ensure_table` | `crates/polaris-storage/crates/vector/src/table.rs` | `creates_table_with_dim`, `reopen_keeps_metadata`, `different_embedder_rejected`, `different_dim_rejected` |
| Vector Upsert And Search | method | `LanceStore::search` | `crates/polaris-storage/crates/vector/src/search.rs` | `inserted_vector_is_top_hit`, `filter_restricts_results`, `k_larger_than_rows_returns_all`, `empty_table_is_empty` |
| Source Deletion | method | `LanceStore::delete_source` | `crates/polaris-storage/crates/vector/src/delete.rs` | `deletes_only_matching_source`, `returns_deleted_count`, `delete_missing_source_is_zero`, `search_after_delete_excludes_rows` |

**Risks & open questions.**
- lancedb 0.40 pulls Arrow 58 and DataFusion 54 (VERIFIED); measured in S0-9: 36 min at -j4, 6.4 GB peak compiler RAM, 4.8 GB target, 249 MB exe (VERIFIED) — the reason it is opt-in. Fat LTO over slint may still exceed 16 GB (thin LTO daily).
- LanceDB needs `protoc` for its own builds (VERIFIED, S0-9 attempt 1); protoc 36.0 is now a per-PC install (`win-protoc`, optional) — never a path in the repo's `.cargo/config.toml`.
- `Qwen3-Embedding-8B` output dimension is **UNVERIFIED** here; the dimension is data in `TableSpec`, never a constant. Embedding calls need a key (BLOCKED-ON-ACCESS); local Needle 3 is gated by S0-6.
- lancedb 0.40.0 API surface (connection, table, merge-insert, index builders) and its exact Arrow re-exports: API to confirm at implementation.
- Chatlog references `lancedb 0.14` and "sled/RocksDB" for docs; both are stale.

**Chatlog sources.** Msg 11 (LanceDB embedded vector store under `.polaris/storage`), msg 18 (L3 semantic cache, session memory, codebase RAG, doc embeddings), msg 33 (Needle 3 embeddings + LanceDB local semantic search, `/search-local`), msg 17 (`fetch_codebase_rag`). Corrected: version 0.14 -> 0.40.0; Arrow pinned via lancedb re-exports; dimension not hard-coded.

---

### `polaris-storage-sync` — Byte-exact, blake3-incremental workspace mirroring into sandboxes, governed by .polarisignore

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-storage/crates/sync` | L1 | lib | `polaris-storage` | P2 |

**Purpose.** The Windows workspace stays authoritative; this crate mirrors it into the sandbox volume (OpenShell WSL2 gateway, Contree, remote gateway). It computes a manifest `(path, size, blake3)`, diffs it against the remote manifest and sends only changed blobs through the sdk `SandboxBackend` transport. Transfer is byte-exact (no EOL translation) and governed by `.polarisignore`; a hard deny-list keeps secrets (`.env`, `*.pem`, `*.key`, `.git/config`) out of every sandbox.

**API sketch.**
```rust
pub struct Manifest { pub entries: BTreeMap<RelPath, FileEntry> }                      // deterministic ordering
pub struct FileEntry { pub size: u64, pub blake3: Cid, pub mode: FileMode, pub kind: EntryKind }
pub enum EntryKind { File, Dir, Symlink { target: RelPath } }
pub struct IgnoreRules { /* .polarisignore (gitignore syntax) + non-overridable DenyList */ }
pub struct DenyList(Vec<Pattern>);                                                     // defaults: .env, *.pem, *.key, .git/config
pub fn scan(root: &Path, rules: &IgnoreRules, prev: Option<&ScanCache>) -> Result<(Manifest, ScanCache), SyncError>;
pub struct SyncPlan { pub upload: Vec<RelPath>, pub delete: Vec<RelPath>, pub rename_hints: Vec<(RelPath, RelPath)>, pub conflicts: Vec<PathConflict>, pub denied: Vec<RelPath> }
pub fn plan(local: &Manifest, remote: &Manifest) -> SyncPlan;
pub enum PathConflict { CaseOnly { a: RelPath, b: RelPath }, ReservedName(RelPath), TrailingDotOrSpace(RelPath), AlternateStream(RelPath), TooLong(RelPath) }
pub fn apply<B: SandboxBackend>(backend: &B, root: &Path, plan: &SyncPlan, opts: ApplyOpts) -> impl Future<Output = Result<SyncReport, SyncError>> + Send;
pub enum SyncError { Walk(String), Io(..), Backend(String), Conflicts(Vec<PathConflict>), RootEscape(RelPath) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-cas`. External: `ignore = 0.4.33`, `globset = 0.4.20`, `blake3 = 1.8.7`, `walkdir = 2.5.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: manifest scan, ignore + deny-list evaluation, diff/plan, conflict detection, apply with partial-failure resume.
- [ ] Spike S0-7 first: sync performance and EOL/case fidelity (Windows -> WSL2 ext4 volume vs `/mnt/c`), 1k/10k/100k files; record raw output.
- [ ] Implement `scan` with an incremental `ScanCache` (mtime+size -> cached hash; rehash on mismatch) and a "paranoid" mode that always rehashes; record cache-validity limits.
- [ ] Implement `IgnoreRules`: `.polarisignore` via gitignore syntax, plus a **separate hard `DenyList` evaluated after** so a `!.env` negation cannot re-include it.
- [ ] Implement `plan` (pure function over two manifests) including rename hints by equal blake3.
- [ ] Implement conflict detection for Windows: case-only collisions, reserved device names (`CON`, `NUL`, `COM1`...), trailing dot/space, NTFS alternate streams, over-long paths; block the sync of conflicting entries with a clear report.
- [ ] Implement symlink/junction policy: never follow outside the root; record links as links; `RootEscape` for any resolved path outside the workspace.
- [ ] Implement `apply` over the `SandboxBackend` transport with bounded concurrency (semaphore), resumable on partial failure, byte-exact writes, executable bit preserved.
- [ ] Integrate a secret-content pre-check hook (caller-provided closure using `polaris-security-automata`) so secrets in non-denied files are flagged before upload (cross-crate wiring at the composition root).
- [ ] Handle notify overflow from the platform watcher as a forced full rescan (contract documented here, implemented by the caller).

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | manifest of a known tree; deny-list beats negation; plan = empty for equal manifests; rename detected by hash |
| proptest | `apply(plan(a, b))` transforms `b` into `a` (model-based, in-memory backend); plan is deterministic and order-independent |
| fuzz | `fuzz_sync_ignore_patterns` (arbitrary `.polarisignore` text), `fuzz_sync_manifest_decode` |
| scalability | 1k / 10k / 100k files: initial scan and no-change rescan times (targets: no-change rescan of 10k files < 1 s with warm cache; to be measured) |
| load | 100 MB changed across 1k files uploads at link speed with bounded memory (< 256 MB RSS, target) |
| stress | file changes during scan (torn read), disappearing files, 1M-file tree, file locked by another process |
| spike | S0-7: sync performance and EOL/case fidelity (Windows -> WSL2) |
| security | symlink pointing at `C:\Users\...\.ssh` is not followed; `.env` never uploaded even with `!.env` in `.polarisignore`; path-traversal names (`..\`) rejected |
| fixtures | hostile paths (reserved names, case clashes, ADS) and CRLF/binary files **generated at test time**, never committed (AV/secret scanners) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Manifest Scan | function | `scan` | `crates/polaris-storage/crates/sync/src/scan.rs` | `scan_is_deterministic`, `unchanged_file_not_rehashed_with_warm_cache`, `changed_content_same_mtime_detected_in_paranoid_mode`, `denied_files_absent_from_manifest` |
| Ignore And Deny Rules | struct | `IgnoreRules` | `crates/polaris-storage/crates/sync/src/ignore.rs` | `polarisignore_excludes_matches`, `deny_list_defaults_present`, `negation_cannot_reinclude_denied`, `git_config_denied` |
| Sync Plan | function | `plan` | `crates/polaris-storage/crates/sync/src/plan.rs` | `equal_manifests_empty_plan`, `new_file_uploaded`, `removed_file_deleted`, `rename_detected_by_hash` |
| Path Conflict Detection | function | `detect_conflicts` | `crates/polaris-storage/crates/sync/src/conflicts.rs` | `case_only_pair_flagged`, `reserved_name_flagged`, `trailing_dot_flagged`, `alternate_stream_flagged` |

**Risks & open questions.**
- mtime-based skip can miss same-size edits within timestamp granularity or on drvfs/9p mounts (**LIKELY**); paranoid mode and S0-7 data decide the default.
- A pattern deny-list is best-effort: secrets in arbitrary files pass it. Defence in depth needs the automata secret scan (cross-crate wiring; open question who owns the closure at the composition root).
- Case-insensitive Windows -> case-sensitive ext4 sync can silently drop or clobber one of two case-clashing files; surfaced as conflicts, never auto-resolved (**VERIFIED** design intent; behaviour UNVERIFIED until S0-7).
- `ignore 0.4.33` honours `.gitignore` by default; the crate must configure custom ignore files explicitly. API to confirm at implementation.
- Transport shape (`SandboxBackend::upload` signature, streaming vs batch) is defined by the sdk trait and not final.

**Chatlog sources.** Msg 15 (incremental sync to ConTree: send only blobs whose BLAKE3 is missing remotely, up to 95% traffic saved -- **UNVERIFIED**), msg 11/13 (lease/OCC context), plan (`.polarisignore`, byte-exact, host-initiated traffic only, deny-list default). Corrected: workspace authoritative on Windows; secrets excluded by a non-overridable deny-list; EOL translation forbidden.

---

### `polaris-security` — Facade re-exporting the security children (capability, automata, secrets, pii, rules)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security` | L3 | facade | — | P2 |

**Purpose.** A pure facade over the five security children; it hosts the threat-model documentation (what Biscuit, DFA scanning, the secret vault, the PII vault and the rule packs each bound, and what they explicitly do not prevent). It sits at L3 because `pii` and `rules` are L3; the L1 children remain usable on their own.
**Re-exports.** `pub use polaris_security_capability as capability; pub use polaris_security_automata as automata; pub use polaris_security_secrets as secrets; pub use polaris_security_pii as pii; pub use polaris_security_rules as rules;`
**Dependencies.** Internal: the five security children. External: none.
**TODO.**
- [ ] Write the threat model: assets (API keys, workspace, host), adversaries (prompt injection via web/docs, hostile repo, compromised sub-agent), controls per child, residual risk stated honestly (Biscuit bounds damage but does not stop injection; in-RAM sealing does not defeat a same-process attacker).
- [ ] Document the fail-open / fail-closed matrix for S1-backed consumers (safety gate and PII fail closed; triage and watchdog fail open).
- [ ] Add a facade smoke test that every re-export resolves.

---

### `polaris-security-capability` — Biscuit capability tokens: session authority, per-frame attenuation, authorization

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security/crates/capability` | L1 | lib | `polaris-security` | P3 |

**Purpose.** Issues a root Biscuit token per session and lets a parent frame attenuate it for a child (path, churn limit, TTL, no-unsafe, diff hash) without access to the root key; the verifier then authorizes a command against facts about the concrete mutation. This is layer 1 of the three-layer `HybridMutationVerifier` (Datalog/Ed25519, AST metrics, S1 congruence); layers 2 and 3 live in other crates. Honest bound: a token **limits the damage** a compromised sub-agent can do; it does not prevent prompt injection.

**API sketch.**
```rust
pub struct Authority { /* session root keypair, held only in kernel memory; never logged or serialized */ }
pub struct Token(TokenBytes);                         // opaque bytes from the sdk; parse only via this crate
pub struct Scope {
    pub resource: ResourceScope,                      // exact path, path prefix, or AST symbol scope
    pub max_churn_lines: u32,
    pub ttl: core::time::Duration,
    pub allow_unsafe: bool,
    pub diff_blake3: Option<Cid>,                     // bind a token to one exact patch
    pub rights: Rights,                               // read | propose_diff | execute_tool | spawn_subagent
}
pub struct Facts<'a> { pub resource: &'a str, pub churn_lines: u32, pub now: jiff::Timestamp, pub touches_unsafe: bool, pub diff_blake3: Option<Cid>, pub right: Right }
pub trait Clock { fn now(&self) -> jiff::Timestamp; }  // injected; tests use a fake clock
impl Authority {
    pub fn new_session() -> Result<Self, CapabilityError>;
    pub fn public_key(&self) -> PublicKeyBytes;
    pub fn issue_root(&self, agent: FrameId, root: &Scope) -> Result<Token, CapabilityError>;
}
pub fn attenuate(parent: &Token, child: &Scope) -> Result<Token, CapabilityError>;        // no private key needed
pub fn authorize(token: &Token, facts: &Facts<'_>, pk: &PublicKeyBytes) -> Result<Authorization, CapabilityError>;
pub enum Authorization { Allowed { checks_passed: u16 }, Denied { failed_check: CheckId } }   // toml names the return type `Decision`; renamed to avoid clashing with the sdk S1 `Decision`
pub enum CapabilityError { BadSignature, Malformed, Expired, ScopeWidening, Datalog(String), Keygen }
```

**Dependencies.** Internal: `polaris-sdk`. External: `biscuit-auth = 6.0.0` (internally uses rand 0.8; keys must come only through biscuit's own API), `zeroize = 1.9.1` (derive).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: root issuance, attenuation, authorization with each check type, widening rejection, expiry.
- [ ] Define the Datalog vocabulary once (facts: `resource`, `churn`, `time`, `touches_unsafe`, `diff_blake3`, `right`; checks as in the chatlog but with correct polarity) and freeze it in a versioned constant.
- [ ] Implement `Authority::new_session`/`issue_root`; keep the private key inside a type with no `Debug`/`Serialize` and zeroize on drop where the library permits (API to confirm).
- [ ] Implement `attenuate` so each child scope is provably narrower: attenuation adds only `check if` blocks; test that a widened scope still fails authorization.
- [ ] Implement `authorize` with facts from an injected `Clock` (the chatlog hard-coded `current_time = 1729000000`; that is a bug, not a design).
- [ ] Bind `diff_blake3` into checks so a token authorizes one exact patch (prevents payload swap in the command queue).
- [ ] Provide token size/serialisation helpers and a test for deep attenuation chains (child of child of child).
- [ ] Add a revocation-id hook if the library supports it (API to confirm); keep it optional.
- [ ] Windows: nothing platform-specific; add the Linux leg and verify the crate builds with the aws-lc-rs provider (`cargo tree -i ring` stays empty for this crate's tree).

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | root allows its scope; child denies sibling path; expired token denied with the fake clock; wrong public key -> `BadSignature` |
| proptest | for any chain of attenuations, authorization of any facts allowed by the child is also allowed by every ancestor (monotonicity); a child can never gain a right |
| fuzz | `fuzz_biscuit_token_bytes` (arbitrary bytes into `Token` parse + authorize: `Err`, no panic) |
| security | attenuation corpus: attempts to widen path, raise churn limit, extend TTL, add `unsafe`, drop the diff hash -- every one denied; replay of a token for a different diff hash denied |
| load | authorize p99 per call recorded with 1/4/16 blocks (target to be measured; the chatlog's "~5 us" is **UNVERIFIED** and likely optimistic because each block carries an Ed25519 signature) |
| fixtures | golden tokens generated at test time with a fixed seed where the API allows |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Root Token Issuance | method | `Authority::issue_root` | `crates/polaris-security/crates/capability/src/authority.rs` | `root_token_verifies_with_public_key`, `root_token_wrong_key_rejected`, `root_scope_grants_listed_rights_only`, `authority_debug_does_not_print_key` |
| Token Attenuation | function | `attenuate` | `crates/polaris-security/crates/capability/src/attenuate.rs` | `child_cannot_widen_path`, `child_cannot_raise_churn`, `child_ttl_cannot_exceed_parent`, `unsafe_forbidden_stays_forbidden`, `chain_of_three_attenuations_monotone` |
| Mutation Authorization | function | `authorize` | `crates/polaris-security/crates/capability/src/authorize.rs` | `in_scope_mutation_allowed`, `out_of_scope_path_denied`, `expired_token_denied`, `diff_hash_mismatch_denied`, `uses_injected_clock` |

**Risks & open questions.**
- biscuit-auth 6.0.0 is a major bump from the chatlog's 4.0 API; builder/authorizer macros and block creation may differ. **UNVERIFIED**: API to confirm at implementation.
- The chatlog's attenuation example was semantically wrong (it asserted a `churn_limit` fact and compared `<=` against the token's own number, and used `UNIX_EPOCH` seconds with a fixed "now"); the corrected design checks authorizer-supplied facts against token-embedded limits.
- Biscuit bounds, not prevents, injection damage (plan correction, VERIFIED): a token for `src/math/matrix.rs` still lets an injected agent corrupt that file within its limits.
- Where the session root key lives and how child frames in other processes (worker, guest) verify tokens (public key distribution) is not settled; the rpc named-pipe token auth is separate.
- Token-size growth with deep chains and per-block signature verification cost are **UNVERIFIED**.

**Chatlog sources.** Msg 12-13 (Event-driven core, Biscuit/Datalog attenuation, `check if` rules for resource, churn, no-unsafe, TTL; hybrid verifier layer 1), msg 15 (`computed_diff_blake3`/`expected_diff_blake3` payload attestation), msg 1 (child sub-agent ceilings), plan (child frames inherit attenuated tokens). Corrected: biscuit 4.0 -> 6.0.0; fixed-time and inverted-limit bugs; `ed25519-dalek` dependency dropped (keys via biscuit); "mathematical guarantee against prompt injection" reduced to damage bounding.

---

### `polaris-security-automata` — Startup-compiled DFA scanners for secrets, PII masks and dangerous shell patterns, plus a regex-syntax HIR guard

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security/crates/automata` | L1 | lib | `polaris-security` | P2 |

**Purpose.** Compiles multi-pattern rule sets once at startup and scans bytes in one linear pass without allocating. It backs secret detection, PII masks, dangerous-shell-command detection (blast radius, safety gate) and log redaction. User-supplied regexes (for example in directives) pass an HIR guard that bounds size and nesting. It also provides Shannon-entropy helpers for high-entropy secret candidates.

**API sketch.**
```rust
pub struct RuleSet { /* compiled patterns + metadata */ }
pub struct RuleSetBuilder { /* patterns with ids, severity, category */ }
impl RuleSetBuilder { pub fn add(&mut self, id: RuleId, pattern: &str, cat: Category) -> Result<&mut Self, AutomataError>; pub fn build(self, limits: &CompileLimits) -> Result<RuleSet, AutomataError>; }
pub struct CompileLimits { pub dfa_size_bytes: usize, pub max_patterns: u32, pub nest_limit: u32 }
pub struct DfaScanner { /* forward DFA set + reverse DFAs for start offsets + Aho-Corasick literal prefilter */ }
pub struct ScanBuf { /* caller-owned, pre-sized match storage */ }
pub struct Match { pub rule: RuleId, pub start: u32, pub end: u32 }
impl DfaScanner {
    pub fn new(rules: RuleSet) -> Result<Self, AutomataError>;
    pub fn scan(&self, haystack: &[u8], out: &mut ScanBuf) -> Result<usize, AutomataError>;   // hot path: zero-alloc (DFA/PII scan)
    pub fn any_match(&self, haystack: &[u8]) -> Result<bool, AutomataError>;                 // hot path: zero-alloc
}
pub fn validate_user_regex(pattern: &str, limits: &UserRegexLimits) -> Result<ValidatedPattern, AutomataError>;
pub fn shannon_entropy_milli(bytes: &[u8]) -> u32;                                           // millibits/byte, integer result, deterministic
pub fn high_entropy_spans(haystack: &[u8], cfg: &EntropyCfg, out: &mut ScanBuf) -> usize;    // candidate secrets (base64/hex-ish runs)
pub struct DfaRedactor { /* impl sdk Redactor for log redaction */ }
pub enum AutomataError { Syntax(String), TooLarge { limit: usize }, TooNested, TooManyPatterns, Search(String), BufferFull }
```

**Dependencies.** Internal: `polaris-sdk`. External: `regex-automata = 0.4.18` (default-features off; `std`, `syntax`, `perf`, `unicode`, `dfa-build`, `dfa-search`, `hybrid`, `meta`), `regex-syntax = 0.8.11` (`std`, `unicode`), `aho-corasick = 1.1.5`, `memchr = 2.8.3`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: rule compile with limits, scan, any_match, HIR guard, entropy spans, redactor.
- [ ] Spike: build a dense DFA for the default rule pack (secrets + PII + shell patterns) and record state count, memory and build time under `CompileLimits`; fall back to `hybrid` (lazy DFA with a per-thread cache) if dense blows up.
- [ ] Implement the `RuleSetBuilder` with `regex-syntax` parsing, nest/size limits and typed errors; the chatlog's `.unwrap()` on search is replaced by `Result`.
- [ ] Implement `scan` producing spans: forward DFA gives end offsets; pair with a reverse DFA (or the meta engine) for start offsets; use Aho-Corasick/memchr as a literal prefilter (`AKIA`, `-----BEGIN`, `sk-`).
- [ ] Implement overlapping/all-match semantics where needed and document leftmost-first behaviour for masking.
- [ ] Implement `validate_user_regex`: reject oversize HIR, deep nesting, huge counted repetitions and large Unicode classes; Rust regex has no backtracking, so the real risk is compile-time blowup, not ReDoS.
- [ ] Implement integer-only entropy (no `as` casts: `u32::try_from`, `f64::from`, or a fixed-point log table) and `high_entropy_spans` with configurable minimum length and threshold.
- [ ] Implement `DfaRedactor` against the sdk `Redactor` trait with a caller-owned output buffer.
- [ ] Counting-allocator tests for `scan`, `any_match` and `redact_into` after warm-up; document that building rule sets allocates (init only).
- [ ] Windows: Unicode and CRLF handling in patterns, scanning of non-UTF-8 bytes, 32-bit offsets for inputs up to 4 GiB with an explicit `TooLarge` error beyond.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | known secrets (generated fake keys) matched; look-alikes not; every default rule has a positive and a negative case |
| proptest | `scan` result equals the reference `regex` crate results on random haystacks for the supported subset; spans are within bounds and ordered |
| fuzz | `fuzz_automata_scan` (arbitrary haystack), `fuzz_automata_user_regex` (arbitrary pattern into the guard: `Err` or valid, no panic, bounded time/memory) |
| kani / miri | Kani: entropy arithmetic has no overflow for inputs up to a bound; Miri: whole crate (pure Rust) |
| load | scan >= 500 MB/s per core on a 10-rule pack (target, to be measured); `any_match` on 4 KB chunks < 2 us p99 |
| stress | 1 GB haystack streamed in chunks (boundary-spanning matches: define and test chunk-overlap policy); 5,000-pattern set rejected by limits |
| scalability | rule count 10 / 100 / 1,000: build time, DFA bytes and throughput table |
| security | bypass corpus: homoglyphs, zero-width splits, base64-wrapped keys, line-folded secrets -- documented as detected or explicitly out of scope |
| fixtures | fake keys/PII generated at test time (never committed; secret scanners) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Multi-Pattern DFA Scan | method | `DfaScanner::scan` | `crates/polaris-security/crates/automata/src/scan.rs` | `finds_all_rules_in_one_pass`, `reports_start_and_end_offsets`, `no_match_on_clean_text`, `scan_does_not_allocate_after_warmup`, `buffer_full_is_error_not_panic` |
| User Regex Guard | function | `validate_user_regex` | `crates/polaris-security/crates/automata/src/guard.rs` | `rejects_oversize_pattern`, `rejects_deep_nesting`, `accepts_simple_pattern`, `rejects_huge_counted_repetition` |
| Entropy Candidate Detection | function | `high_entropy_spans` | `crates/polaris-security/crates/automata/src/entropy.rs` | `random_base64_flagged`, `english_prose_not_flagged`, `short_runs_ignored`, `entropy_of_constant_is_zero` |
| DFA Redactor | struct | `DfaRedactor` | `crates/polaris-security/crates/automata/src/redactor.rs` | `redacts_matched_spans`, `leaves_clean_text_identical`, `reuses_output_buffer`, `idempotent_on_redacted_text` |

**Risks & open questions.**
- Chatlog feature names `dfa-dense` / `hybrid-search` do **not** exist in 0.4.18 (plan note, VERIFIED); the working set is `dfa-build`, `dfa-search`, `hybrid`, `meta`, `syntax`, `perf`, `unicode`. Exact builder/search calls (`dense::DFA::builder`, `Automaton::try_search_fwd`, overlapping search): API to confirm at implementation.
- Dense DFAs can explode in states with Unicode classes and many patterns; `CompileLimits` and a hybrid fallback are the mitigation. Memory/time are **UNVERIFIED** until the spike.
- "ReDoS protection" in the chatlog is partly misleading: linear-time matching is a property of the engine (LIKELY), but pattern *compilation* size is the exploitable axis.
- Entropy thresholds (for example ~4.5 bits/char for 20+ char base64-like runs) are heuristics: **GUESSING** until calibrated against fixtures; false positives on hashes/UUIDs are certain.
- Pattern scanning cannot see secrets split across tokens, encoded, or in binary files; stated as residual risk in the facade threat model.

**Chatlog sources.** Msg 33 (AOT dense DFA compilation, multi-pattern single-pass scan, HIR inspection via regex-syntax, `SecurityDfaScanner`), msg 11 (Aho-Corasick/AST pattern matching, Shannon entropy + masks for secrets, dangerous shell patterns), msg 27 (Rust regex linear-time guarantee), msg 21 (IP/email regexes), msg 31 (blast-radius command analysis). Corrected: version-specific feature names; `unwrap()` removed; ReDoS wording; zero-width/Unicode-evasion limits documented.

---

### `polaris-security-secrets` — Secret vault: keyring storage, secrecy/zeroize handling, ChaCha20-Poly1305 RAM sealing

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security/crates/secrets` | L1 | lib | `polaris-security` | P3 |

**Purpose.** Loads `NEBIUS_API_KEY` / `TAVILY_API_KEY` from the OS keyring (Windows Credential Manager) or the environment into redacting secret values, keeps them sealed in RAM with ChaCha20-Poly1305 under an ephemeral session key, decrypts only for the duration of a call and zeroizes afterwards. Config holds references only (`polaris-config` `SecretRef`); sandboxes never receive keys (OpenShell provider profiles substitute them per endpoint).

**API sketch.**
```rust
pub struct SecretName(String);                                    // validated: [A-Z0-9_]{1,64}
pub struct SecretVault { /* sealed entries; key and ciphertext in separate allocations */ }
pub struct SessionSealKey { /* 32 bytes, zeroize on drop, no Debug/Display/Serialize */ }
impl SecretVault {
    pub fn new() -> Result<Self, SecretsError>;                                          // generates the ephemeral key via getrandom
    pub fn load(&mut self, name: SecretName, source: &SecretRef) -> Result<(), SecretsError>;   // keyring or env
    pub fn store_keyring(&self, name: &SecretName, value: SecretString) -> Result<(), SecretsError>;
    pub fn with_secret<R>(&self, name: &SecretName, f: impl FnOnce(&str) -> R) -> Result<R, SecretsError>;  // plaintext never escapes the closure
    pub fn contains(&self, name: &SecretName) -> bool;
    pub fn forget(&mut self, name: &SecretName);                                          // zeroize + remove
    pub fn seal_bytes(&self, plaintext: &[u8]) -> Result<SealedBlob, SecretsError>;       // reused by the PII vault for sealed resume state
    pub fn open_bytes(&self, blob: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, SecretsError>;
}
pub struct SealedBlob { pub nonce: [u8; 12], pub ciphertext: Vec<u8> }
pub enum SecretsError { NotFound(SecretName), Keyring(String), EnvMissing(String), TooLarge, Seal, Open, Entropy, Locked, Utf8 }
```

**Dependencies.** Internal: `polaris-sdk`. External: `keyring = 4.2.0`, `secrecy = 0.10.3`, `zeroize = 1.9.1`, `chacha20poly1305 = 0.11.0`, `getrandom = 0.4.3`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: seal/open round-trip, `with_secret` scoping, keyring load/store, env fallback, nonce uniqueness.
- [ ] Spike: keyring 4.2.0 on Windows (store/read/delete a test credential; blob size limit; behaviour when no backend exists, for example CI containers). API to confirm; the chatlog code did not use keyring at all.
- [ ] Implement `SecretName` validation and `SecretVault::load` from `SecretRef::Keyring`/`Env` (read env via `var_os`; never set it).
- [ ] Implement sealing: random 96-bit nonce per seal from `getrandom`, ChaCha20-Poly1305 with associated data = secret name; reject nonce reuse by construction (fresh nonce each seal).
- [ ] Keep the session key and each ciphertext in separate heap allocations (chatlog correction); never `Clone` key material; `Drop` zeroizes.
- [ ] Implement `with_secret` so plaintext lives in a `Zeroizing` buffer for the closure's duration only; no `Debug`/`Display` on any type here; `compile_fail` doctests prove it.
- [ ] Decide memory locking: `mlock`/`VirtualLock` needs `unsafe` or a vetted crate; this is one of the plan's declared unsafe exceptions. Measure `VirtualLock` working-set limits on Windows first; if a vetted wrapper is unavailable, ship without locking and state it.
- [ ] Add a leak-hygiene test: a seeded canary secret must not appear in `format!("{:?}")` of any vault type, in tracing output, or in a heap scan after `forget` (best-effort).
- [ ] Windows: Credential Manager blob size limit handling (large values rejected with a typed error), per-user scope documented, Linux leg uses the Secret Service or env fallback.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | seal -> open round-trip; wrong key/tamper -> `Open` error; `with_secret` returns closure result; `forget` makes `contains` false |
| proptest | for any byte string: `open(seal(x)) == x`; two seals of the same plaintext have different nonces/ciphertexts; any single-bit flip fails to open |
| fuzz | `fuzz_secrets_open_blob` (arbitrary nonce+ciphertext: `Err`, no panic) |
| security | canary secret never in Debug/log output; ciphertext/key separate allocations (address check); zeroize-on-drop (best-effort memory inspection, documented limits); env var scrubbed from child processes is verified at integration level in platform-process |
| unit (keyring) | real keyring store/load on Windows -- **SKIPPED: no key / no backend** is reported, never faked, when unavailable (D9) |
| load | `with_secret` < 20 us p99 (target) |
| fixtures | fake keys generated at test time only |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| RAM Sealing | struct | `SecretVault` | `crates/polaris-security/crates/secrets/src/vault.rs` | `seal_open_roundtrip`, `tampered_ciphertext_rejected`, `nonce_differs_per_seal`, `key_and_ciphertext_separate_allocations` |
| Scoped Secret Access | method | `SecretVault::with_secret` | `crates/polaris-security/crates/secrets/src/scope.rs` | `closure_receives_plaintext`, `missing_secret_is_not_found`, `plaintext_zeroized_after_call`, `vault_debug_is_redacted` |
| Keyring Loading | method | `SecretVault::load` | `crates/polaris-security/crates/secrets/src/source.rs` | `env_source_loaded`, `missing_env_is_error`, `keyring_unavailable_is_typed_error`, `oversize_value_rejected` |

**Risks & open questions.**
- In-process sealing protects against secrets in swap/core dumps and casual memory scraping; it does **not** defeat an attacker inside the process, since the key sits in the same address space (plan correction: "the mlock'd key sits next to its ciphertext", VERIFIED as a statement of the plan; mitigation here is separate allocations, not a hardware boundary).
- Memory locking requires `unsafe` or an unpinned helper crate; none is in `workspace.toml`. **UNVERIFIED** which path is chosen; a declared, signed-off exception (plan) or omission.
- `zeroize` cannot clear copies made by moves, stack spills or allocator realloc; fixed-capacity buffers reduce but do not remove this (**LIKELY**).
- `secrecy 0.10` replaced `Secret<T>` with `SecretBox<T>`/`SecretString` (**LIKELY**; prompt.md still says `Secret<T>`). keyring 4.x and chacha20poly1305 0.11 APIs (keygen, AEAD traits) to confirm at implementation; the chatlog code targets secrecy 0.8 / chacha20poly1305 0.10 / rand 0.8 and will not compile as written.
- Windows Credential Manager blob limit (about 2.5 KB) is **LIKELY**; keys are per-user, readable by any process of the same user (not a boundary against local malware).

**Chatlog sources.** Msg 27 (Secure RAM Vault: mlock/VirtualLock, ChaCha20-Poly1305 at rest in RAM, ZeroizeOnDrop; `EncryptedRamSecret`), msg 1 (credential custody: keys injected only at the verified egress), msg 31 (`NEBIUS_API_KEY`/Tavily keys). Corrected: `mlock` via `nix` + `unsafe` dropped from the default design; key and ciphertext separated; overclaims ("decrypts straight into CPU registers") removed; Windows Credential Manager added.

---

### `polaris-security-pii` — Reversible PII vault: DFA masks + entropy + S1 PII-NER, placeholder tokens, unmask only at the sandbox boundary

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security/crates/pii` | L3 | lib | `polaris-security` | P4 |

**Purpose.** Masks emails, IPs, keys, names and internal identifiers before any cloud call, replacing them with session placeholders; the forward/reverse maps stay local. Masking is fail-closed: when the S1 PII-NER (via the sdk `DecisionEngine`, the `gliner2-privacy-filter-PII-multi` companion, ONNX published) is unavailable, DFA + entropy still run. Unmasking happens only on text entering a sandbox for execution, never on text returning to the model.

**API sketch.**
```rust
pub enum PiiKind { Email, Ip, ApiKey, PersonName, Org, InternalId, Path, Other }
pub struct PiiVault { /* forward + reverse maps (values zeroizing), per-session nonce, counters per kind */ }
pub struct Masked { pub text: String, pub report: SanitizeReport }
pub struct SanitizeReport { pub counts: Vec<(PiiKind, u32)>, pub ner_used: bool, pub degraded: bool }
pub struct PlaceholderToken(String);                                  // e.g. "<EMAIL_1#a3f9>" -- kind, index, per-session random tag
impl PiiVault {
    pub fn new(scanner: DfaScanner, entropy: EntropyCfg) -> Result<Self, PiiError>;
    pub fn mask(&mut self, text: &str) -> Result<Masked, PiiError>;                       // DFA + entropy; allocation allowed (vault insert)
    pub async fn mask_with_ner<D: DecisionEngine>(&mut self, text: &str, ner: &D, timeout: core::time::Duration) -> Result<Masked, PiiError>;  // NER error/timeout -> degraded DFA-only result, never unmasked
    pub fn unmask_for_sandbox(&self, text: &str, ctx: &SandboxBoundary) -> Result<String, PiiError>;   // only tokens issued by this vault this session
    pub fn export_sealed<S: Sealer>(&self, sealer: &S) -> Result<SealedBlob, PiiError>;   // resume state, sealed on disk
    pub fn import_sealed<S: Sealer>(sealer: &S, blob: &SealedBlob) -> Result<Self, PiiError>;
}
pub struct SandboxBoundary { /* witness: only constructible by the sandbox exec path */ }
pub trait Sealer { fn seal(&self, plaintext: &[u8]) -> Result<SealedBlob, PiiError>; fn open(&self, b: &SealedBlob) -> Result<Zeroizing<Vec<u8>>, PiiError>; }
pub enum PiiError { Scan(String), Ner(String), UnknownToken, BoundaryDenied, Seal, Capacity { limit: usize } }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-security-automata`. External: `zeroize = 1.9.1`, `secrecy = 0.10.3`. Gap: the sealed-resume feature needs a sealing capability; `polaris-security-secrets` is not in this crate's dependency list, hence the `Sealer` trait (wired at the composition root).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: mask, unmask-at-boundary, placeholder-injection defence, NER degradation, sealed export/import.
- [ ] Define placeholder format with a per-session random tag so attacker-authored text such as `<API_KEY_1>` cannot be unmasked into a secret (placeholder-injection defence).
- [ ] Implement `mask` using `DfaScanner` spans + entropy candidates; replace by single left-to-right pass over a pre-collected match list (not repeated `str::replace`, which the chatlog used and which is quadratic and order-dependent).
- [ ] Make masking stable and idempotent: the same value gets the same token within a session; already-masked text is unchanged.
- [ ] Implement `mask_with_ner`: batch via `DecisionEngine`, apply a timeout, mark `degraded = true` on failure; ensure the degraded path never skips the DFA/entropy layer (fail closed).
- [ ] Implement `unmask_for_sandbox` through a boundary witness type; it must refuse to unmask any text bound for a model or log.
- [ ] Implement `SanitizeReport` counts for UI messages ("sanitized N items").
- [ ] Implement sealed export/import via the `Sealer` trait for `/session resume`.
- [ ] Add maximum vault capacity and eviction policy (error, never silent drop).
- [ ] Windows: UTF-8 and UTF-16 inputs (PowerShell output), path-like identifiers with backslashes, and CRLF boundaries in span replacement.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | the chatlog example (IP, `sk-...` key, email) masked and unmasked correctly; same value -> same token; counts reported |
| proptest | `unmask(mask(x)) == x` for any text when unmasking is allowed; masked output contains none of the extracted values; masking is idempotent |
| fuzz | `fuzz_pii_masker` (arbitrary UTF-8 and bytes), `fuzz_pii_unmask_tokens` (tokens adversarially placed) |
| security | PII-egress capture test: fake secrets/PII generated at test time are absent from the serialized request sent to a wiremock Token Factory; placeholder-injection corpus; fail-closed when NER returns an error |
| load | mask 1 MB of log text < 50 ms with DFA+entropy only (target, to be measured) |
| stress | 100k distinct values hit the capacity limit with a typed error |
| fixtures | fake PII/keys generated at test time; NER outputs recorded as **synthetic** until a real model is available |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| PII Masking | method | `PiiVault::mask` | `crates/polaris-security/crates/pii/src/mask.rs` | `masks_ip_key_email`, `same_value_same_token`, `mask_is_idempotent`, `no_original_value_in_output`, `report_counts_match` |
| Sandbox-Boundary Unmasking | method | `PiiVault::unmask_for_sandbox` | `crates/polaris-security/crates/pii/src/unmask.rs` | `restores_issued_tokens`, `foreign_token_not_restored`, `attacker_written_placeholder_not_restored`, `boundary_witness_required` |
| NER Degradation | method | `PiiVault::mask_with_ner` | `crates/polaris-security/crates/pii/src/ner.rs` | `ner_error_falls_back_to_dfa`, `ner_timeout_marks_degraded`, `degraded_still_masks_known_patterns`, `ner_names_masked_when_available` |
| Sealed Resume State | method | `PiiVault::export_sealed` | `crates/polaris-security/crates/pii/src/seal.rs` | `export_import_roundtrip`, `tampered_blob_rejected`, `wrong_sealer_rejected`, `sealed_blob_has_no_plaintext` |

**Risks & open questions.**
- Placeholder injection: an attacker (web page, repo file) writes a placeholder; unmasking it would leak the real value into attacker-visible output. The per-session tag and boundary-only unmasking are **design requirements added here** (not in the chatlog). Effectiveness UNVERIFIED until the security corpus runs.
- Masking can break code semantics (a masked IP inside a config the model must edit); the sandbox-boundary unmask restores it, but diffs the model returns contain placeholders -- round-trip correctness of patches is **UNVERIFIED** and needs an integration test.
- NER quality for code identifiers and internal names is **UNVERIFIED**; `GLiNER2.5-Decide` ships fp32 safetensors only and the S1 stack is gated by spike S0-4; the PII companion has a published ONNX export (VERIFIED, plan). The chatlog's "<15 ms" S1 latency is false for Decide (vendor CPU ~167 ms on 48 cores), so NER stays off the synchronous fast path or is batched.
- Pattern + NER masking is not a complete DLP; semantic leakage (describing a secret in prose) remains. Stated as residual risk.
- Sealed persistence and vault capacity/eviction semantics are open; also whether the registry indexes masked text (see registry crate).

**Chatlog sources.** Msg 21 (Reversible Tokenization Vault, `PiiVault` with forward/reverse maps, IP/email regexes, GLiNER for names/companies/internal systems, unmask at OpenShell/ConTree boundary, "sanitized N items" message), msg 35/38 (PII sanitising listed as local S1 duty), msg 33 (PII masks in DFA matrix). Corrected: `RwLock` + `unwrap` and repeated `replace` removed; S1 latency claim; unmask scope narrowed to the sandbox boundary; placeholder-injection defence added.

---

### `polaris-security-rules` — Security rule library: ast-grep rule packs, DFA patterns and a CWE map

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-security/crates/rules` | L3 | lib | `polaris-security` | P4 |

**Purpose.** Ships versioned rule packs as data and evaluates them on AST diffs: memory and concurrency (including CUDA), command injection, path traversal / symlink races, secrets hygiene and supply chain. Findings carry a rule id, CWE id and severity. Disputed matches can be arbitrated by S1 through the sdk `DecisionEngine`; the deterministic path stays authoritative when S1 is unavailable. It feeds the security gate used before `ProofOfDone` and `/hack`.

**API sketch.**
```rust
pub struct RulePack { pub id: PackId, pub version: PackVersion, pub rules: Vec<Rule> }
pub struct Rule { pub id: RuleId, pub cwe: Cwe, pub severity: Severity, pub lang: Lang, pub kind: RuleKind, pub message: String }
pub enum RuleKind { AstPattern { pattern: String, constraints: Vec<Constraint> }, DfaPattern { regex: String }, Composite(Vec<RuleId>) }
pub struct Cwe(u16);                                          // e.g. 362 race condition, 78 OS command injection, 22 path traversal
pub enum Severity { Info, Low, Medium, High, Critical }
pub struct Finding { pub rule: RuleId, pub cwe: Cwe, pub severity: Severity, pub file: PathBuf, pub span: ByteSpan, pub snippet_digest: Cid, pub disputed: bool }
pub struct RuleEngine { /* compiled packs: ast-grep patterns + DfaScanner */ }
impl RuleEngine {
    pub fn load(packs: &[RulePack]) -> Result<Self, RulesError>;
    pub fn load_embedded() -> Result<Self, RulesError>;                                   // default packs via include_str!
    pub fn audit_diff(&self, old: &Parsed, new: &Parsed, diff: &DiffHunks) -> Result<Vec<Finding>, RulesError>;   // only added/changed lines
    pub fn audit_command(&self, cmd: &str) -> Result<Vec<Finding>, RulesError>;           // dangerous shell patterns
    pub async fn arbitrate<D: DecisionEngine>(&self, f: &[Finding], s1: &D, timeout: core::time::Duration) -> Result<Vec<Finding>, RulesError>;  // timeout -> keep findings (fail closed)
}
pub fn parse_pack(yaml: &str) -> Result<RulePack, RulesError>;                            // serde-saphyr; schema-validated
pub enum RulesError { Yaml(String), BadPattern { rule: RuleId, reason: String }, UnknownCwe(u16), UnsupportedLang, Engine(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-security-automata`, `polaris-code-intel`. External: `ast-grep-core = 0.51.0`, `serde = 1.0.229`, `serde-saphyr = 2.0.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: pack parsing/validation, AST-pattern evaluation on diffs, DFA-pattern evaluation, command audit, S1 arbitration with fail-closed timeout.
- [ ] Confirm what `ast-grep-core` provides for YAML rule configs (the `ast-grep-config` / `ast-grep-language` crates are not in `workspace.toml`); if core alone is insufficient, implement our own rule schema in `serde-saphyr` mapped to core patterns, and tell the toml owner.
- [ ] Define the pack schema and version it; write the first packs: Rust unsafe/race, C/C++ UAF and double free, CUDA (`__syncthreads()` in divergent branch, unsynchronised shared memory, OOB thread index), command injection (`std::process::Command`/`os.system` with tainted args), path traversal and symlink races, hard-coded credentials, typosquatting heuristics.
- [ ] Build the CWE table with explicit ids (for example 362, 78, 22, 59, 416, 415, 798, 787, 125); mark unmapped supply-chain heuristics as "no CWE assigned" rather than inventing one.
- [ ] Implement `audit_diff` restricted to changed hunks (not whole files) using code-intel spans.
- [ ] Implement `audit_command` using the automata scanner, shared with blast radius; return structured findings, not booleans.
- [ ] Implement `arbitrate`: S1 may downgrade a *disputed* finding only when confidence exceeds a configured bar; timeout or error keeps the finding (fail closed).
- [ ] Add a pack regression corpus: each rule has at least one true-positive and one true-negative snippet.
- [ ] Windows: rule paths use forward-slash normalised relative paths; `cmd.exe` / PowerShell dangerous patterns included alongside POSIX ones.

**Tests.**

| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | each rule: TP and TN snippet; CWE/severity present; pack with unknown CWE rejected |
| proptest | auditing an unchanged file yields no new findings; findings are within diff hunks and spans are in bounds |
| fuzz | `fuzz_rules_pack_yaml` (arbitrary YAML), `fuzz_rules_audit_command` (arbitrary command strings) |
| security | known-bad corpus (command injection, traversal, hard-coded key, CUDA race) all detected; known-benign corpus has zero findings (false-positive budget recorded, not assumed); S1 timeout never removes a finding |
| load | 1,000-line diff audited < 100 ms with all default packs (target) |
| scalability | pack count 10 / 100 / 500 rules vs audit latency |
| fixtures | per-rule snippets with provenance; malicious-diff corpora generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Rule Pack Parsing | function | `parse_pack` | `crates/polaris-security/crates/rules/src/pack.rs` | `valid_pack_parses`, `unknown_cwe_rejected`, `duplicate_rule_id_rejected`, `bad_pattern_reports_rule_id` |
| Diff Audit | method | `RuleEngine::audit_diff` | `crates/polaris-security/crates/rules/src/audit.rs` | `finds_command_injection_in_added_line`, `ignores_unchanged_code`, `cuda_syncthreads_in_branch_flagged`, `hardcoded_key_flagged` |
| Command Audit | method | `RuleEngine::audit_command` | `crates/polaris-security/crates/rules/src/command.rs` | `rm_rf_root_flagged`, `curl_pipe_sh_flagged`, `benign_ls_clean`, `powershell_remove_item_recurse_flagged` |
| S1 Arbitration | method | `RuleEngine::arbitrate` | `crates/polaris-security/crates/rules/src/arbitrate.rs` | `timeout_keeps_findings`, `confident_downgrade_removes_disputed_only`, `error_keeps_findings`, `undisputed_never_downgraded` |

**Risks & open questions.**
- Whether `ast-grep-core` 0.51 offers rule-YAML loading without `ast-grep-config` is **UNVERIFIED**; API to confirm at implementation. Language bindings for CUDA/C++ in ast-grep rely on tree-sitter grammars that must match code-intel's (grammar ABI: UNVERIFIED).
- The chatlog's heuristic checks (`code_diff.contains("std::thread::sleep")`) are string matching, not analysis; rules must be AST-based, and their false-positive rate is **UNVERIFIED** until the corpus is built.
- CWE ids listed above are standard identifiers (LIKELY correct from general knowledge; verify against the MITRE list when writing the pack). Typosquatting has no single CWE.
- Rules ship as data compiled into the binary; updating packs without a release is out of scope for the MVP.
- S1 arbitration of security findings depends on a working Decide model (S0-4); until then the deterministic path is the only path.

**Chatlog sources.** Msg 11 (`SecurityRule { id, cwe, severity }`, Security Knowledge Base categories: memory/concurrency incl. CUDA, command injection, filesystem/path traversal and symlink races, supply chain and secrets via entropy+masks, Aho-Corasick/AST matcher + S1 arbiter), msg 33 (hacker static anti-patterns such as blocking sleep in async, `__syncthreads()` in divergent branch), msg 31 (blast-radius command rules). Corrected: lives in the security crate (not `polaris-system-one::security`); string-contains heuristics replaced by AST/DFA rules; `todo!()` removed.

---

## 13.2 Group G2 — Kernel, runtime, models, tools

**Group conventions (apply to every crate below; not repeated per crate).**
- Safe Rust only; no `unwrap` / `expect` / `panic!` / slice indexing / `as` casts; every fallible fn returns `Result<_, <Crate>Error>` where `<Crate>Error` is a `thiserror` enum. No lint attributes anywhere (D10): the prompt.md rules are followed by discipline and review.
- Versions are copied from `@prime/tools/workspace.toml` `[[ext]]`. A crate that a sketch needs but whose `ext = [...]` list in `workspace.toml` lacks is marked `+proposed` (the parent must reconcile `workspace.toml` before `gen`). `thiserror = 2.0.21` is missing from most `ext` lists although prompt.md §4 requires it; it is implied everywhere and not repeated as `+proposed`.
- Layer rule: dependency layer < crate layer, except facade to child and sibling to sibling under the same parent. A child never depends on its parent. Cross-parent same-layer edges (for example `polaris-kernel-operators` to `polaris-runtime-context`) are forbidden; such wiring is done with sdk traits and bound at the composition root (`polaris-kernel`, L5).
- Label vocabulary: VERIFIED (checked in this session against live output, source or docs), LIKELY (consistent with context, not checked), UNVERIFIED (open; the covering spike is named). Nothing in this file has been run or tested; every test row is a plan and every number is a target to measure.
- Descriptor file paths are planned implementation paths. Generated test files go to `<crate>/tests/<logic_name_snake_case>.rs` (prompt.md §6.3) and stay `#[ignore]`d until the implementation lands.
- Hot-path marker `// hot path: zero-alloc` is used only for the closed set from the PLAN core (§0–§12) (SSE parser, bus publish/consume, GCRA check, DFA/PII scan, `§ref` hash/fold, lease lookup, log enqueue). Verification is a safe counting `#[global_allocator]` test wrapper from `polaris-tests`.

**Cross-crate flags raised by this group (for the coordinator; each is repeated in the owning crate's risks).**
1. `Tool` / `ToolSpec` / `ToolOutput` must be defined in `polaris-sdk` (not in `polaris-tools-builtin`), because `polaris-tools-math` and `polaris-tools-rtk` expose tools but do not depend on `builtin`. `workspace.toml` currently lists `trait Tool` under builtin.
2. The enum-dispatch wrappers (`SandboxChoice`, `ChatChoice`, ...) name adapter types (L2) and therefore cannot live in `polaris-sdk` (L0) as the sdk `responsibilities` text says; they live in the `polaris-kernel` composition root. The sdk keeps the traits and the `Backends` bundle.
3. For a zero-alloc bus, `Event` envelopes must be fixed-size and payload-offloaded (ids, scalars, `Cid`/`ShortRef`; strings go to CAS). This is a constraint on `polaris-sdk::Event`.
4. `polaris-system-one-watchdog` and `polaris-runtime-frames` cannot see each other; sdk should hold the read-only `FrameView` and the opaque `ScratchpadRef` that connect them.
5. `polaris-kernel-directives` must persist directive state (D11) but lists only `thiserror` in `ext`; `serde` is `+proposed` there (or the persisted form lives in sdk).
6. `polaris_expand` / `polaris_recall` are claimed by builtin (tool schemas) and context (expansion logic); the backing traits (`RefResolver`, `RecallBackend`) belong in sdk.
7. `reflect` (auto-learn project invariants, P7): RESOLVED at merge — owner is `polaris-kernel-operators` (`ReflectOp`), recorded in `workspace.toml`; the operators -> directives edge was added too.
8. `tokio` test-util (`tokio::time::pause`) is not in the root tokio feature list; it must be enabled as a dev-dependency feature for supervisor, ratelimit-like and lease tests.

---

### `polaris-kernel` — facade over the kernel children and the composition root (L5)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel` | L5 | facade | (top-level) | P1 |

**Purpose.** Re-exports the six kernel children and is the single place where concrete backends are constructed and injected (prompt.md §6.2 "single implementation point"). It resolves the prompt.md §6.2 "generics, no `dyn`" rule with one `Backends` associated-type bundle, enum dispatch for runtime-selected variants, and typestate sessions. `polaris-rpc` never depends on this crate; it serves the sdk `KernelApi` trait that `KernelHandle` implements.

**Re-exports.** `pub use polaris_kernel_bus as bus;` `polaris_kernel_directives as directives;` `polaris_kernel_supervisor as supervisor;` `polaris_kernel_reaper as reaper;` `polaris_kernel_operators as operators;` `polaris_kernel_verifier as verifier;` (workspace.toml: `pub use polaris_kernel_*`). Children never depend on this facade.

**Composition root: API sketch.**
```rust
// Backends bundle: the ONLY place where trait objects are replaced by concrete types.
pub trait Backends: Send + Sync + 'static {
    type Chat: ChatModel;          type Embed: EmbeddingProvider;
    type Decide: DecisionEngine;   type Sandbox: SandboxBackend;
    type Search: WebSearch;        type Docs: DocParser;
    type Vectors: VectorStore;     type Events: EventStore;
    type Platform: PlatformOps;    type Redact: Redactor;
}
// Enum dispatch for runtime-selected variants (chosen by $sandbox / config); no dyn on any hot path.
pub enum SandboxChoice { OpenShellLocal(OpenShellBackend), OpenShellRemote(OpenShellBackend), Contree(ContreeBackend) }
pub enum ChatChoice    { TokenFactory(TokenFactoryClient), OpenAiCompatible(TokenFactoryClient) } // local Ollama/llama.cpp for the P3 skeleton
pub enum EmbedChoice   { TokenFactory(TokenFactoryClient), Needle(NeedleBridge) }                 // Needle only after S0-6
pub enum SearchChoice  { Tavily(Tavily), Searxng(Searxng) }
// impl SandboxBackend for SandboxChoice { ... }  -- one `match` per trait method, no logic in arms.
pub struct ProdBackends;                       // impl Backends for ProdBackends { type Sandbox = SandboxChoice; ... }

// Session typestate (prompt.md §2.1): invalid transitions do not compile.
pub struct Created; pub struct Configured; pub struct Running; pub struct Paused; pub struct Archived;
pub struct Session<S> { /* SessionId, DirectiveState, frame root, _s: PhantomData<S> */ }
impl Session<Created>    { pub fn configure(self, cfg: SessionConfig) -> Result<Session<Configured>, KernelError>; }
impl Session<Configured> { pub async fn start(self) -> Result<Session<Running>, KernelError>; }
impl Session<Running>    { pub fn apply(&mut self, d: SessionDirective) -> Result<Seq, KernelError>;
                           pub async fn pause(self) -> Result<Session<Paused>, KernelError>;
                           pub async fn archive(self) -> Result<Session<Archived>, KernelError>; }
impl Session<Paused>     { pub async fn resume(self) -> Result<Session<Running>, KernelError>; }
impl Session<Archived>   { pub async fn reopen(id: SessionId, ctx: Arc<KernelCtx>) -> Result<Session<Paused>, KernelError>; } // /session resume: replay log

pub struct Kernel<B: Backends> { /* bus, supervisor, reaper, pipeline, verifier, runtime parts */ }
impl<B: Backends> Kernel<B> {
    pub async fn start(cfg: PolarisConfig, backends: B) -> Result<KernelHandle, KernelError>;
}
pub struct KernelHandle { /* cheap clone: command sender, event subscribe, shutdown trigger */ }
impl KernelApi for KernelHandle { /* sdk trait; polaris-rpc serves it */ }
pub trait Projection { fn apply(&mut self, e: &EventEnvelope); }   // generic, not dyn
```

**Startup order (each step returns `Result`; a failure unwinds already-started steps in reverse).**
1. (In the app `main`, not here) install the rustls aws-lc-rs provider and the global allocator.
2. `polaris-config` load (defaults < `~/.polaris` < `.polaris` < env < session).
3. `polaris-log` init (sinks, `Redactor`); 4. secret vault (keyring or env into sealed `SecretBox`); 5. telemetry.
6. Storage open (`events.redb`, `registry.db`, `cas/`, caches); crash-recovery check of the reaper journal (orphaned sandboxes, temp dirs, pids from a previous run are reaped before anything new starts).
7. Reaper (root `CancellationToken`) then bus (with the `EventStore` injected) then supervisor.
8. Platform ops probe (WSL2, Job Objects), then backend construction and selection (`$sandbox(auto)` result), then the security `Authority` (root Biscuit key for this session).
9. Operators pipeline + hybrid verifier + runtime parts (leases, frames, context); spawn supervised workers (`polaris-worker --mode s1` first).
10. Projections subscribe, `KernelHandle` is returned; UIs attach.

**Shutdown order (reverse, with one overall deadline, default 10 s, to be measured).** Stop accepting commands, then cancel the frame tree, drain in-flight tool calls, flush and fsync the event log, destroy sandboxes and temp dirs, stop workers (CTRL_BREAK then kill-tree), flush logs, drop secrets (zeroize). Returns a `ShutdownReport` (per-resource outcome), never `()`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Session` typestate transitions, `Kernel::start` startup/rollback order, `SandboxChoice` enum dispatch, `Kernel` shutdown report.
- [ ] Re-export all six children; add compile-only doctests naming one public item per child.
- [ ] Define `Backends`, the `*Choice` enums and `ProdBackends`; one `match` per trait method; contract test that every enum arm forwards (no logic in arms).
- [x] Implement `Session<S>` typestate and the `Archived -> Paused` reopen path by replaying `SessionConfigured`/`DirectiveApplied` and explicit lifecycle events (D11). Real CAS/redb/SQLite, exclusive live ownership, paged replay, rebuildable catalogue and committed-projection failure reporting; PHASES_STATUS §E25. The full verified command dispatcher remains a separate open task.
- [ ] Implement the orchestrator loop: command receive, verify (kernel-verifier), event-log append, projections, dispatch; `log-then-publish` ordering.
- [ ] Implement startup with reverse-order rollback and the reaper-journal orphan sweep; Windows: Ctrl+C, CTRL_BREAK, console close and logoff events via `tokio::signal::windows`.
- [ ] Wire fail-open / fail-closed policy per S1 consumer (plan matrix) at the pipeline assembly, not inside S1.
- [ ] Expose `KernelHandle: KernelApi` and keep `polaris-rpc` free of any kernel dependency.

**Tests** (facade-level; child suites live in the children).
| Category | Cases / budget |
|---|---|
| smoke | start and stop an embedded kernel with fake backends in under 1 s (target); `polaris doctor` lists reaper-journal orphans |
| unit | enum dispatch forwards every `SandboxBackend` method; illegal session transitions are compile-fail tests (`trybuild`-style, proposed) |
| stress | shutdown while 32 frames and 4 sandboxes are active: zero leaked resources reported by the reaper |
| security | command without a valid token is rejected before any event is appended |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Session typestate | struct | `Session<S>` | `crates/polaris-kernel/src/session.rs` | `created_configures_to_configured`, `paused_resumes_to_running`, `reopen_replays_directives_in_order`, `archived_cannot_apply_directive_compile_fail` |
| Kernel startup order | function | `Kernel::start` | `crates/polaris-kernel/src/start.rs` | `start_failure_rolls_back_in_reverse_order`, `orphans_from_previous_run_are_reaped_first`, `bus_receives_event_store_before_first_command`, `secrets_never_reach_log_sinks` |
| Sandbox enum dispatch | enum | `SandboxChoice` | `crates/polaris-kernel/src/backends.rs` | `each_variant_forwards_exec`, `each_variant_forwards_destroy`, `auto_selection_prefers_local_openshell`, `remote_variant_requires_mtls_config` |

**Risks & open questions.**
- VERIFIED (source of plan): a `Backends` bundle plus enum dispatch is the approved ADR; the extra enum arms are boilerplate that must stay logic-free. Compile-time cost of a large associated-type bundle is UNVERIFIED (measure `cargo check` time at P3).
- VERIFIED (§E25): `Session<Archived> -> Paused` reopens through ordered durable replay; archived objects release the live lease immediately. Running recovery also commits a pause before returning a mutable owner.
- VERIFIED (current layer boundary): the kernel implements session/registry projections; `polaris-ui-model` builds view models above L5 from the same events. The kernel does not depend on the L6 consumer.
- VERIFIED: layer rule forbids `polaris-sdk` (L0) from naming adapter types, so the enum wrappers live here (flag 2).

**Chatlog sources.** msg 1 (kernel as orchestrator, state machine, event bus, `Cargo.toml` members), msg 13 (EDA/CQRS, commands vs events), msg 39 (operator pipeline order). Corrected: chatlog sketches `todo!()` bodies and `Box<dyn Error>`; replaced by typed errors; "actor-like state machine" became typestate sessions.

---

### `polaris-kernel-bus` — CQRS command/event bus: bounded commands in, broadcast events out, log-then-publish

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/bus` | L1 | lib | `polaris-kernel` | P1 |

**Purpose.** Agents never mutate state directly; they send `Command`s (each carrying a capability token) into a bounded queue, and approved facts leave as immutable `EventEnvelope`s on a broadcast channel that every UI and projection subscribes to. Persistence goes through an injected sdk `EventStore`, always before fan-out ("log-then-publish"), so no subscriber ever sees an event that is not durable. Backpressure is explicit: a full command queue is a typed error, never a silent drop.

**API sketch.**
```rust
pub struct BusConfig { pub command_capacity: NonZeroUsize, pub event_capacity: NonZeroUsize }
pub struct Bus<S: EventStore> { /* mpsc::Sender<Command>, broadcast::Sender<EventEnvelope>, store: S, counters */ }
pub struct CommandReceiver { /* mpsc::Receiver<Command> */ }
pub struct EventStream { /* broadcast::Receiver<EventEnvelope> + last seen Seq */ }

impl<S: EventStore> Bus<S> {
    pub fn new(cfg: BusConfig, store: S) -> Result<(Bus<S>, CommandReceiver), BusError>;
    pub fn try_send(&self, cmd: Command) -> Result<(), BusError>;      // hot path: zero-alloc; Err(Full) = backpressure
    pub async fn send(&self, cmd: Command) -> Result<(), BusError>;    // awaits capacity, cancellable
    pub async fn publish(&self, ev: Event) -> Result<Seq, BusError>;   // store.append(ev) THEN fanout; cold-ish (I/O)
    pub fn fanout(&self, env: EventEnvelope) -> FanoutOutcome;          // hot path: zero-alloc
    pub fn subscribe(&self) -> EventStream;
    pub fn stats(&self) -> BusStats;                                    // sent, rejected_full, lagged, subscribers
}
impl CommandReceiver { pub async fn recv(&mut self) -> Option<Command>; pub fn try_recv(&mut self) -> Result<Command, BusError>; } // hot path: zero-alloc
impl EventStream {
    pub async fn next(&mut self) -> Result<EventEnvelope, BusError>;    // Err(Lagged{missed, resume_from}) -> caller replays from the store
}
pub enum FanoutOutcome { Delivered { receivers: usize }, NoReceivers }  // "no receivers" is data, not an error
pub enum BusError { Full, Closed, Lagged { missed: u64, resume_from: Seq }, Store(EventStoreError), Config(&'static str) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `tokio = 1.54.0` (features `sync`, `rt`), `tracing = 0.1.44`; dev: `proptest = 1.12.0`, `loom = 0.7.2`, `hdrhistogram = 7.6.0`.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/bus_{command_backpressure,log_then_publish,lag_recovery,zero_alloc_hot_path}.json` (red = unresolved API, then green).
- [x] sdk `Event` is fixed-shape (ids, scalars, `Cid`; large text goes to CAS), so fan-out clones never allocate payloads (flag 3) — VERIFIED by the dhat test below.
- [x] `BusConfig` uses `NonZeroUsize` capacities (zero is unrepresentable); `Bus::new` rejects capacities above tokio's panic limits with `BusError::Config`.
- [x] Implement `publish` (sync): an ordering `Mutex` around append-to-`EventStore` -> `Seq` -> broadcast; on store error nothing is broadcast (`store_error_suppresses_fanout`). A planted fan-out-before-append mutation was caught by 6 tests.
- [x] Lag handling: `RecvError::Lagged(n)` becomes `BusError::Lagged { missed, resume_from }`; the first retained event is stashed so nothing after the gap is lost; `resume_from_store_replays_missed_events` replays the gap from the store.
- [x] Counting-allocator test (dhat): `try_send`, `try_recv`, `publish` and `try_next` perform **zero allocations after warm-up** — VERIFIED for tokio mpsc + broadcast (block reuse), so the ring fallback below is not needed.
- [x] ~~Fallback ring of atomic slots~~ — not needed (previous item).
- [ ] Loom model of publish/subscribe/close ordering — open: tokio's channels are not loom-instrumented; needs a loom-friendly model of the ordering lock (or `tokio`'s own loom cfg).
- [x] `tracing` with structured fields only (`event subscriber lagged`: missed, resume_from); counters `rejected_full` and `lagged` exposed via `Bus::stats()` for telemetry.
- [ ] Fuzz `fuzz_bus_ops` (random op sequences vs a model), load/stress (hdrhistogram) — open. Miri: 13 tests clean (§E15).

**Tests.**
| Category | Cases / budgets (targets, not measured) |
|---|---|
| unit | full queue returns `Full`; closed bus returns `Closed`; publish assigns strictly increasing `Seq`; store failure suppresses fan-out |
| proptest | any interleaving of sends keeps per-sender FIFO order; `Seq` is gap-free for N publishes; lagged receiver can always resume from `resume_from` |
| loom | 2 publishers + 1 subscriber + close: no event observed before append, no panic, no deadlock (bounded model) |
| kani | `BusConfig` validation and sequence-counter increment: no overflow for any `u64` start (checked arithmetic) |
| miri | pure-logic parts (config, stats, envelope encode) |
| fuzz | `fuzz_bus_ops` (random op sequences over send/publish/subscribe/close; assert invariants) |
| load | 1 M commands/s sustained into a no-op consumer; fan-out to 8 subscribers p99 < 50 microseconds; hdrhistogram recorded |
| stress | slow subscriber (1 event/s) while publishing 100 k events/s: publisher never blocks, subscriber gets `Lagged`, resyncs |
| spike | `tests/spike/polaris-spike-bus`: tokio mpsc/broadcast vs fixed atomic ring under the counting allocator (no S0-x number assigned) |
| fixtures | recorded `Command`/`Event` sequences from a scripted session (synthetic, labelled as such) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Command backpressure | function | `Bus::try_send` | `crates/polaris-kernel/crates/bus/src/bus.rs` | `try_send_returns_full_at_capacity`, `try_send_after_close_returns_closed`, `try_send_never_blocks`, `try_send_allocates_nothing_after_warmup` |
| Log-then-publish | async_function | `Bus::publish` | `crates/polaris-kernel/crates/bus/src/publish.rs` | `publish_appends_before_fanout`, `store_error_suppresses_fanout`, `seq_is_strictly_increasing`, `no_receivers_is_not_an_error` |
| Lag recovery | struct | `EventStream` | `crates/polaris-kernel/crates/bus/src/stream.rs` | `lagged_reports_missed_and_resume_seq`, `resume_from_store_replays_missed_events`, `closed_stream_returns_closed` |

**Risks & open questions.**
- UNVERIFIED: tokio's bounded `mpsc` allocates list blocks lazily and recycles them; whether steady state is truly allocation-free must be measured with the counting allocator (spike `polaris-spike-bus`). `broadcast` pre-allocates its ring, but cloning an `EventEnvelope` that holds heap data would allocate, hence flag 3.
- VERIFIED (plan): hot path set includes bus publish/consume; everything else here (store append) is I/O and exempt.
- LIKELY: a lagging UI is the common failure (TUI redraw stall); resync-from-log is the mitigation, not a bigger buffer.
- UNVERIFIED: whether the command queue should be priority-aware (InteractiveUser vs Background, as in ratelimit). Deferred; a single FIFO is the P1 scope.

**Chatlog sources.** msg 13 (`PolarisEventBus`: mpsc commands, broadcast events, command/event lists). Corrected: chatlog `publish_event` swallowed the send result with `let _ =`; here "no receivers" is an explicit outcome. Chatlog events carried `String` fields (allocating); replaced by offloaded payloads. Chatlog put `bus.rs` in kernel; it is its own L1 child here.

---

### `polaris-kernel-directives` — recursive-descent grammar for `$directives`, `/commands` and `@mentions`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/directives` | L1 | lib | `polaris-kernel` | P1 |

**Purpose.** Turns one line (or multi-line message) of user input into typed values: session-scoped `SessionDirective`s, one optional `SlashCommand`, and `Mention`s with byte spans, plus the residual prose. The parser is a hand-written recursive-descent parser that is total (never panics, linear time, bounded nesting), works on `&str` with checked `get(..)` slicing only, and keeps unknown `$name(...)` as `Custom` metadata. Per D11, directives apply from the point they are seen, are persisted with the session as `DirectiveApplied` events referencing canonical text in CAS, and are restored by `/session resume`; new sessions start from configuration defaults committed in `SessionConfigured` (§E25).

**Grammar (EBNF, ISO-14977 style; keywords are case-insensitive, paths and quoted strings are case-preserving; `ws` = space or tab).**
```ebnf
input         = [ ws ] , ( command_line | prose ) ;
(* a slash command is only recognised when "/" is the first non-ws character of the whole input *)
command_line  = "/" , cmd_name , { ws , cmd_arg } , [ ws ] ;
cmd_name      = lower , { lower | digit | "-" } ;
cmd_arg       = quoted | bare_word ;
bare_word     = bare_char , { bare_char } ;              (* bare_char: any char except ws and quotes *)
prose         = { prose_item } ;
prose_item    = code_fence | code_span | directive | mention | attachment | text_run ;
code_fence    = "```" , { any_char - "```" } , [ "```" ] ;  (* directives and mentions inside are ignored *)
code_span     = "`" , { any_char - "`" } , [ "`" ] ;
attachment    = ? an input segment flagged as attachment by the UI; never parsed ? ;
text_run      = { any_char - ( "$" | "@" | "`" ) } | "$" | "@" ;   (* a "$" or "@" that starts no valid directive/mention is plain text *)

directive     = "$" , dname , [ "(" , [ ws ] , [ arglist ] , [ ws ] , ")" ] ;   (* bare "$name" means "on" *)
dname         = lower , { lower | digit | "-" | "_" } ;
arglist       = arg , { [ ws ] , "," , [ ws ] , arg } ;
arg           = kv | model_entry | nested | quoted | atom ;
kv            = ident , [ ws ] , "=" , [ ws ] , value ;      (* tried first: an ident immediately followed by "=" *)
value         = quoted | nested | atom ;
model_entry   = model_name , "(" , [ ws ] , range , [ ws ] , ")" ;   (* nano(1-3)   "nvidia/Nemotron-3_5-Lightning"(2-high) *)
model_name    = quoted | atom ;
range         = effort , [ [ ws ] , "-" , [ ws ] , effort ] ;  (* one effort means lo = hi; lo <= hi required *)
effort        = level_name | "1" | "2" | "3" | "4" | "5" ;
level_name    = "low" | "medium" | "high" | "xhigh" | "max" ;  (* low=1 medium=2 high=3 xhigh=4 max=5 *)
nested        = ident , "(" , [ ws ] , [ arglist ] , [ ws ] , ")" ;   (* depth limit applies *)
atom          = atom_char , { atom_char } ;                  (* atom_char: any char except ws , "," "(" ")" and quotes; URLs, paths, model ids are atoms *)
ident         = ( lower | "_" ) , { lower | digit | "_" | "-" } ;
quoted        = '"' , { qchar } , '"' | "'" , { qchar } , "'" ;
qchar         = ? any char except the closing quote ? | "\\" | "\" , '"' | "\" , "'" ;   (* only these 3 escapes; any other backslash is literal, so "C:\dir\file" works *)

mention       = "@" , ( typed_mention | path_mention ) ;     (* must follow start, ws, "(" or "[" ; "dev@company.com" is NOT a mention *)
typed_mention = kind , "(" , [ ws ] , path_arg , [ ws ] , ")" | kind ;   (* bare @scans / @xlsx = kind marker without path *)
kind          = "file" | "docx" | "pdf" | "xlsx" | "img" | "scans" ;
path_mention  = quoted | path_token ;                        (* @src/lib.rs    @"dir with space/a.rs" *)
path_arg      = quoted | path_atom ;                         (* path_atom: like atom but stops only at ws and ")" *)
path_token    = path_char , { path_char } ;                  (* path_char: any char except ws and quotes *)
lower         = "a".."z" ;  digit = "0".."9" ;
```

**Directive catalog (typed targets; a bare name is `on`; `(off)` turns it off; the last occurrence wins).**
| Directive | Accepted forms | Typed value |
|---|---|---|
| `$websearch` `$subagents` `$links` `$skills` `$mcp` `$rag` `$browser` `$needle` `$nebius-datalab` `$nebius-speculative` | bare, `(on)`, `(off)`; list-valued ones also `(a, b, "c d")` | `Toggle`, plus `Vec<String>` for the list-valued ones |
| `$subagent-models` | `(m(e1-e2), m2(e), ...)`; effort names or 1..5; quoted names | `Vec<ModelEffort { model, lo, hi }>` |
| `$nvidia` | bare (= auto), `(auto)`, `(list)`, `(off)` | `NvidiaMode` |
| `$nvidia-docs` | `(sync)` | `NvidiaDocsArg::Sync` |
| `$sandbox` (alias `$nebius-sandbox`, normalised) | `(auto\|openshell\|microvm\|contree\|remote)` | `SandboxMode` |
| `$nebius-branching` | `(on, max_branches=N)` | `{ on: bool, max_branches: Option<u8> }` |
| `$concurrency` | `(web=15, inference=8, io=128)`; pools `inference web docs disk s1 sandbox`, `io` aliases `disk` | `Vec<(Pool, u32)>` |
| `$ratelimit` | `(rpm=N, tpm=N)`; positional `(600, 200000)` accepted | `{ rpm: Option<u32>, tpm: Option<u32> }` |
| `$budget` | `(usd=5)` or `(tokens=2000000)` | `Budget::{ Usd(String), Tokens(u64) }` (USD kept as a decimal string, parsed by telemetry) |
| `$storage-level` | `(global)` / `(isolated)` | `StorageLevel` |
| `$env` | `(nix)` / `(docker)` / `(host)` | `EnvKind` |
| `$ci` | `(dagger)` | `CiBackend` |
| any other `$name[(args)]` | any `arglist` | `Custom { name, args }`, kept as session metadata, never an error |

**Slash catalog.** Core: `/model [id]`, `/usage`, `/stats`, `/status`, `/quit` (alias `/exit`), `/plan`, `/goal [text]`, `/diff`, `/rollback [step]`, `/undo`, `/sessions`, `/session resume <id>`, `/session replay <id> [step]`, `/settings`, `/help [topic]`, `/clear`, `/providers`, `/onboarding`. NVIDIA: `/nv-profile`, `/nv-verify`. Nebius: `/nebius-status`, `/contree-branches`, `/datalab-sync`, `/datalab-finetune`. Resources: `/concurrency-stats`, `/cache stats|purge`, `/quota`. Research: `/research <goal>`, `/research status|report|resume` (the three sub-words are reserved; quote a goal that equals one of them). Context: `/context-map`, `/context-expand <ref>`, `/parallel-dag`. Browser: `/screenshot`, `/browser goto <url>|tree`, `/ui-diff`. Security: `/hack`, `/redteam`. Search: `/search-local <query>`. CI and env: `/ci`, `/ci-run`, `/nix-flake`, `/proof-of-done`. An unknown command is `DirectiveError::UnknownCommand` (shown to the user), unlike unknown `$names`.

**API sketch.**
```rust
pub struct ParseLimits { pub max_input_bytes: usize /* default 64 KiB */, pub max_depth: u8 /* default 8 */, pub max_args: u16 /* default 64 */ }
pub fn parse(input: &str, limits: &ParseLimits) -> Result<ParsedInput, DirectiveError>;
pub fn parse_segments(segs: &[Segment<'_>], limits: &ParseLimits) -> Result<ParsedInput, DirectiveError>; // Segment::{ Typed(&str), Attachment(&str) }

pub struct ParsedInput { pub command: Option<SlashCommand>, pub directives: Vec<Spanned<SessionDirective>>, pub mentions: Vec<Spanned<Mention>>, pub prose: String }
pub struct Spanned<T> { pub value: T, pub span: core::ops::Range<usize> }   // byte offsets, always on char boundaries
pub enum SessionDirective { WebSearch(Toggle), Subagents(Toggle), SubagentModels(Vec<ModelEffort>), Links(ListArg), Skills(ListArg), Mcp(ListArg),
    Rag(ListArg), Nvidia(NvidiaMode), NvidiaDocs(NvidiaDocsArg), Sandbox(SandboxMode), NebiusBranching { on: bool, max_branches: Option<u8> },
    NebiusDatalab(Toggle), NebiusSpeculative(Toggle), Concurrency(Vec<(Pool, u32)>), RateLimit { rpm: Option<u32>, tpm: Option<u32> },
    Budget(Budget), StorageLevel(StorageLevel), Browser(Toggle), Needle(Toggle), Env(EnvKind), Ci(CiBackend), Custom { name: String, args: Vec<Arg> } }
pub enum Toggle { On, Off }
pub enum SlashCommand { Model(Option<String>), Usage, Stats, Status, Quit, Plan, Goal(Option<String>), Diff, Rollback(Option<u32>), Undo, Sessions,
    SessionResume(SessionId), SessionReplay { id: SessionId, step: Option<u64> }, Settings, Help(Option<String>), Clear, Providers, Onboarding,
    NvProfile, NvVerify, NebiusStatus, ContreeBranches, DatalabSync, DatalabFinetune, ConcurrencyStats, CacheStats, CachePurge, Quota,
    Research(ResearchCmd), ContextMap, ContextExpand(ShortRef), ParallelDag, Screenshot, Browser(BrowserCmd), UiDiff, Hack, Redteam,
    SearchLocal(String), Ci, CiRun, NixFlake, ProofOfDone }
pub enum Mention { File(PathArg), Docx(PathArg), Pdf(PathArg), Xlsx(Option<PathArg>), Img(Option<PathArg>), Scans(Option<PathArg>) }
pub enum ResearchCmd { Start(String), Status, Report, Resume }
pub struct ModelEffort { pub model: String, pub lo: Effort, pub hi: Effort }    // Effort comes from sdk (1..=5)
pub enum DirectiveError { TooLong, TooDeep, TooManyArgs, UnbalancedParen { at: usize }, BadEffortRange { at: usize },
    UnknownCommand(String), BadArg { directive: String, at: usize } }

// D11 persistence: a pure, deterministic, order-dependent fold over directive events.
pub struct DirectiveState { /* map name -> latest SessionDirective */ }
impl DirectiveState { pub fn apply(&mut self, d: SessionDirective); pub fn from_defaults(cfg: &PolarisConfig) -> DirectiveState; }
pub fn render(d: &SessionDirective) -> String;   // canonical text form for /status, /settings and round-trip tests
```

**Dependencies.** Internal: `polaris-sdk` (newtypes `Effort`, `SessionId`, `ShortRef`). External: `thiserror = 2.0.21`; dev: `proptest = 1.12.0`. **No serde (flag 5 resolved, P1):** the persisted form of a directive is its canonical text (`render`), stored in CAS behind sdk's fixed-size `Event::DirectiveApplied { directive: Cid }`; resume re-parses it, and `parse(render(d)) == d` is property-tested and fuzzed.

**TODO.**
- [x] Write JSON descriptors (prompt.md §6.3): `specs/descriptors/directives_{input_parser,subagent_model_ranges,slash_command_table,mention_parser,directive_persistence,directive_rendering}.json` (tests first; red = unresolved API).
- [x] Implement the byte cursor (`peek`, `bump`, `get(a..b)` with char-boundary checks, no indexing) and enforce the limits (input size, depth, arg count) before recursing.
- [x] Implement `quoted` with the 3-escape rule and a Windows-path test (`"C:\Users\me\a b.txt"` round-trips).
- [x] Implement `arglist` / `kv` / `model_entry` / `nested` (unified as `Arg::Call`; whitespace-separated words join into one atom, so `nano(1 - 3)` works); mixed forms such as `nano(1-medium)`.
- [x] Implement all typed directives from the catalog; unknown `$name(...)` becomes `Custom`; alias `$nebius-sandbox` is normalised to `Sandbox`.
- [x] Implement code span / fence / attachment skipping and the "mention must follow start, whitespace, `(` or `[`" rule (emails and `foo@bar` stay prose). The same boundary rule applies to `$` (P1 decision: `a$b` stays prose).
- [x] Implement the slash table (including `/quit` = `/exit`, `/session resume|replay`, `/cache stats|purge`, `/research ...`, `/browser goto|tree`); `/name` followed by a non-space (`/usr/bin`) is prose.
- [x] Implement `DirectiveState::apply` (returns the replaced value) and `with_defaults` (takes the defaults as values: `polaris-config` is a same-layer crate, so `PolarisConfig` cannot be a parameter). Emitting `DirectiveApplied` per applied directive is the kernel's job (P1-X-1).
- [x] Implement `render`; canonical forms asserted explicitly (no insta needed) and `parse(render(d)) == d` proven by proptest (5,000 cases) and the fuzz target.
- [x] Windows specifics: accept `\` in atoms and paths, make sure drive letters (`C:`) are never mistaken for `kv`/`range`, handle CRLF in multi-line input, keep `$` followed by digits (`$5`, `$100`) as prose.
- [x] Miri over all 39 tests (no UB); fuzz target `directives_grammar` (parse + render→parse roundtrip; 137,091 runs / 61 s, no crash); Kani: cursor harnesses VERIFIED (`cursor_bump_stays_on_char_boundaries`, `cursor_take_while_stays_on_char_boundaries`). A whole-`parse` Kani harness is infeasible here (CBMC > 3.4 GB on 3-byte inputs, PLAN W24).
- [ ] Load test (1 MiB of mixed prose at >= 100 MB/s, linear scaling) and `tests/fixtures/directives/*.txt` corpus — not done yet (P10 matrix).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | every catalog row parses; `$websearch` = on; `$websearch(off)`; `$subagent-models(nano(1-2), "nvidia/nemotron-3-super-120b-a12b"(high-max))`; `hi < lo` is `BadEffortRange`; `/exit` = `/quit` |
| proptest | `parse` never panics on arbitrary Unicode; every `Spanned.span` is in bounds and on char boundaries; `parse(render(d)) == d`; nesting beyond the limit yields `TooDeep`; prose bytes outside tokens are preserved |
| fuzz | `fuzz_directive_parse` (bytes via `from_utf8_lossy`), `fuzz_directive_roundtrip`, `fuzz_slash_table` |
| kani | bounded parse of ASCII input up to 12 bytes: no panic, no arithmetic overflow, span invariant holds |
| miri | whole crate (pure) |
| load | parse 1 MiB of mixed prose at >= 100 MB/s (target), linear scaling checked at 1x/2x/4x |
| security | 64 KiB of `(` and `$a(` repeated is rejected in O(n) with `TooDeep`; stack use bounded by the depth limit; no regex is used, so no ReDoS surface |
| fixtures | `tests/fixtures/directives/*.txt`: valid, invalid and chatlog-derived examples (model names are opaque here and validated by `polaris-models`) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Input parser | function | `parse` | `crates/polaris-kernel/crates/directives/src/parser.rs` | `bare_directive_means_on`, `directive_inside_code_span_is_ignored`, `unknown_directive_kept_as_custom`, `email_is_not_a_mention`, `unbalanced_paren_reports_offset`, `parse_never_panics_on_arbitrary_input` |
| Subagent model ranges | function | `parse_model_efforts` | `crates/polaris-kernel/crates/directives/src/effort.rs` | `named_and_numeric_efforts_are_equal`, `single_effort_means_lo_equals_hi`, `reversed_range_is_rejected`, `nested_parens_do_not_leak_between_entries`, `quoted_model_id_with_slash_parses` |
| Slash command table | enum | `SlashCommand` | `crates/polaris-kernel/crates/directives/src/slash.rs` | `exit_aliases_quit`, `research_subwords_are_reserved`, `session_resume_requires_id`, `unknown_command_is_an_error`, `quoted_goal_equal_to_status_starts_research` |
| Directive persistence | struct | `DirectiveState` | `crates/polaris-kernel/crates/directives/src/state.rs` | `last_occurrence_wins`, `fold_is_deterministic_for_same_event_order`, `new_session_starts_from_config_defaults`, `custom_directive_survives_resume` |

**Risks & open questions.**
- VERIFIED (SDK source and §E25): `polaris-sdk::contracts::Event::DirectiveApplied` references canonical directive text in CAS; `SessionConfigured` binds immutable per-session defaults. The kernel owns ordered replay and the last-wins fold.
- UNVERIFIED: exact argument shapes of `$ratelimit(rpm,tpm)` and `$budget(usd/tokens)` (the plan gives only the names). The grammar accepts `key=value` and positional numbers; the typed layer can narrow later without changing the grammar.
- LIKELY: `@scans` and bare `@xlsx` / `@img` semantics are underspecified (chatlog msg 21 shows `@docx(path)` and `@pdf(path)` only); the grammar allows a bare kind marker and the ingest crate decides the meaning.
- VERIFIED (read in chatlog msg 1): the chatlog parser used `&input[1..]`, fixed offsets (`content[16..]`), `starts_with("subagent-models(")` and `todo!()`; all are replaced by the cursor above.
- No spike covers this crate. The main risk is parser ambiguity (`kv` vs a URL containing `=`), mitigated by "ident immediately followed by `=`" plus fuzzing.

**Chatlog sources.** msg 0 (the original idea: bare = on, `$subagent-models(modelA(effortfrom-effortto), ...)`, `$links`, `skills`, `mcp`, `rag`, `/model /usage /stats /quit(exit) /plan /goal`), msg 1 (section 5 tables, `SessionDirective` / `SlashCommand` enums, `/diff`, `/rollback`), msg 3 (effort 1..5 mapping that feeds `$subagent-models`), msg 17 (`$concurrency`, `/concurrency-stats`), msg 21 (`@docx/@pdf/@xlsx/@img/@scans`, `/research ...`), msg 23 (`/context-map`, `/context-expand`, `/parallel-dag`). Corrected: model names are no longer the stale `nemotron-70b/340b`; effort names and numbers are both accepted; unknown names keep custom metadata (demo convention) instead of failing.

---

### `polaris-kernel-supervisor` — OTP-style supervision of tokio tasks and worker processes (D3 hybrid)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/supervisor` | L2 | lib | `polaris-kernel` | P1 |

**Purpose.** Implements the approved D3 split: Rust actors (tokio tasks) are restarted in-process, one-for-one, inside a restart-intensity limit; native and FFI work (ONNX Runtime, PDF/OCR, the Needle engine) runs in `polaris-worker` child processes, one OS process per mode (`s1`, `ingest`, `nano`), restarted by the same policy. Tokio already isolates task panics (`JoinError::is_panic`), so no `catch_unwind` wrapper is used on tasks; segfaults and aborts in native code cannot be caught in-process at all, which is why those workloads are processes. Failed work is routed to a bounded dead-letter queue and surfaced to the UI as an event.

**API sketch.**
```rust
pub struct RestartIntensity { pub max_restarts: u32, pub window: core::time::Duration }       // default 3 per 10 s (chatlog value, configurable)
pub enum RestartPolicy { Permanent, Transient, Temporary }                                    // OTP semantics
pub struct TaskSpec { pub name: ChildName, pub policy: RestartPolicy, pub intensity: RestartIntensity, pub backoff: BackoffSpec }
pub struct WorkerSpec { pub mode: WorkerMode, pub exe: std::path::PathBuf, pub args: Vec<String>, pub intensity: RestartIntensity,
                        pub health_interval: core::time::Duration, pub memory_limit_mb: Option<u32> }
pub enum WorkerMode { S1, Ingest, Nano }

pub struct Supervisor<P: PlatformOps> { /* children table, bus handle, telemetry sink, DLQ */ }
impl<P: PlatformOps> Supervisor<P> {
    pub fn new(platform: P, bus: BusHandle, root: CancellationToken) -> Supervisor<P>;
    pub fn spawn_task<F, Fut>(&self, spec: TaskSpec, make: F) -> Result<ChildId, SupervisorError>
        where F: Fn(CancellationToken) -> Fut + Send + Sync + 'static, Fut: Future<Output = Result<(), TaskError>> + Send + 'static;
    pub async fn spawn_worker<H: HealthProbe>(&self, spec: WorkerSpec, probe: H) -> Result<WorkerHandle, SupervisorError>;
    pub async fn stop(&self, id: ChildId, grace: core::time::Duration) -> Result<StopOutcome, SupervisorError>;
    pub fn snapshot(&self) -> SupervisorSnapshot;                                              // for /stats, doctor
}
pub trait HealthProbe: Send + Sync { fn check(&self) -> impl Future<Output = Result<Health, HealthError>> + Send; } // implemented with tonic at the composition root
pub enum ExitClass { Clean, Error(i32), Crash(u32) /* NTSTATUS-style code on Windows */, Killed, HealthTimeout }

pub enum BreakerState { Closed, Open { until: Instant }, HalfOpen }
pub struct CircuitBreaker { /* failures, window, cooldown */ }
impl CircuitBreaker {
    pub fn new(max_failures: u32, window: core::time::Duration, cooldown: core::time::Duration) -> CircuitBreaker;
    pub fn allow(&mut self, now: Instant) -> bool;
    pub fn on_success(&mut self, now: Instant); pub fn on_failure(&mut self, now: Instant) -> BreakerState;
}
pub struct DeadLetterQueue<T> { /* bounded ring, oldest dropped with a counter */ }
pub struct DeadLetter<T> { pub item: T, pub child: ChildName, pub reason: ExitClass, pub attempts: u32 }
pub enum SupervisorError { UnknownChild, SpawnFailed(String), IntensityExceeded { child: ChildName }, Platform(PlatformError), ShuttingDown }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-log`, `polaris-telemetry`, `polaris-kernel-bus`. External: `tokio = 1.54.0` (+ dev feature `test-util`, flag 8), `tokio-util = 0.7.20`, `backon = 1.6.0`; dev: `proptest = 1.12.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: restart-intensity accounting, `CircuitBreaker` state machine, task supervision loop, worker process supervision, DLQ routing.
- [x] Implement `CircuitBreaker` as a pure state machine over an injected `Instant` (fake clock friendly); no wall-clock calls inside. Evidence: PHASES_STATUS §E22.
- [x] Implement `spawn_task`: run the factory under a `JoinHandle`, classify `JoinError` (panic vs cancel), apply `RestartPolicy`, backoff with jitter via `backon` (not the chatlog's fixed 50 ms sleep), cancel via a child token. Evidence: PHASES_STATUS §E22.
- [ ] Implement restart intensity: more than `max_restarts` inside `window` trips the breaker, stops restarting, sends the item to the DLQ, publishes a `ChildFailed` event for the UI notice.
- [ ] Implement `spawn_worker` over the sdk `PlatformOps` trait (Job Object tree on Windows, kill-on-drop); classify exit codes (`0xC0000005`-style access violations as `Crash`).
- [ ] Implement the health loop (`HealthProbe`) with timeouts: a hung worker is killed and restarted; the in-flight request fails with a typed error so the S1 client can apply fail-open or fail-closed.
- [ ] Implement graceful stop: cancel token, wait `grace`, then `PlatformOps` terminate (CTRL_BREAK where available, else kill-tree).
- [x] Implement `SupervisorSnapshot` for `/stats` and `doctor` (restart counts, breaker state, DLQ length). Includes dropped letters and lost event publications; PHASES_STATUS §E22.
- [ ] Windows specifics: worker control channel is a named pipe created by the composition root, not here; verify child cleanup when the parent is killed by Task Manager (Job Object `KILL_ON_JOB_CLOSE`).
- [ ] Add the crash-loop stress harness (worker that exits immediately, worker that crashes after N ms, worker that hangs).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | breaker: Closed to Open after N failures in window, Open to HalfOpen after cooldown, HalfOpen success closes; intensity window slides; `Temporary` child is never restarted |
| proptest | breaker never goes Closed to HalfOpen directly; restart count never exceeds `max_restarts` per window for any failure timestamps; DLQ length bounded by capacity |
| kani | breaker counters: no overflow for any `u32` failure sequence up to a bound |
| loom | not applicable (single supervising task owns state) |
| smoke | spawn a task that panics once: it is restarted and the panic never reaches the caller; spawn a trivial worker and stop it |
| stress | 200 consecutive worker kills: zero leaked processes (checked via the process table), bounded restart delay growth, no unbounded memory growth |
| load | restart latency of a trivially panicking task p99 under 5 ms (the chatlog "<1 ms" is a target to measure, not a fact) |
| security | worker spawned with a scrubbed environment (no `NEBIUS_API_KEY`); `WorkerSpec.exe` must be an absolute path (no search-path hijack) |
| fixtures | tiny test worker binary built in the `polaris-tests` crate: modes `exit0`, `exit1`, `crash`, `hang`, `leak-child` |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Circuit breaker | struct | `CircuitBreaker` | `crates/polaris-kernel/crates/supervisor/src/breaker.rs` | `opens_after_n_failures_in_window`, `half_open_after_cooldown`, `half_open_success_closes`, `half_open_failure_reopens`, `old_failures_age_out_of_window` |
| Task supervision | async_function | `Supervisor::spawn_task` | `crates/polaris-kernel/crates/supervisor/src/task.rs` | `panicking_task_is_restarted`, `intensity_exceeded_routes_to_dlq`, `temporary_child_not_restarted`, `cancel_token_stops_child`, `panic_does_not_poison_supervisor` |
| Worker supervision | async_function | `Supervisor::spawn_worker` | `crates/polaris-kernel/crates/supervisor/src/worker.rs` | `crashed_worker_restarts_with_backoff`, `hung_worker_killed_after_health_timeout`, `crash_loop_trips_breaker`, `stop_kills_whole_process_tree` |

**Risks & open questions.**
- VERIFIED (PLAN core, D3): `panic = "unwind"` is required for task isolation; the chatlog claim that `catch_unwind` catches OCR/PDF segfaults is false and is corrected by the process split. A panic inside `Drop` during unwinding still aborts the process (LIKELY, Rust semantics).
- UNVERIFIED: restart latency "<1 ms" (chatlog) is unmeasured; it also ignores backoff, which this design adds deliberately.
- UNVERIFIED: Windows crash exit codes seen from `process-wrap` (NTSTATUS values) need confirmation on this machine in P2 (`polaris-platform-process`).
- LIKELY: tests need `tokio::time::pause`, so `tokio` `test-util` must be a dev feature (flag 8).
- No spike covers it; the worker crash-loop is covered by the `stress` suite and the plan's spike list ("worker crash loops").

**Chatlog sources.** msg 21 (section 4: Erlang/OTP supervision tree, one-for-one, circuit breaker 3 in 10 s, DLQ, UI notice "PDF parser skipped corrupted page 4"; section 5.3 `TaskSupervisor::spawn_resilient`), msg 13 (UI notice via bus). Corrected: `AssertUnwindSafe(..).catch_unwind()` replaced by `JoinError::is_panic`; "Let it crash" for native code means a separate process, not a `catch_unwind`; fixed 50 ms pause replaced by jittered backoff; `break` after the breaker trips now also publishes an event instead of a comment.

---

### `polaris-kernel-reaper` — deterministic teardown: cancellation tree, kill-trees, sandbox and temp cleanup

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/reaper` | L2 | lib | `polaris-kernel` | P1 |

**Purpose.** Owns the root `CancellationToken` and a registry of everything that must be released: tasks, child processes (via `PlatformOps` kill-tree), sandboxes, temp directories and IPC segments. `shutdown_all` tears resources down in a fixed order under a deadline and returns a per-resource report; resources that survive are reported (and journaled) instead of silently ignored. `Drop` cannot be async, so `Drop` only does cheap synchronous best effort; real teardown is the explicit async path plus a crash-safe journal that the next startup sweeps.

**API sketch.**
```rust
pub struct Reaper<P: PlatformOps> { /* root token, registry, journal path, platform */ }
impl<P: PlatformOps> Reaper<P> {
    pub fn new(platform: P, journal_dir: std::path::PathBuf) -> Result<Reaper<P>, ReaperError>;
    pub fn child_token(&self) -> CancellationToken;
    pub fn register<R: Teardown + Send + 'static>(&self, kind: ResourceKind, resource: R) -> Result<ReapId, ReaperError>;
    pub fn release(&self, id: ReapId) -> Result<(), ReaperError>;                // resource finished cleanly; remove from registry and journal
    pub async fn shutdown_all(&self, deadline: core::time::Duration) -> ShutdownReport;
    pub async fn sweep_orphans(&self) -> Result<Vec<OrphanReport>, ReaperError>;  // startup: read journal left by a crashed run
    pub fn leak_report(&self) -> LeakReport;                                     // doctor
}
pub trait Teardown { fn teardown(self: Box<Self>, reason: TeardownReason) -> impl Future<Output = Result<(), TeardownError>> + Send; } // cold path: the only boxed seam, justified because resources are heterogeneous
pub enum ResourceKind { Task, Process, Sandbox, TempDir, IpcSegment, WorkerPipe }
pub enum TeardownReason { Shutdown, CtrlC, Panic, ParentDied }
pub struct ShutdownReport { pub entries: Vec<(ReapId, ResourceKind, Outcome)>, pub elapsed: core::time::Duration, pub deadline_exceeded: bool }
pub enum Outcome { Released, Forced, Failed(String), Skipped }
pub enum ReaperError { Journal(std::io::Error), AlreadyShutDown, UnknownResource }
```
Teardown order inside `shutdown_all`: (1) cancel the root token (cascades to frames, tools, workers); (2) wait a grace period for cooperative exit; (3) kill process trees (CTRL_BREAK, then Job Object terminate); (4) destroy sandboxes (registered closures call the `SandboxBackend`); (5) remove temp dirs; (6) unlink IPC segments (none until S0-8); (7) fsync and clear the journal.

**Dependencies.** Internal: `polaris-sdk`, `polaris-log`, `polaris-kernel-bus`. External: `tokio-util = 0.7.20`, `tempfile = 3.27.0`; `+proposed tokio = 1.54.0` (timeouts, `signal`); the journal is a plain file written atomically with `tempfile::NamedTempFile::persist`; dev: `proptest = 1.12.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: cancellation cascade, ordered teardown with deadline, journal write/replay, orphan sweep, leak report.
- [x] Implement `child_token` / root cancel; test cascade through three levels and that cancelling a child does not cancel siblings. PHASES_STATUS §E23.
- [x] Implement the registry with deterministic ordering (by `ResourceKind` priority then reverse registration order). PHASES_STATUS §E23.
- [x] Implement the crash-safe journal (`.polaris/run/reaper.journal`, one record per live resource: kind, id, pid or sandbox id, path), written atomically on register and release. v1 escapes record separators, fsyncs the temporary file, and retains unresolved locators; PHASES_STATUS §E23. Power-loss testing remains unverified.
- [ ] Implement `sweep_orphans`: on startup read the journal, check liveness (pid exists and start time matches, sandbox id still known to the gateway), reap leftovers, report each one.
- [ ] Implement `shutdown_all` with a per-step and an overall deadline; `Forced` outcomes recorded; the function always returns a report and never panics or blocks forever.
- [ ] Wire signals at the composition root (documented here): `tokio::signal::ctrl_c`, and on Windows `ctrl_break`, `ctrl_close`, `ctrl_shutdown`, `ctrl_logoff`.
- [ ] Windows specifics: temp dirs may be locked by antivirus (retry removal with bounded backoff); a Job Object with `KILL_ON_JOB_CLOSE` is the backstop when the parent dies without running any Rust code; `/dev/shm` and `cgroup.kill` apply only inside Linux sandboxes, not on the Windows host.
- [ ] Implement `Drop` for registered guards as synchronous best effort only (set cancel flag, try-remove temp path) and document that it is not the teardown path.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | order of teardown matches the list above; deadline exceeded yields `Forced` for the stragglers; double `shutdown_all` returns `AlreadyShutDown` |
| proptest | for any registration order, every registered resource appears exactly once in the report; journal replay equals live registry |
| smoke | start a child process that spawns a grandchild, call `shutdown_all`: both gone (process table check) |
| stress | 1000 temp dirs + 50 fake sandboxes + 20 processes torn down under a 10 s deadline (target); AV-lock simulation (file held open) is retried then reported, not hung |
| security | journal file contains no secrets (paths, ids and pids only); sweep never kills a pid whose start time differs from the journaled one (pid reuse) |
| fixtures | journals from a simulated crash (truncated last record, duplicated record) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Cancellation cascade | struct | `Reaper::child_token` | `crates/polaris-kernel/crates/reaper/src/token.rs` | `cancelling_root_cancels_grandchildren`, `cancelling_child_leaves_sibling_running`, `token_created_after_cancel_is_already_cancelled` |
| Ordered shutdown | async_function | `Reaper::shutdown_all` | `crates/polaris-kernel/crates/reaper/src/shutdown.rs` | `teardown_order_is_cancel_process_sandbox_temp`, `deadline_marks_stragglers_forced`, `report_lists_every_resource_once`, `second_call_is_rejected` |
| Crash-safe journal | async_function | `Reaper::sweep_orphans` | `crates/polaris-kernel/crates/reaper/src/journal.rs` | `orphan_process_with_matching_start_time_is_killed`, `reused_pid_is_not_killed`, `truncated_last_record_is_ignored`, `release_removes_journal_record` |

**Risks & open questions.**
- VERIFIED (PLAN core): Windows has no SIGTERM; graceful stop is CTRL_BREAK where available, then kill; Job Objects via `process-wrap`. UNVERIFIED on this machine: CTRL_BREAK delivery to a child in a new process group when no console is shared (spike S0-8 is about iceoryx2, not this; verify in P2 with the process crate).
- VERIFIED (chatlog correction): cancellation "in 0 ms" is cooperative; a task busy in `spawn_blocking` or native code only stops at its next await or when its process is killed.
- LIKELY: `panic::set_hook` cannot run async cleanup; it is limited to setting the cancel flag and flushing the journal.
- UNVERIFIED: iceoryx2 segment cleanup semantics on Windows (S0-8); the `IpcSegment` kind is declared now and unused until then.
- Chatlog Firecracker/TAP teardown rows are dropped: D2 uses the OpenShell `vm` driver (libkrun), torn down through the sandbox backend.

**Chatlog sources.** msg 27 (section 5 deallocation matrix and section 6.3 `ResourceReaper`: cancellation tree, process groups, shm, `tempfile::TempDir`, `secrecy`/`zeroize`). Corrected: `cgroup.kill` is Linux-sandbox-only; Firecracker rows replaced by sandbox-backend teardown; secrets zeroizing stays in `polaris-security-secrets` (not reaper); `info!` "all deallocated safely" replaced by a real report, because the chatlog log line claimed success without checking.

---

### `polaris-kernel-operators` — functional operator pipeline: `pipe` / `fork_branch` / `fold` / `assert_guard` and the kernel-side operators

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/operators` | L4 | lib | `polaris-kernel` | P3 |

**Purpose.** Request handling is a typed pipeline of small, pure, individually testable operators implementing the sdk `FunctionalOperator` trait, composed with consuming combinators. Every operator carries an explicit fail policy (fail closed or fail open with a timeout and a declared fallback), so the plan's S1 failure matrix is encoded in types and not scattered through call sites. This crate owns only the operators whose dependencies it may legally see (PII sanitiser, S1 triage, rate limit, directives); the remaining pipeline stages (context assemble, S2, tool execution, fold, proof of done) are operators defined in their owning crates and are assembled into one `Pipeline` at the composition root.

**Pipeline stage order (PLAN core; owner in brackets).** 1 SanitizePii [here]; 2 Directives [here, `+proposed` dep on `polaris-kernel-directives`]; 3 S1Triage [here]; 4 ContextAssemble [`polaris-runtime-context`]; 5 RateLimit [here]; 6 S2 call [`polaris-nebius-cloud-token-factory` via root]; 7 tool calls under leases [`polaris-runtime-leases`, `polaris-tools`]; 8 sandbox exec [backend via root]; 9 fold to CAS [`polaris-runtime-context`]; 10 verifiers [`polaris-kernel-verifier`]; 11 ProofOfDone [`polaris-runtime-guards`]; 12 events to UIs [bus].

**API sketch.**
```rust
// sdk trait (shape assumed; confirm against polaris-sdk at implementation):
// pub trait FunctionalOperator<In>: Send + Sync { type Out; type Err;
//     fn name(&self) -> &'static str;
//     fn execute(&self, input: In) -> impl Future<Output = Result<Self::Out, Self::Err>> + Send; }

pub struct Pipeline<T> { /* value: T, stage trace: Vec<StageRecord> (cold) */ }
impl<T> Pipeline<T> {
    pub fn new(value: T) -> Pipeline<T>;
    pub async fn pipe<Op: FunctionalOperator<T>>(self, op: &Op) -> Result<Pipeline<Op::Out>, PipelineError<Op::Err>>;
    pub async fn fork_branch<O, E, P, A, B>(self, pred: P, fast: &A, deep: &B) -> Result<Pipeline<O>, PipelineError<E>>
        where P: Fn(&T) -> bool, A: FunctionalOperator<T, Out = O, Err = E>, B: FunctionalOperator<T, Out = O, Err = E>;
    pub async fn fold<Op, I>(self, op: &Op, items: I) -> Result<Pipeline<Op::Out>, PipelineError<Op::Err>> where I: IntoIterator, Op: FoldOperator<T, I::Item>;
    pub async fn assert_guard<G: Guard<T>>(self, guard: &G) -> Result<Pipeline<T>, PipelineError<G::Err>>;
    pub fn into_inner(self) -> T;                      // named into_inner on purpose: the chatlog `unwrap` name is banned
}
pub enum PipelineError<E> { Stage { stage: &'static str, source: E }, Cancelled { stage: &'static str }, Shed { stage: &'static str } }

pub enum FailPolicy { Closed, Open { timeout: core::time::Duration } }
pub trait Fallback<In, Out> { fn fallback(input: &In) -> Out; }          // used only by fail-open wrappers
pub struct OpenOp<Op, Fb> { /* inner op, timeout, fallback */ }          // on timeout or error returns Fb::fallback(..) and records a `FailedOpen` event
pub struct ClosedOp<Op> { /* inner op */ }                                // on timeout or error returns the error; request does not proceed

pub struct SanitizePiiOp<R: Redactor> { /* PiiVault handle */ }          // Closed: DFA + entropy only if S1 unavailable; never sends unmasked text
pub struct DirectivesOp;                                                  // parse $ / @ ; applies SessionDirectives to the session
pub struct S1TriageOp<D: DecisionEngine> { /* router */ }                // Open: on timeout default route = S2 with default effort
pub struct RateLimitOp { /* DualLimiter, Priority */ }                    // acquires permit; background work may be Shed near limits
pub struct ReflectOp<D: DecisionEngine> { /* P7: auto-learn project invariants */ }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-security-pii`, `polaris-system-one-router`, `polaris-ratelimit`; `+proposed polaris-kernel-directives` (L1, legal edge). External: `tokio = 1.54.0` (timeouts); dev: `proptest = 1.12.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Pipeline::pipe`, `fork_branch`, `OpenOp`/`ClosedOp` fail policies, `SanitizePiiOp`, `S1TriageOp`.
- [ ] Implement `Pipeline<T>` combinators with generics only (no `BoxFuture`, no `dyn`); the chatlog `FunctionalOperator` returned `Pin<Box<dyn Future>>`, which breaks prompt.md §4.
- [ ] Implement `ClosedOp` / `OpenOp` with `tokio::time::timeout`; `OpenOp` publishes a `FailedOpen` telemetry event so silent degradation is visible in `/stats`.
- [ ] Implement `SanitizePiiOp` so that the type of its output (`Masked<String>`) is the only type the S2 stage accepts (the unmasked `String` cannot reach the network by construction).
- [ ] Implement `S1TriageOp` against the router API (route plus effort 1..5), consulting `DirectiveState` for `$subagent-models` overrides.
- [ ] Implement `RateLimitOp` with RAII permit release on cancellation (cancel-safety test).
- [ ] Implement `DirectivesOp` (verified directive application emits sdk `DirectiveApplied` events through the durable session API; D11).
- [ ] Document cancel-safety for every operator (what is dropped when the future is dropped mid-stage) and test it with `tokio::time::pause` plus drop-at-await-points.
- [ ] P7: implement `ReflectOp` (S1 classifies "project rule declared"; writes to the invariants table and mirrors `.polaris/RULES.md`; flag 7).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | `pipe` threads values in order; `fork_branch` runs exactly one branch; closed op error stops the pipeline; open op falls back after the timeout and records the event |
| proptest | for pure operators, `pipe(f).pipe(g)` equals the composed function for all inputs; `fork_branch(p, a, a)` equals `pipe(a)`; stage trace length equals stages executed |
| security | PII: every secret-looking token in a generated corpus is masked before the S2 stage; if the PII op fails, no network call is attempted (fake S2 asserts zero calls) |
| stress | 10 000 concurrent pipelines with random cancellation: zero leaked rate-limit permits, no deadlock |
| load | pipeline overhead per stage under 2 microseconds with no-op operators (criterion, target; not a hot-path-closed-set item) |
| smoke | end-to-end fake pipeline (sanitize, directives, triage, fake S2) over a scripted prompt |
| fixtures | `tests/fixtures/pii/` generated at test time (fake keys, IPs, emails; never committed) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Pipeline combinators | struct | `Pipeline<T>` | `crates/polaris-kernel/crates/operators/src/pipeline.rs` | `pipe_runs_stages_in_order`, `fork_branch_runs_one_branch_only`, `stage_error_names_the_stage`, `cancel_between_stages_returns_cancelled`, `into_inner_returns_last_value` |
| Fail policies | struct | `OpenOp` / `ClosedOp` | `crates/polaris-kernel/crates/operators/src/policy.rs` | `open_op_falls_back_after_timeout`, `open_op_records_failed_open_event`, `closed_op_error_blocks_request`, `open_op_passes_success_through` |
| PII sanitise operator | struct | `SanitizePiiOp` | `crates/polaris-kernel/crates/operators/src/sanitize.rs` | `masked_output_type_is_required_by_s2_stage`, `s1_timeout_falls_back_to_dfa_entropy`, `pii_failure_blocks_cloud_call`, `counts_of_masked_items_reported` |
| S1 triage operator | struct | `S1TriageOp` | `crates/polaris-kernel/crates/operators/src/triage.rs` | `timeout_defaults_to_s2_default_effort`, `subagent_models_directive_overrides_effort_mapping`, `trivial_route_skips_s2` |

**Risks & open questions.**
- VERIFIED (read in `workspace.toml`): the operator crate may not depend on `polaris-runtime-context`, `polaris-runtime-guards` or the S2 client (cross-parent L4 or sideways edges), so only part of the pipeline lives here; the composition root assembles the rest.
- LIKELY: deeply generic combinator chains lengthen compile times and produce unreadable type names; mitigated by named stage aliases and by measuring `cargo check` time at P3.
- UNVERIFIED: the exact `FunctionalOperator` shape in sdk (async fn in trait, associated types); the sketch above is an assumption to confirm.
- UNVERIFIED: whether `ReflectOp` belongs here (flag 7); it needs only the `DecisionEngine` trait and the event store.

**Chatlog sources.** msg 39 (section 1: `Pipeline`, `pipe`, `fork_branch`, `assert_guard`, operator list Map/Filter/Branch/Fold/AssertGuard, `.pipe(SanitizePiiOp)`, `Needle3Op` branch), msg 31 (`reflect` rule distiller, here `ReflectOp`), msg 21 (PII vault in front of the cloud), msg 3 (S1 triage). Corrected: `BoxFuture`/`dyn` operators replaced by generics; method `unwrap` renamed `into_inner`; chatlog `FilterAstOp` / `Needle3Op` are owned by code-intel and the nano bridge respectively (the latter gated by S0-6); fail-open/closed encoded per the approved matrix.

---

### `polaris-kernel-verifier` — HybridMutationVerifier: Biscuit Datalog authorization, AST metric calculus, S1 semantic congruence

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-kernel/crates/verifier` | L4 | lib | `polaris-kernel` | P3 |

**Purpose.** Every proposed mutation (`ProposeDiff` command) passes three layers in strict order before it can become a `DiffApproved` event: (1) Biscuit signature chain and Datalog checks (resource, churn, TTL, no-unsafe, diff blake3), (2) deterministic AST metrics (churn, cyclomatic delta, tree-edit distance) against limits in the token facts and config, (3) S1 checks (pre-flight for test tampering and secrets, then intent congruence). Layers 1 and 2 are deterministic and run before any model call; a rejection there costs no tokens. Biscuit bounds the damage of a prompt-injected subagent to the rights in its token; it does not prevent injection and does not judge whether an in-scope edit is malicious.

**API sketch.**
```rust
pub struct Score(u16);                                        // fixed-point 0..=1000; avoids float comparisons (prompt.md: float_cmp)
pub struct VerifierLimits { pub max_churn_lines: u32 /* default 150 (chatlog) */, pub max_cyclomatic_delta: i32,
                            pub max_tree_edit_distance: u32, pub congruence_threshold: Score /* default 750 (chatlog), UNCALIBRATED */,
                            pub s1_timeout: core::time::Duration }
pub struct MutationRequest<'a> { pub agent: FrameId, pub token: &'a [u8], pub target: &'a WorkspacePath,
                                 pub old: &'a [u8], pub new: &'a [u8], pub intent: &'a str }
pub struct HybridMutationVerifier<D: DecisionEngine, C: Clock> { /* root public key, limits, decision engine, clock, authorizer */ }
impl<D: DecisionEngine, C: Clock> HybridMutationVerifier<D, C> {
    pub fn new(root: PublicKeyBytes, limits: VerifierLimits, decide: D, clock: C) -> Result<Self, VerifierError>;
    pub async fn verify(&self, req: &MutationRequest<'_>) -> Result<VerificationDecision, VerifierError>; // Err = infrastructure failure; callers treat as reject (fail closed)
}
pub enum VerificationDecision {
    Approved(ApprovedMutation),                                // witness type: only this crate can construct it
    RejectedAuthorization { violated_check: String },          // layer 1
    RejectedMetric { metric: Metric, observed: i64, limit: i64 }, // layer 2
    RejectedSemantic { score: Score, threshold: Score },       // layer 3
    NeedsApproval { reason: ApprovalReason },                  // S1 timeout or low-confidence pre-flight: ask the human, never auto-approve
}
pub enum Metric { Churn, CyclomaticDelta, TreeEditDistance, TouchesUnsafe }
pub struct ApprovedMutation { /* private fields: agent, target, diff blake3, verified-at Seq */ }   // the event-log append for DiffApproved requires it
pub enum VerifierError { Token(CapabilityError), Metrics(CodeIntelError), Decision(DecisionError), Clock }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-security-capability`, `polaris-code-intel`, `polaris-system-one-verifier`. External: none in `workspace.toml` (`ext = []`); `blake3 = 1.8.7` is reached through sdk/capability; dev: `proptest = 1.12.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `HybridMutationVerifier::verify` layer ordering, metric limit checks, token fact construction, `ApprovedMutation` witness, S1 timeout handling.
- [ ] Implement fact construction per mutation: `resource(path)`, `churn(actual)`, `time(now)`, `touches_unsafe(bool)`, `diff_blake3(hash)`; the chatlog bug (the actual churn was loaded as `churn_limit`, and time was a literal `1729000000`) is not repeated; time comes from an injected `Clock`.
- [ ] Implement layer 1 through `polaris-security-capability::authorize` (signature chain, then Datalog); no `allow if true` policy; default deny.
- [ ] Implement layer 2 via `code-intel::metrics(old, new)`; compare with limits from the token facts first and config second; thresholds are data, not literals.
- [ ] Implement layer 3: `system-one-verifier::preflight(diff)` (test tampering, secrets; fail closed) then the congruence question through `DecisionEngine`; on S1 timeout return `NeedsApproval` (UNVERIFIED policy, see risks).
- [ ] Make `ApprovedMutation` constructible only here (private constructor) and require it in the event-log append for `DiffApproved` (witness-token pattern, prompt.md §2.1).
- [ ] Return structured rejection reasons with the failing layer so the UI and the agent get a corrective message (feeds the self-correction loop).
- [ ] Add a calibration hook: record (score, human verdict) pairs for later threshold calibration (plan: thresholds start conservative and are calibrated as labelled fixtures accrue).
- [ ] Windows specifics: normalise path separators and case before comparing `resource(path)` to the token scope, and reject `..`, drive-relative and UNC forms.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | forged signature rejected at layer 1; churn 151 vs limit 150 rejected at layer 2; S1 timeout yields `NeedsApproval`; approved path returns the witness |
| proptest | an attenuated token never authorises anything its parent did not; for any metrics and limits, layer 2 verdict equals the pure comparison; layers 1 and 2 never call the decision engine (fake counts zero calls) |
| fuzz | `fuzz_verify_token_bytes` (arbitrary bytes as token; must return an error or reject, never panic or hang) |
| kani | metric comparison functions: no overflow for any `i64`/`u32` inputs (checked arithmetic) |
| security | injection corpus generated at test time (diff touching an unscoped file, diff adding `unsafe`, diff replacing an RNG call with a constant, diff deleting assertions): each is rejected or sent to approval; token replay after TTL rejected |
| load | layers 1 and 2 only: 50 000 verifications/s on this machine (target; the chatlog "5 microseconds" is unmeasured) |
| stress | 1 000 concurrent verifications with a slow S1: bounded queueing, timeouts honoured |
| fixtures | malicious-diff corpus and fake keys generated at test time (never committed); small labelled congruence set marked synthetic |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Hybrid mutation verification | async_function | `HybridMutationVerifier::verify` | `crates/polaris-kernel/crates/verifier/src/verify.rs` | `forged_token_rejected_at_layer_1`, `out_of_scope_path_rejected`, `churn_over_limit_rejected_without_s1_call`, `expired_token_rejected_using_injected_clock`, `s1_timeout_needs_approval`, `approved_returns_witness` |
| Token fact building | function | `build_facts` | `crates/polaris-kernel/crates/verifier/src/facts.rs` | `churn_fact_holds_actual_value_not_limit`, `touches_unsafe_true_when_unsafe_added`, `resource_fact_is_normalised_path`, `diff_hash_fact_matches_blake3_of_diff` |
| Approval witness | struct | `ApprovedMutation` | `crates/polaris-kernel/crates/verifier/src/witness.rs` | `witness_cannot_be_constructed_outside_crate_compile_fail`, `append_diff_approved_requires_witness`, `witness_binds_diff_hash` |

**Risks & open questions.**
- VERIFIED (PLAN core): Biscuit bounds injection damage and does not prevent it; the chatlog msg 13 claim "mathematical guarantee, the agent physically cannot leave `resource(...)`" is true only for actions routed through this verifier and says nothing about the content of an in-scope edit.
- UNVERIFIED: the biscuit-auth 6.0.0 builder/authorizer API (macros `authorizer!`, `block!`, check syntax) must be confirmed at implementation; the chatlog used the 4.0 API, which is stale. biscuit-auth 6 uses rand 0.8 internally; keys only via biscuit's API (VERIFIED, plan).
- UNVERIFIED: S1 timeout policy for the congruence layer. The plan fixes fail-closed for the safety gate and fail-open for triage/tool-pruning/watchdog/sifter but does not name congruence; this plan proposes `NeedsApproval` (closed to the human). Coordinator to confirm.
- UNVERIFIED: S1 latency on this hardware. The chatlog "under 20 ms" is false for GLiNER2.5-Decide here (vendor CPU latency about 167 ms on 48 cores, VERIFIED in the PLAN core (§0–§12); S0-4 measures it), so the sync budget for layer 3 is a target to measure; thresholds 750 / 150 are uncalibrated defaults.
- UNVERIFIED: tree-edit distance cost on large files; if too slow it is computed on changed functions only (decision in code-intel).

**Chatlog sources.** msg 13 (hybrid verifier levels 1 to 3, Biscuit tokens and attenuation, Datalog checks, `HybridMutationVerifier`, `VerificationDecision`), msg 11 (S1 diff arbiter and merge, handled in `polaris-system-one-verifier`), msg 31 (Proof-of-Done kept in `polaris-runtime-guards`). Corrected: literal timestamp and `allow if true`; `unwrap()` on authorizer calls; inverted `churn_limit` fact; `biscuit-auth = "4.0"` and `ed25519-dalek` direct dependency (keys are managed by biscuit); the "5 microseconds" figure; the "wow-effect" comparison table claims.

---

### `polaris-runtime` — facade over the agent-runtime children (frames, context, dag, leases, guards, research)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime` | L4 | facade | (top-level) | P2 |

**Purpose.** Pure re-export crate; no logic. Owns the documentation of the agent loop that the children implement together and the composition root wires: one S2 client and one S1 engine are shared by every context frame; a step is assemble context, call S2, run tool calls under leases, verify, record events; a task ends only through the Proof-of-Done gate. The children cannot see each other (only the root can wire them), so the loop is documented here, not coded here.

**Re-exports.** `pub use polaris_runtime_frames as frames;` `polaris_runtime_context as context;` `polaris_runtime_dag as dag;` `polaris_runtime_leases as leases;` `polaris_runtime_guards as guards;` `polaris_runtime_research as research;` (workspace.toml: `pub use polaris_runtime_*`). Children never depend on this facade.

**Agent loop (doc target).** (1) watchdog inspects the frame (S1, fail open); (2) context assembled with a byte-stable prefix; (3) rate-limit permit; (4) S2 streams; (5) each tool call: capability check, lease acquire, execute (sandbox for shell), fold output to CAS; (6) `ProposeDiff` goes to the verifier; (7) events published; (8) on "done" claim, Proof-of-Done runs build and tests in a sandbox; only a `Proven` result closes the task, otherwise stderr returns to S2 (bounded retries).

**TODO.**
- [ ] Re-export all six children and add compile-only doctests naming one public item per child.
- [ ] Write the agent-loop diagram and the failure-mode table (what happens on S1 timeout, S2 429, lease conflict, verifier reject, sandbox crash) into the crate docs.
- [ ] Add a `check-layers` expectation that no runtime child depends on `polaris-runtime` or on a runtime sibling not listed in `workspace.toml` (mechanical gate in `xtask`).
- [ ] Record which sdk traits each child consumes so the composition root's wiring list can be generated from the docs.

---

### `polaris-runtime-frames` — agents as context frames over one S2 client and one S1 engine

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/frames` | L4 | lib | `polaris-runtime` | P3 |

**Purpose.** An agent is not a separate model or process: it is a lightweight frame with a role, an attenuated capability token, resource quotas, a pointer to its epistemic scratchpad, a step counter and a history of diff CIDs. Child frames are created with a token that is the parent's token plus extra Datalog checks and quotas that can only shrink, never grow. The crate also holds the per-frame agent-loop step as a generic function over sdk traits, so the root can instantiate it with concrete backends.

**API sketch.**
```rust
pub struct Spawned; pub struct Active; pub struct Suspended; pub struct Finished;       // typestate (prompt.md §5.2)
pub struct AgentContextFrame<S> { /* id: FrameId, parent: Option<FrameId>, role: AgentRole, token: CapabilityToken,
                                    quota: ResourceQuota, scratchpad: ScratchpadRef, history: Vec<Cid>, steps: u32, model: ModelId, _s: PhantomData<S> */ }
pub enum AgentRole { Architect, KernelDeveloper, TestEngineer, SecurityAuditor, Researcher, Custom(RoleName) } // chatlog values + extension point; type lives in sdk
pub struct ResourceQuota { pub cpu_millis: u32, pub memory_mb: u32, pub network: NetworkPolicyRef, pub max_steps: u32,
                           pub max_depth: u8 /* chatlog default 3 */, pub token_budget: u64 }
pub struct Scope { /* path globs, symbol scopes, churn limit, ttl, allow_unsafe: bool */ }
pub struct FrameStack { /* ordered frames; the "cognitive call stack" */ }

impl AgentContextFrame<Active> {
    pub fn spawn_child(&self, role: AgentRole, scope: Scope, quota: ResourceQuota) -> Result<AgentContextFrame<Spawned>, FrameError>; // scope and quota must be subsets of the parent's
    pub fn suspend(self) -> AgentContextFrame<Suspended>;
    pub fn finish(self, outcome: FrameOutcome) -> AgentContextFrame<Finished>;
    pub fn record_step(&mut self, fp: FastFingerprint, diff: Option<Cid>) -> Result<StepNo, FrameError>; // enforces max_steps
    pub fn view(&self) -> FrameView;                           // read-only snapshot type from sdk, consumed by the watchdog
}
impl AgentContextFrame<Spawned> { pub fn activate(self, models: &ModelRegistry) -> Result<AgentContextFrame<Active>, FrameError>; } // model chosen by effort and $subagent-models

// One loop step, generic over sdk traits (no dyn); the root supplies the operators.
pub struct StepInput<'a> { pub frame: &'a mut AgentContextFrame<Active>, pub goal: &'a str }
pub async fn run_step<A, M, T, V>(input: StepInput<'_>, assemble: &A, chat: &M, tools: &T, verify: &V) -> Result<StepOutcome, FrameError>
    where A: FunctionalOperator<AssembleReq, Out = Prompt>, M: ChatModel, T: ToolRunner, V: StepVerifier;
pub enum StepOutcome { Continue, ToolResults(Vec<ToolResultRef>), ClaimedDone, Blocked(Blocker), Aborted(AbortReason) }
pub enum FrameError { QuotaExceeded(&'static str), ScopeNotSubset, DepthLimit, StepLimit, Model(ModelError), Capability(CapabilityError) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-security-capability`, `polaris-models`. External: `tokio = 1.54.0`, `serde = 1.0.229` (features `derive`); dev: `proptest = 1.12.0`. (`ToolRunner`, `StepVerifier` are sdk-level traits to add; the frame crate must not depend on `kernel-operators`, `runtime-context` or `tools`.)

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `spawn_child` attenuation, frame typestate transitions, `record_step` limits, `run_step`, model selection on `activate`.
- [ ] Implement `Scope` subset checking (paths by glob containment, churn and TTL by `min`, `allow_unsafe` can only turn off) and make `spawn_child` reject any amplification.
- [ ] Implement `ResourceQuota` subset checking and remaining-budget accounting (child quotas are carved out of the parent's remaining budget).
- [ ] Implement the typestate transitions and `FrameStack` (depth limit default 3, `max_steps` default to be calibrated).
- [ ] Implement `run_step` over sdk traits: assemble, S2 stream, tool calls, verify, record; each await point is cancel-safe and documented.
- [ ] Implement `view()` (sdk `FrameView`) with only fingerprints, role, step count and scratchpad ref so the watchdog never sees secrets (flag 4).
- [ ] Define role boundaries as data (allowed tool names and path scopes per `AgentRole`), checked before the capability check so a doc-writer frame cannot call network code tools even with a broad token.
- [ ] Add per-frame concurrency accounting hook for the shared Token Factory permit pool (the frames share one client and one rate limiter).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | child scope narrower than parent accepted; wider rejected; depth 4 rejected at default limit; finished frame cannot record steps (compile-fail) |
| proptest | for any generated parent scope and child request, the child's effective rights are a subset of the parent's; quotas never exceed the parent's remaining budget; spawn then finish returns quota to the parent |
| kani | quota arithmetic (`checked_sub`/`checked_add`) has no overflow for any `u32`/`u64` input |
| scalability | 1 to 32 frames sharing one fake S2: per-frame overhead under 1 ms and memory growth linear (targets) |
| stress | 1 000 spawn/finish cycles with random cancellation: no quota leak |
| security | prompt-injected child tries `spawn_child` with wider scope: rejected, event logged; attenuation corpus (arbitrary caveat order) never widens |
| fixtures | role/tool allow-list table as TOML in `tests/fixtures/roles/` |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Child frame attenuation | method | `AgentContextFrame::spawn_child` | `crates/polaris-runtime/crates/frames/src/spawn.rs` | `child_scope_must_be_subset`, `child_quota_carved_from_parent`, `depth_limit_enforced`, `ttl_is_min_of_parent_and_request`, `unsafe_allowance_cannot_be_enabled_by_child` |
| Frame typestate | struct | `AgentContextFrame<S>` | `crates/polaris-runtime/crates/frames/src/state.rs` | `spawned_activates_with_model`, `suspended_cannot_record_step_compile_fail`, `finish_releases_quota`, `step_limit_returns_error` |
| Agent loop step | async_function | `run_step` | `crates/polaris-runtime/crates/frames/src/step.rs` | `tool_call_requires_capability`, `claimed_done_is_not_completion`, `cancel_mid_stream_leaves_frame_consistent`, `step_fingerprint_recorded_for_watchdog` |

**Risks & open questions.**
- VERIFIED (PLAN core): children inherit attenuated Biscuit tokens and sandbox resource quotas (cpu, memory, network policy). UNVERIFIED: how per-frame quotas map onto OpenShell policy fields (owned by `polaris-openshell-policy`, P2).
- UNVERIFIED (chatlog correction): the claim "100% prefix-cache reuse across frames, TTFT < 80 ms" depends on Token Factory prefix caching (spike S0-5). It also requires that the role prompt come after the shared stable prefix; per-frame system prompts that differ at the start would defeat any prefix cache.
- LIKELY: `workspace.toml` lists `frames` with no dependency on `context`, so the scratchpad is held as an opaque `ScratchpadRef` (flag 4); alternatively a `frames -> context` sibling edge is legal and could be added.
- UNVERIFIED: default step and depth limits (chatlog: depth 3, "max 3 retries" in the verification loop); calibrate in P3.

**Chatlog sources.** msg 29 (agent = virtual context frame over one S2 and one S1; `AgentContextFrame`, `AgentRole`, cognitive call stack), msg 13 (attenuated tokens for child frames, `SpawnSubagentCommand`), msg 3 (effort routing and depth limit 3), msg 1 (ReAct loop and sub-agent swarm limits). Corrected: `String` ids and `Arc<RwLock<..>>` scratchpad replaced by newtypes and a reference; watchdog code stays in `polaris-system-one-watchdog`; "TTFT < 80 ms" demoted to a measurement target.

---

### `polaris-runtime-context` — lossless context virtualization: stable prefix, scratchpad, `§ref` folding

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/context` | L4 | lib | `polaris-runtime` | P3 |

**Purpose.** Builds each prompt as three parts: a byte-stable prefix (system prompt, tool schemas, versioned repo map) that a provider prefix cache can reuse, the epistemic scratchpad, and a sliding window. Large tool outputs and repeated file reads are folded into CAS and replaced by a short `§ref:<12hex>` marker with a deterministic digest; the model gets the original byte-for-byte via `polaris_expand`. Nothing important is ever replaced by an LLM-written summary. The compiler-diagnostics fold uses RTK so build noise shrinks without losing error codes or spans.

**API sketch.**
```rust
pub struct ContextAssembler<C: ContentStore, K: Tokenizer> { /* config, prefix cache, content store, token estimator */ }
pub struct StablePrefix { /* system text, tool specs sorted by name, repo-map version */ pub hash: PrefixHash /* blake3 */ }
pub struct AssembledPrompt { pub prefix: StablePrefix, pub scratchpad: String, pub window: Vec<Message>, pub stats: AssemblyStats }
impl<C: ContentStore, K: Tokenizer> ContextAssembler<C, K> {
    pub fn assemble(&self, req: &AssembleReq) -> Result<AssembledPrompt, ContextError>;
    pub async fn fold(&self, tool: ToolKind, output: &[u8]) -> Result<Folded, ContextError>;   // stores original in CAS, returns condensed view + ref
    pub async fn expand(&self, r: ShortRef, range: Option<ByteRange>) -> Result<bytes::Bytes, ContextError>; // byte-exact; backs polaris_expand
    pub fn context_map(&self) -> ContextMapReport;                                             // backs /context-map
}
pub struct Folded { pub view: String, pub r: ShortRef, pub original_bytes: u64, pub kind: FoldKind }
pub enum FoldKind { Inline, Diagnostics, HeadTail { head: u32, tail: u32 }, Skeleton, UnchangedFile { since_turn: u32 } }
pub fn short_ref_token(cid: &Cid, out: &mut RefBuf) -> usize;       // hot path: zero-alloc  ("§ref:" + 12 hex written into a preallocated buffer)
pub fn fold_head_tail(raw: &[u8], head: u32, tail: u32, out: &mut Vec<u8>) -> FoldStats; // hot path: zero-alloc once `out` has capacity

pub struct EpistemicScratchpad { pub goal: String, pub invariants: Vec<Entry>, pub confirmed: Vec<Entry>, pub disproven: Vec<Entry>,
                                 pub blockers: Vec<Entry>, pub touched_symbols: Vec<SymbolRef> }
pub struct Entry { pub turn: u32, pub text: String, pub evidence: Option<ShortRef> }          // append-only; overflow is folded to CAS with a ref, never dropped
pub struct ContextMapReport { pub total_tokens: u64, pub window_cap: u64, pub prefix_tokens: u64, pub scratchpad_tokens: u64,
                              pub active_scope_tokens: u64, pub folded_saved_tokens: u64, pub is_estimate: bool }
pub enum ContextError { Store(StoreError), UnknownRef(ShortRef), RefCollision, Budget, Rtk(RtkError) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-cas`, `polaris-code-intel`, `polaris-tools-rtk`. External: `blake3 = 1.8.7`; dev: `proptest = 1.12.0`, `insta = 1.49.0`, `criterion = 0.8.2`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: prefix assembly and stability, `fold` rules per tool kind, `short_ref_token`, `expand` byte-exactness, scratchpad append/overflow, `context_map`.
- [ ] Implement deterministic prefix serialisation (sorted tools, no timestamps or random ids, fixed number formatting, versioned repo map regenerated only on change) and expose `PrefixHash`; the same inputs must give identical bytes across runs and platforms (LF only).
- [ ] Implement `fold` rules: below the inline threshold (default 1 200 bytes, configurable) keep inline; compiler output goes through RTK first; repeated reads of an unchanged file become `[FILE: path | §ref:... | unchanged since turn N]`; otherwise head/tail fold with a line count.
- [ ] Implement `short_ref_token` and the head/tail fold without allocation (caller-supplied buffers); collision handling asks CAS for a longer id on the rare 12-hex clash.
- [ ] Implement `expand` (full or byte range) and prove round-trip `expand(fold(x)) == x` byte-exact with proptest, including invalid UTF-8 and CRLF.
- [ ] Implement `EpistemicScratchpad` as an append-only table placed after the stable prefix (it mutates, so it must not be inside the cached prefix); overflow entries are folded to CAS, never dropped.
- [ ] Implement the sliding window compression (only the middle window is touched; the prefix and scratchpad head are never reordered).
- [ ] Implement `ContextMapReport` with an honest `is_estimate` flag (bytes/4 heuristic until a real tokenizer for the target model is integrated).
- [ ] Provide the `RefResolver` implementation for the `polaris_expand` tool (flag 6) and a prefix-stability metric (fraction of requests whose `PrefixHash` equals the previous one), which is also the S0-5 fallback signal.
- [ ] Windows specifics: normalise paths in `[FILE: ...]` markers to forward slashes, treat CRLF and LF outputs as distinct bytes (CAS ids differ; the digest line notes it).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | inline below threshold; `unchanged since turn` marker only when the file blake3 matches; scratchpad entries keep turn numbers; ref marker length is 5 + 12 characters |
| proptest | `expand(fold(x)) == x` for arbitrary bytes; folding is deterministic; same inputs give identical prefix bytes and hash; scratchpad overflow never loses an entry (count preserved across fold) |
| fuzz | `fuzz_ref_parse` (arbitrary text scanned for `§ref:` markers; never panics), `fuzz_fold_bytes` |
| kani | `fold_head_tail` bounds: head+tail never exceeds the input length; no overflow in line counters |
| miri | pure fold and ref-formatting functions |
| load | fold throughput at least 200 MB/s on 10 MB logs (target; the chatlog "0.2 ms/KB" is about 5 MB/s and is not a goal); alloc count for `short_ref_token` = 0 (counting allocator) |
| stress | 150 000-token synthetic session folded in a loop for 10 minutes: bounded memory, CAS growth linear |
| scalability | window of 1 k, 10 k, 100 k tool outputs: assemble time grows linearly |
| security | folded content containing the literal `§ref:` text cannot forge a reference (markers are validated against CAS); secrets pass through the sanitiser before being folded into a prompt that goes to the cloud |
| fixtures | real `cargo` JSON and text logs, Windows CRLF logs, tool outputs with invalid UTF-8 (all generated or recorded, labelled) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Tool output folding | async_function | `ContextAssembler::fold` | `crates/polaris-runtime/crates/context/src/fold.rs` | `short_output_stays_inline`, `compiler_output_uses_rtk_view`, `unchanged_file_read_becomes_marker`, `head_tail_reports_folded_line_count`, `fold_then_expand_is_byte_exact` |
| Stable prefix | method | `ContextAssembler::assemble` | `crates/polaris-runtime/crates/context/src/prefix.rs` | `identical_inputs_give_identical_prefix_hash`, `tool_order_is_sorted_by_name`, `scratchpad_is_after_prefix`, `window_compression_never_touches_prefix` |
| Short ref token | function | `short_ref_token` | `crates/polaris-runtime/crates/context/src/refs.rs` | `token_has_fixed_length`, `zero_allocations_after_warmup`, `collision_extends_id`, `forged_marker_not_resolved` |
| Epistemic scratchpad | struct | `EpistemicScratchpad` | `crates/polaris-runtime/crates/context/src/scratchpad.rs` | `entries_are_append_only`, `overflow_folds_to_cas_with_ref`, `disproven_hypothesis_survives_window_compaction` |

**Risks & open questions.**
- VERIFIED (PLAN core): the stable prefix is system + tools + versioned repo map, then the scratchpad, then the window. The chatlog put the scratchpad inside the cached header, which would invalidate the cache on every update; corrected.
- UNVERIFIED: Token Factory prefix-caching behaviour and cached-token reporting (spike S0-5). Until proven, keep-warm pings stay disabled and `PrefixHash` stability is the only local signal.
- UNVERIFIED: the chatlog figures (70 to 85 percent fewer tokens, "about 13 tokens per ref", 86.8 percent savings, 0.2 ms/KB) are unmeasured; measure with the real tokenizer and a recorded session.
- LIKELY: 12 hex characters (48 bits) is collision-safe per session but not globally; CAS owns collision-checked short refs (`short_ref(&Cid)`).
- UNVERIFIED: Tool-pruning by S1 per step changes the tool list, which would break the stable prefix. Proposal: keep the full builtin tool list in the prefix and prune only MCP tools into a small number of fixed "tool profiles".
- OPEN: `polaris_recall` needs registry FTS search, which this crate cannot reach (no dependency on `polaris-storage-registry`); recall is served through an sdk trait implemented at the root (flag 6).

**Chatlog sources.** msg 23 (lossless context virtualization: three tiers, `§ref`, `polaris_expand`, repo map skeleton, epistemic scratchpad, `ContextVirtualizer`, `/context-map`, `/context-expand`), msg 29 (scratchpad per frame), msg 11 (prefix caching and request coalescing), msg 31 (token weather HUD inputs). Corrected: `§ref:CID§` full-hash form became `§ref:<12hex>`; `expect("CAS write failed")`, slice indexing and `todo!()` removed; "scratchpad never evicted" became append-only with CAS overflow; coalescing of S2 requests is deferred (rate-limit and S0-5 decide).

---

### `polaris-runtime-dag` — DAG scheduler, named semaphore pools, speculative races, structured cancellation

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/dag` | L4 | lib | `polaris-runtime` | P7 |

**Purpose.** Runs a validated acyclic task graph with maximum safe parallelism: each node starts exactly once, as soon as its dependencies have succeeded, under a named concurrency pool (`inference`, `web`, `docs`, `disk`, `s1`, `sandbox`) that protects against HTTP 429, `EMFILE`, socket starvation and tensor-pool contention. The same crate offers the speculative race (several sources in parallel, first accepted result wins, losers are aborted and release their permits) used by search and by Contree branching. Cancellation is structured: dropping or cancelling a run cancels every node.

**API sketch.**
```rust
pub enum Pool { Inference, Web, Docs, Disk, S1, Sandbox }
pub struct PoolLimits { pub inference: u32 /*6*/, pub web: u32 /*10*/, pub docs: u32 /*16*/, pub disk: u32 /*64*/, pub s1: u32 /*3*/, pub sandbox: u32 /*4*/ } // chatlog defaults or mid-ranges; to calibrate
pub struct Pools { /* one tokio Semaphore per pool + counters */ }
impl Pools {
    pub fn new(limits: PoolLimits) -> Result<Pools, DagError>;
    pub async fn acquire(&self, pool: Pool) -> Result<PoolPermit, DagError>;   // Err(Closed) instead of the chatlog `expect`
    pub fn resize(&self, pool: Pool, permits: u32) -> Result<(), DagError>;    // $concurrency(...)
    pub fn stats(&self) -> PoolStats;                                          // /concurrency-stats: active, queued, aborted_early per pool
}
pub struct NodeId(u32);
pub struct DagBuilder<T> { /* nodes, edges */ }
impl<T: NodeTask> DagBuilder<T> {
    pub fn node(&mut self, id: NodeId, pool: Pool, task: T) -> Result<&mut Self, DagError>;
    pub fn edge(&mut self, from: NodeId, to: NodeId) -> Result<&mut Self, DagError>;
    pub fn build(self) -> Result<Dag<T>, DagError>;                            // rejects cycles, unknown ids, duplicate ids (Kahn check)
}
pub trait NodeTask: Send + Sync + 'static {
    type Output: Send + 'static;
    fn run(&self, ctx: NodeCtx) -> impl Future<Output = Result<Self::Output, NodeError>> + Send; // ctx carries the CancellationToken and the held permit
}
pub enum OnFailure { FailFast, ContinueIndependent, Retry { max: u8 } }
pub struct RunOptions { pub on_failure: OnFailure, pub max_parallel: Option<u32> }
pub async fn run<T: NodeTask>(dag: Dag<T>, pools: &Pools, cancel: CancellationToken, opts: RunOptions) -> Result<DagReport<T::Output>, DagError>;
pub struct DagReport<O> { pub results: Vec<(NodeId, NodeResult<O>)>, pub order_started: Vec<NodeId>, pub snapshot: DagSnapshot /* for /parallel-dag */ }

pub trait Accept<T>: Send + Sync { fn accept(&self, candidate: &T) -> impl Future<Output = bool> + Send; } // e.g. S1 relevance >= threshold
pub async fn race<T: Send + 'static, A: Accept<T>>(branches: Vec<Branch<T>>, accept: &A, cancel: CancellationToken) -> Result<RaceOutcome<T>, DagError>;
pub enum RaceOutcome<T> { Won { index: usize, value: T, aborted: usize }, NoneAccepted }
pub enum DagError { Cycle, UnknownNode(NodeId), DuplicateNode(NodeId), Closed, Cancelled, NodeFailed(NodeId) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `tokio = 1.54.0` (`JoinSet`, `Semaphore`, `sync`), `tokio-util = 0.7.20` (`CancellationToken`); dev: `proptest = 1.12.0`, `hdrhistogram = 7.6.0`, `tokio` feature `test-util` (flag 8).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `DagBuilder::build` validation, `run` exactly-once scheduling, `Pools` permits and resize, `race` abort semantics.
- [ ] Implement `DagBuilder` with a typestate-free but validated `build()` (Kahn topological check) so an invalid graph can never be run.
- [ ] Implement the scheduler: in-degree counters, a started bitset, FIFO ready queue (ties by `NodeId` for reproducible traces), `JoinSet` for running nodes; a node is spawned exactly once.
- [ ] Handle failures and panics deliberately: `JoinError` (panic or cancel) maps to a node failure; `OnFailure` decides whether dependents are skipped, siblings continue or the run aborts; the loop terminates in all cases (the chatlog loop spun forever when a node failed).
- [ ] Implement `Pools` with typed permits (RAII), `resize` for `$concurrency`, counters for `/concurrency-stats`; replace every `expect` on `acquire_owned` with `Err(DagError::Closed)`.
- [ ] Implement `race` with `JoinSet::abort_all()` after the first accepted result; document that abort is cooperative and that blocking or native work must be run via supervised processes or sandbox cancellation.
- [ ] Implement `DagSnapshot` (running, ready, blocked-on, finished) for the TUI/GUI `/parallel-dag` view.
- [ ] Take branch counts from `$nebius-branching(max_branches=N)` (default up to 4 per chatlog) and wire sandbox-pool permits to them at the root.
- [ ] Windows specifics: `disk` pool default is bounded by the process handle limit (not the Unix `ulimit -n` of the chatlog); document the limit and measure.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | diamond graph runs A, then B and C in parallel, then D; cycle rejected at build; failed node skips dependents under `ContinueIndependent` |
| proptest | for random DAGs every node runs exactly once and never before all its dependencies finished; the run terminates for any failure pattern; permits always return to the initial count |
| fuzz | `fuzz_dag_build` (random edge lists: never panics, cycles always detected) |
| scalability | 1 to 32 workers over a 10 000-node graph: scheduler overhead per node under 50 microseconds (target) |
| spike | `tests/spike` "50-subagent bursts": 50 inference nodes against a pool of 6 never exceed 6 concurrent; 429 storm is the ratelimit crate's job |
| stress | cancel mid-run 1 000 times: no task survives, all permits released; race with 4 branches where 3 hang: losers aborted within 100 ms of the winner (target) |
| security | not applicable (no untrusted input; node tasks are trusted code) |
| fixtures | graph description files (TOML) used by `/parallel-dag` snapshot tests |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| DAG validation | method | `DagBuilder::build` | `crates/polaris-runtime/crates/dag/src/builder.rs` | `cycle_is_rejected`, `unknown_dependency_is_rejected`, `duplicate_node_is_rejected`, `empty_graph_runs_to_empty_report` |
| Exactly-once scheduling | async_function | `run` | `crates/polaris-runtime/crates/dag/src/run.rs` | `each_node_executes_exactly_once`, `dependents_wait_for_all_dependencies`, `panicking_node_does_not_hang_run`, `cancel_stops_all_nodes`, `ready_nodes_start_without_waiting_for_siblings` |
| Named pools | struct | `Pools` | `crates/polaris-runtime/crates/dag/src/pools.rs` | `permits_never_exceed_limit`, `resize_applies_to_new_acquires`, `closed_pool_returns_error_not_panic`, `stats_count_queued_waiters` |
| Speculative race | async_function | `race` | `crates/polaris-runtime/crates/dag/src/race.rs` | `first_accepted_wins_and_others_abort`, `rejected_candidates_do_not_end_race`, `losers_release_permits`, `none_accepted_returns_none` |

**Risks & open questions.**
- VERIFIED (read in chatlog msg 23, section 3.2): the chatlog `execute_parallel` re-scans all nodes on every loop iteration without recording started nodes, so a running node is spawned again (duplicate execution), and `while completed.len() < total` never ends if a node fails, panics or the graph has a cycle. This crate tracks a started set, validates acyclicity, and defines failure semantics.
- VERIFIED (plan correction): the chatlog "work-stealing scheduler on Tokio/Rayon" is unnecessary; the Tokio multi-thread runtime already work-steals, and Rayon is for CPU-bound data parallelism, not async tasks.
- LIKELY: cooperative abort cannot stop a task blocked in `spawn_blocking`, a native call or a remote sandbox command; losers in a sandbox race must also be cancelled through the backend (reaper registration).
- UNVERIFIED: pool defaults (chatlog ranges 4 to 8, 8 to 12, 16, 64, 2 to 4, 3 to 5) are guesses until measured against Token Factory soft caps (15-minute windows) and this 16 GB machine.
- UNVERIFIED: acceptance threshold for S1 validation of race results (chatlog 0.90) is uncalibrated; the `Accept` trait keeps it out of this crate.

**Chatlog sources.** msg 17 (Async Concurrency Mesh, six semaphore pools and defaults, `JoinSet` speculative race with `abort_all`, `$concurrency(web=15, inference=8, io=128)`, `/concurrency-stats`), msg 23 (DAG scheduler, `/parallel-dag`, speculative branch-and-prune in Contree), msg 21 (research workers as parallel supervised tasks). Corrected: `Box<dyn Fn() -> Pin<Box<dyn Future>>>` nodes replaced by a generic `NodeTask`; `.expect(..)` on semaphores removed; "lock-free MVCC codebase state" is not built here (see `polaris-runtime-leases`: OCC on base blake3 plus immutable CAS snapshots).

---

### `polaris-runtime-leases` — anti-thrashing file and AST-scope leases with FIFO queues, OCC and a churn cycle breaker

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/leases` | L4 | lib | `polaris-runtime` | P2 |

**Purpose.** Agents never write a file without a lease. A lease is either the whole file or one AST symbol; non-overlapping symbol leases in the same file run in parallel, whole-file leases exclude everything on that file, and waiters queue FIFO so an exclusive request cannot starve. Writes are checked with optimistic concurrency: the writer states the blake3 of the content it based its edit on, and a stale base is rejected with a notice. A churn counter revokes write rights from a frame that keeps rewriting the same file and escalates to S2, which ends edit wars.

**API sketch.**
```rust
pub struct LeaseKey(/* canonical workspace-relative path, Unicode case-folded, '/' separators */);
impl LeaseKey { pub fn from_path(root: &WorkspaceRoot, p: &std::path::Path) -> Result<LeaseKey, LeaseError>; } // rejects ".." escape, UNC, ADS, reserved device names
pub enum LeaseScope { EntireFile, AstNode(SymbolPath) }                // SymbolPath e.g. `crate::engine::compute`; resolved by code-intel at acquire and at write
pub struct LeaseRequest { pub agent: FrameId, pub key: LeaseKey, pub scope: LeaseScope, pub base: Option<Blake3Hash> }
pub struct LeaseManager { /* table, per-file FIFO queues, churn counters, event sink */ }
impl LeaseManager {
    pub fn new(cfg: LeaseConfig, events: EventSink) -> LeaseManager;
    pub async fn acquire(&self, req: LeaseRequest) -> Result<LeaseGuard, LeaseError>;
    pub fn try_acquire(&self, req: LeaseRequest) -> Result<LeaseGuard, LeaseError>;           // Busy { holder }
    pub async fn acquire_many(&self, reqs: Vec<LeaseRequest>) -> Result<Vec<LeaseGuard>, LeaseError>; // sorted by key; all-or-nothing; no lock-order deadlocks
    pub fn holder_of(&self, key: &LeaseKey, scope: &LeaseScope) -> Option<FrameId>;           // hot path: zero-alloc
    pub fn check_write(&self, g: &LeaseGuard, current: Blake3Hash) -> Result<WriteTicket, LeaseError>; // OCC; StaleBase { expected, actual }
    pub fn note_edit(&self, agent: FrameId, key: &LeaseKey, subtask: SubtaskId) -> ChurnVerdict;
    pub async fn revoke_all(&self, agent: FrameId, why: RevokeReason);
}
pub struct LeaseGuard { /* released on Drop (synchronous, best effort) or via release() */ }
pub struct WriteTicket { /* witness: the write applier requires it */ }
pub enum ChurnVerdict { Ok, Warn { edits: u32 }, Revoke { escalate_to_s2: bool } }              // default N = 3 edits per subtask (chatlog value)
pub enum LeaseError { Busy { holder: FrameId }, StaleBase { expected: Blake3Hash, actual: Blake3Hash }, Revoked(RevokeReason), Timeout, InvalidPath, Closed }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-code-intel`. External: `tokio = 1.54.0`, `dashmap = 6.2.1`; `+proposed arc-swap = 1.9.3` (read-mostly snapshot for the lookup hot path); dev: `proptest = 1.12.0`, `loom = 0.7.2`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: scope conflict predicate, FIFO acquisition, OCC `check_write`, churn cycle breaker, `LeaseKey` canonicalisation.
- [ ] Implement `LeaseKey::from_path` with Windows rules: case folding, `\` to `/`, reject `..` escapes, UNC/verbatim prefixes, alternate data streams (`name:stream`), reserved device names (`CON`, `NUL`, `COM1`...), trailing dots and spaces; short 8.3 names are resolved or rejected.
- [ ] Implement the conflict predicate: `EntireFile` conflicts with everything on that file; two `AstNode` leases conflict iff one symbol path is a prefix of the other or they are equal (nested symbols conflict, siblings do not); unknown languages fall back to `EntireFile`.
- [ ] Implement per-file FIFO queues with fairness (a queued exclusive request blocks later shared-style requests) and an acquire timeout.
- [ ] Implement `acquire_many` ordering by key so multi-file edits cannot deadlock.
- [ ] Implement OCC: grant carries `base`; `check_write` compares the current blake3; on mismatch emit `LeaseInvalidated` to waiters and the holder with the fresh diff reference.
- [ ] Implement `note_edit` and the churn cycle breaker (revoke write rights after more than N edits of one file within one subtask; emit event; escalate to S2).
- [ ] Implement the lock-free read path for `holder_of` (immutable snapshot swapped on grant and release) if `dashmap` shard locks fail the zero-alloc/lock-free budget.
- [ ] Implement crash behaviour: leases are in-memory only and are cleared at startup (frames do not survive a restart); guards release on `Drop`; a TTL reaps leases of a frame that vanished.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | two sibling symbols granted together; nested symbols conflict; exclusive waiter blocks later requests (FIFO); stale base rejected |
| proptest | model-based: random acquire/release sequences against a sequential reference model never hold two conflicting leases; no waiter starves under bounded releases; `LeaseKey` is idempotent and maps `A.rs` / `a.rs` / `.\a.rs` to one key |
| loom | 2 to 3 threads acquiring and releasing overlapping scopes: no lost wake-up, FIFO preserved, no double grant |
| kani | `scopes_conflict` predicate is symmetric and reflexive for bounded symbol paths |
| fuzz | `fuzz_lease_ops` (random op streams), `fuzz_lease_key` (arbitrary path strings; never panics, never escapes the root) |
| load | `holder_of`: 10 M lookups/s single thread and p99 under 1 microsecond with zero allocations after warm-up (targets) |
| stress | 64 frames contending for 20 files with random cancellation: zero leaked leases, no deadlock, bounded wait p99 |
| security | hostile-path corpus generated at test time (traversal, reserved names, case clashes, ADS, junction names): all rejected or canonicalised, never granted outside the root |
| fixtures | multi-language source files with known symbol ranges (Rust, Python, CUDA) from code-intel fixtures |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Scope conflict rules | function | `scopes_conflict` | `crates/polaris-runtime/crates/leases/src/scope.rs` | `sibling_symbols_do_not_conflict`, `nested_symbol_conflicts_with_parent`, `entire_file_conflicts_with_any_scope`, `unknown_language_falls_back_to_entire_file` |
| FIFO acquisition | async_function | `LeaseManager::acquire` | `crates/polaris-runtime/crates/leases/src/manager.rs` | `waiters_are_granted_in_order`, `exclusive_waiter_blocks_later_symbol_requests`, `guard_drop_wakes_next_waiter`, `acquire_times_out`, `acquire_many_is_all_or_nothing` |
| Optimistic concurrency | method | `LeaseManager::check_write` | `crates/polaris-runtime/crates/leases/src/occ.rs` | `matching_base_issues_ticket`, `stale_base_rejected_with_both_hashes`, `stale_write_notifies_waiters` |
| Lease key canonicalisation | function | `LeaseKey::from_path` | `crates/polaris-runtime/crates/leases/src/key.rs` | `case_variants_map_to_one_key`, `traversal_is_rejected`, `reserved_device_name_is_rejected`, `alternate_data_stream_is_rejected`, `backslashes_are_normalised` |
| Churn cycle breaker | method | `LeaseManager::note_edit` | `crates/polaris-runtime/crates/leases/src/churn.rs` | `fourth_edit_in_subtask_revokes_rights`, `different_subtask_resets_counter`, `revocation_escalates_to_s2` |

**Risks & open questions.**
- VERIFIED (PLAN core, hot-path closed set): lease lookup is zero-alloc. UNVERIFIED: `dashmap` reads take a shard `RwLock`; prompt.md §2.2 forbids mutexes in the hot path, so the arc-swap snapshot fallback may be required (decided by the load test).
- VERIFIED (plan correction): the chatlog "lock-free MVCC codebase state" is reduced to OCC on the base blake3 plus immutable CAS snapshots; there is no separate MVCC store.
- VERIFIED (chatlog correction): the chatlog consulted S1 inside the lock path (adjacent AST nodes); locking stays deterministic, S1 only arbitrates diffs afterwards in `polaris-system-one-verifier`.
- LIKELY: tree-sitter symbol resolution is syntactic; renames or macro-generated items can make symbol paths unstable, so a lease re-resolves the symbol at write time and conflicts conservatively.
- UNVERIFIED: NTFS case folding vs Unicode simple case folding differ in rare cases; over-locking (folding more) is safe, so folding is conservative.
- UNVERIFIED: the churn threshold N (chatlog "more than 3"; toml says "N times").

**Chatlog sources.** msg 11 (section 4: AST-aware file and scope leases, `ExclusiveFileLease` / `GranularScopeLease`, FIFO queue, invalidation notice, cycle breaker after more than 3 changes, `FileLeaseManager`), msg 23 (MVCC readers, writers in independent scopes), msg 13 (`AcquireAstScopeCommand`, `FileLeaseGrantedEvent`/`RevokedEvent`). Corrected: `String` agent ids and `HashMap<PathBuf, ..>` keys replaced by newtypes and canonical keys (Windows case and path forms); `todo!()` removed; S1 removed from the lock path.

---

### `polaris-runtime-guards` — Blast Radius estimator and the Proof-of-Done completion gate

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/guards` | L4 | lib | `polaris-runtime` | P2 |

**Purpose.** Two native re-implementations of ideas from Claude Code mods. Blast Radius estimates, before execution, which files a command would delete or overwrite (and which of them hold uncommitted work), and which call sites an AST signature change would affect, and requires approval when the impact is critical. Proof-of-Done makes "task complete" a state the agent cannot declare: only a fresh, parsed, successful build-and-test run in a sandbox, with no assertion tampering, yields the `Proven` witness that closes a task.

**API sketch.**
```rust
// Blast radius
pub struct BlastInput<'a> { pub command: &'a str, pub cwd: &'a WorkspacePath, pub shell: ShellKind /* Posix, PowerShell, Cmd */ }
pub trait WorkspaceView { fn list_matching(&self, pat: &GlobPattern) -> Result<Vec<FileInfo>, ViewError>; fn dirty(&self) -> Result<DirtySet, ViewError>; } // implemented by vcs + sync at the root
pub struct BlastReport { pub deleted: Vec<WorkspacePath>, pub overwritten: Vec<WorkspacePath>, pub total_bytes: u64, pub uncommitted_hit: Vec<WorkspacePath>,
                         pub ast_impact: Option<AstImpact>, pub severity: Severity, pub confidence: Confidence, pub reasons: Vec<String> }
pub enum Severity { None, Low, High, Critical }
pub enum Confidence { Exact, Estimated, Unknown }                    // Unknown command shape is treated as Critical
pub struct AstImpact { pub symbol: SymbolPath, pub call_sites: u32, pub files: u32, pub crates: u32, pub test_call_sites: u32 }
pub fn blast_radius<V: WorkspaceView>(view: &V, input: &BlastInput<'_>) -> Result<BlastReport, GuardError>;
pub fn ast_blast_radius(old: &Skeleton, new: &Skeleton, graph: &SymbolGraph) -> Result<Option<AstImpact>, GuardError>;
pub enum GuardVerdict { Allow, NeedsApproval(BlastReport), Deny(String) }

// Proof of Done: typestate completion
pub struct Pending; pub struct Proven;
pub struct Completion<S> { /* task id, tree hash, evidence */ }
pub struct ProofPolicy { pub require_zero_warnings: bool, pub min_tests: u32 /* default 1 */, pub max_proof_age_edits: u32 /* 0: tree must be unchanged since the run */ }
pub struct RunEvidence { pub command: String, pub exit_code: i32, pub tree_hash: Blake3Hash, pub stdout: Cid, pub stderr: Cid,
                         pub open_issues: u32 /* from RTK ledger after reconcile */, pub tests: TestSummary, pub tamper: TamperReport }
pub struct TestSummary { pub passed: u32, pub failed: u32, pub ignored: u32, pub parse: ParseStatus /* Parsed, Unparseable */ }
pub struct TamperReport { pub removed_assertions: u32, pub new_ignores: u32, pub tautologies: u32 } // computed by system-one-verifier postflight and passed in
pub fn proof_of_done(c: Completion<Pending>, run: &RunEvidence, now_tree: Blake3Hash, p: &ProofPolicy) -> Result<Completion<Proven>, ProofFailure>;
pub enum ProofFailure { BuildFailed { open_issues: u32 }, TestsFailed(u32), NoTestsRan, Unparseable, TreeChanged, Tampering(TamperReport), Warnings(u32) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-code-intel`, `polaris-tools-rtk`. External: `globset = 0.4.20`; dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `blast_radius`, `ast_blast_radius`, `proof_of_done` conditions, `Completion` typestate, shell command classification.
- [ ] Implement a conservative command classifier for POSIX shells, PowerShell and `cmd` (`rm`, `del`, `rmdir`, `Remove-Item`, `git reset --hard`, `git clean`, `find -delete`, `cargo clean`, redirection `>` truncation, `mv` overwrite); any command that cannot be classified is `Confidence::Unknown` and `Critical`.
- [ ] Implement glob expansion through `WorkspaceView` (no direct filesystem access in this crate), counting files and bytes and intersecting with the dirty set from `polaris-vcs`.
- [ ] Implement `ast_blast_radius` from `code-intel` skeleton diffs and the symbol graph; report call sites as an estimate (syntactic, not type-resolved).
- [ ] Implement `Completion<S>` typestate and `proof_of_done`: exit code 0 and `open_issues == 0` (RTK ledger), parsed test counts with `passed >= min_tests` and `failed == 0`, `TamperReport` clean, and evidence `tree_hash` equal to the current tree hash.
- [ ] Parse test runners into `TestSummary` (cargo test and nextest summary lines, pytest summary); an unparseable output is `Unparseable`, never a pass (chatlog hard-coded 1 or 0).
- [ ] Return the failing stderr reference so the agent loop can feed it back for bounded self-correction (max 3 retries per chatlog).
- [ ] Emit `BlastRadiusAlert` and `ProofOfDoneBlocked` events for the UI approve/abort flow.
- [ ] Windows specifics: normalise `\` in command arguments, handle `%VAR%` and `$env:VAR` as unknown expansions (Unknown confidence), case-insensitive path matching.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | `rm -rf build target` counts files from a fake view and marks uncommitted hits; unknown command is Critical; `cargo test` with zero tests run is `NoTestsRan` |
| proptest | classifier never panics on arbitrary command strings; severity is monotone in the number of affected files; `Proven` is only returned when every condition holds (property over generated evidence) |
| fuzz | `fuzz_shell_classify` (arbitrary bytes as command) |
| security | falsified-completion corpus generated at test time: assertions replaced by `assert!(true)`, tests marked `#[ignore]`, test command narrowed to a filter matching nothing, stale run before later edits, forged stdout claiming success: all blocked |
| smoke | with a real sandbox in P2: failing test blocks, passing test closes (keyed checks report SKIPPED only where a key is needed; this one needs none) |
| load | classify 100 000 commands/s (target); blast radius over a 100 k-file fake view under 100 ms (target) |
| fixtures | recorded cargo/pytest outputs for pass, fail, zero-test, panic and compile-error cases (labelled synthetic until re-recorded) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Command blast radius | function | `blast_radius` | `crates/polaris-runtime/crates/guards/src/blast.rs` | `rm_rf_lists_files_and_bytes`, `uncommitted_files_raise_severity`, `unknown_command_is_critical`, `powershell_remove_item_recognised`, `glob_outside_workspace_is_critical` |
| AST blast radius | function | `ast_blast_radius` | `crates/polaris-runtime/crates/guards/src/ast_blast.rs` | `signature_change_counts_call_sites`, `body_only_change_has_no_impact`, `test_call_sites_reported_separately` |
| Proof of done gate | function | `proof_of_done` | `crates/polaris-runtime/crates/guards/src/proof.rs` | `build_failure_blocks`, `zero_tests_is_not_a_pass`, `assertion_tampering_blocks`, `tree_changed_since_run_blocks`, `unparseable_output_is_not_a_pass`, `all_conditions_yield_proven` |
| Completion typestate | struct | `Completion<S>` | `crates/polaris-runtime/crates/guards/src/completion.rs` | `pending_cannot_close_task_compile_fail`, `proven_carries_evidence_cid`, `proven_only_constructed_by_gate` |

**Risks & open questions.**
- VERIFIED (read in chatlog msg 31): the chatlog `verify_proof_of_done` used `.expect(..)`, set `tests_passed` to 1 or 0 from the exit code alone, and `estimate_blast_radius` only matched `rm ` and returned an empty report; this plan requires parsed counts, evidence freshness and unknown-is-critical.
- VERIFIED (plan correction): "Proof-of-Done as a cryptographic invariant" is implemented as a typestate witness plus recorded evidence, not a cryptographic proof. A malicious agent can still weaken tests in ways a classifier misses; the S1 postflight is a heuristic.
- UNVERIFIED: shell parsing is inherently incomplete (quoting, aliases, `eval`, here-docs); the `Unknown`-is-`Critical` rule is the safety net and will produce approval prompts for unusual commands.
- LIKELY: static call-site counts are approximate (no type resolution); the UI must say "estimated".
- OPEN: whether Blast Radius evaluates against the Windows host workspace, the sandbox mirror, or both; proposal: both, because the host is authoritative and the mirror holds uncommitted agent work.

**Chatlog sources.** msg 31 (Blast Radius, AST blast radius example, Proof-of-Done barrier with S1 assertion check, `NativeAugmentationGuard`, token weather inputs), msg 1 (section 9: verification loop, stderr back to Nemotron, max 3 retries), msg 11 (S1 verifier tamper checks live in `polaris-system-one-verifier`). Corrected: panic-on-failure and fake counts removed; "cryptographic invariant" reworded; reflect and token-weather HUD are not in this crate (flag 7; telemetry).

---

### `polaris-runtime-research` — `/research` mode: Karpathy autoresearch loop plus a Polaris evidence ledger and adversarial critique

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-runtime/crates/research` | L4 | lib | `polaris-runtime` | P7 |

**Purpose.** Long-running, resumable research and optimisation. The core loop follows Karpathy's autoresearch as verified in the PLAN core (§0–§12): a human-written `program.md` fixes the goal and the metric, the agent edits, runs a fixed-budget experiment, measures, keeps or reverts the change, and repeats. Polaris adds an evidence ledger (every claim cites a source), an adversarial skeptic frame that tries to refute a result before it is marked confirmed, and checkpoints so `/research resume` continues after a crash. The epistemic graph and devil's-advocate parts are Polaris additions, not Karpathy's design.

**API sketch.**
```rust
pub struct ProgramMd { pub goal: String, pub metric: MetricSpec, pub budget: ExperimentBudget, pub editable: Vec<GlobPattern>, pub readonly: Vec<GlobPattern> }
pub struct MetricSpec { pub name: String, pub direction: Direction /* Min | Max */, pub command: Vec<String>, pub parse: MetricParse }  // the harness runs it; the agent cannot edit it
pub struct ExperimentBudget { pub wall_time: core::time::Duration, pub tokens: u64, pub max_experiments: Option<u32> }
pub struct ResearchLoop<W: ExperimentWorkspace, R: ResearchRunner, S: EventStore> { /* program, workspace, runner, store, state */ }
pub trait ExperimentWorkspace { // implemented at the root with polaris-vcs worktrees
    fn begin(&self) -> impl Future<Output = Result<ExperimentId, WorkspaceError>> + Send;
    fn keep(&self, id: ExperimentId) -> impl Future<Output = Result<CommitRef, WorkspaceError>> + Send;
    fn revert(&self, id: ExperimentId) -> impl Future<Output = Result<(), WorkspaceError>> + Send;
}
impl<W, R, S> ResearchLoop<W, R, S> {
    pub fn new(program: ProgramMd, ws: W, runner: R, store: S) -> Result<Self, ResearchError>;
    pub async fn step(&mut self) -> Result<StepReport, ResearchError>;     // one hypothesis -> experiment -> metric -> keep/revert -> log
    pub async fn run_until(&mut self, stop: StopCondition) -> Result<RunSummary, ResearchError>;
    pub async fn resume(program: ProgramMd, ws: W, runner: R, store: S) -> Result<Self, ResearchError>; // from the last completed iteration
    pub fn status(&self) -> ResearchStatus;                                 // /research status
    pub fn report(&self) -> Result<String, ResearchError>;                  // /research report (Markdown)
}
pub struct Hypothesis { pub id: HypId, pub statement: String, pub status: HypStatus /* Open, Confirmed, Disproven, Inconclusive */, pub evidence: Vec<EvidenceId> }
pub struct Evidence { /* private fields; constructor requires a Source */ }
pub enum Source { Url { url: String, retrieved: jiff::Timestamp, cid: Cid }, PdfLine { cid: Cid, page: u32, line: u32 }, AstSymbol(SymbolRef), TestRun { cid: Cid }, Experiment { id: ExperimentId, metric: MetricValue } }
pub struct ResearchCheckpoint { pub generation: u32, pub hypotheses: Vec<Hypothesis>, pub ledger: Cid, pub best: Option<MetricValue>, pub spent: Spent }
pub enum StopCondition { Budget, Plateau { patience: u32 }, Manual, MaxExperiments(u32) }
pub enum ResearchError { Program(&'static str), Workspace(WorkspaceError), Runner(RunnerError), Store(StoreError), BudgetExhausted, Frame(FrameError) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-runtime-frames`, `polaris-runtime-context`. External: `serde = 1.0.229` (features `derive`), `jiff = 0.2.38`; dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `ProgramMd` parsing and validation, `ResearchLoop::step` keep/revert rule, `Evidence` source requirement, checkpoint and `resume`, adversarial confirmation.
- [ ] Implement `ProgramMd` parsing (Markdown front matter plus fenced metric block) and validation: goal non-empty, metric command present, editable and readonly globs disjoint, budgets positive.
- [ ] Protect the measurement: the metric command, `program.md` and the harness files are readonly for the agent frame (capability scope) so the metric cannot be gamed by editing it.
- [ ] Implement `step`: S2 proposes one hypothesis and a patch (child frame with attenuated token), experiment runs in a sandbox within the fixed budget, metric parsed, `keep` iff strictly better in the declared direction (ties revert), otherwise `revert`; both outcomes logged.
- [ ] Implement the evidence ledger: `Evidence` cannot be built without a `Source`; claims in the report cite ledger ids; the S1 sifter feeds candidate quotes (via sdk trait).
- [ ] Implement the skeptic: before a hypothesis becomes `Confirmed`, a read-only frame tries to refute it (re-run on another seed or input, look for confounders); unrefuted after one round, confirmed; refuted, disproven.
- [ ] Implement checkpointing into the event store after every completed iteration and `resume` restoring state; an interrupted in-flight experiment is discarded and re-run.
- [ ] Implement budgets (wall time, tokens, `$budget` USD) and stop conditions (plateau patience, max experiments); expose progress to `/research status` and the final `/research report` with the ledger table.
- [ ] Windows specifics: experiment working copy is a git worktree on a short path (`target-dir` rules), metric output parsed as UTF-8 with CRLF tolerance.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | better metric keeps, equal or worse reverts (Min and Max); `Evidence` without a source does not compile; resume restores generation and best metric |
| proptest | for any sequence of metric values the kept sequence is monotone in the declared direction; checkpoint round-trips; report cites only existing ledger ids |
| smoke | scripted toy problem (minimise a polynomial by editing a constant) runs 10 iterations with a fake runner and fake workspace |
| stress | kill the process at random points across 200 runs, then resume: no iteration is applied twice, no worktree leaks |
| security | agent attempts to edit the metric script or `program.md`: rejected by scope; experiment sandbox has no network unless `$websearch` is on |
| fixtures | sample `program.md` files (valid and invalid), synthetic metric outputs |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Program definition | function | `ProgramMd::parse` | `crates/polaris-runtime/crates/research/src/program.rs` | `valid_program_parses`, `missing_metric_is_rejected`, `overlapping_editable_and_readonly_rejected`, `non_positive_budget_rejected` |
| Keep or revert rule | method | `ResearchLoop::step` | `crates/polaris-runtime/crates/research/src/step.rs` | `improvement_is_kept`, `tie_is_reverted`, `worse_is_reverted`, `budget_exceeded_aborts_experiment`, `metric_parse_failure_reverts` |
| Evidence ledger | struct | `Evidence` | `crates/polaris-runtime/crates/research/src/evidence.rs` | `evidence_requires_source`, `report_cites_ledger_ids`, `url_source_records_retrieval_time_and_cid` |
| Checkpoint and resume | async_function | `ResearchLoop::resume` | `crates/polaris-runtime/crates/research/src/checkpoint.rs` | `resume_restores_generation`, `inflight_experiment_is_discarded`, `resume_with_changed_program_is_rejected` |

**Risks & open questions.**
- VERIFIED (PLAN core): the autoresearch loop is human `program.md` (goal and metric), edit, fixed-budget run, measure, keep or revert, repeat; epistemic graph and devil's advocate are Polaris additions (the chatlog attributed them to Karpathy).
- UNVERIFIED: unattended multi-hour runs cost tokens; budgets and `$budget` must gate them. No keys yet (D9), so real-model research runs are BLOCKED-ON-ACCESS; the loop is exercisable with the local OpenAI-compatible endpoint (P3 skeleton) and fakes.
- LIKELY: Goodhart risk (the agent optimises the metric by editing the harness); mitigated by readonly scope, which must be enforced by the capability layer, not by prompt text.
- OPEN: where hypotheses are generated (frame role `Researcher`) and how S1 contributes (fact distillation, sifting) is specified by the system-one crates; this crate only defines the traits it consumes.

**Chatlog sources.** msg 21 (section 1 and 6: `/research`, `/research status|report|resume`, hypothesis tree, evidence ledger with URL/PDF line/AST symbol/test run citations, adversarial critique, crash-resilient checkpoints in redb and LanceDB, status panel format), msg 29 (Researcher role frame), msg 17 (parallel web/docs/code exploration). Corrected: attribution of epistemic graph and devil's advocate; LanceDB checkpoints dropped (the event store is enough); "indefinitely" replaced by explicit budgets and stop conditions.

---

### `polaris-models` — model registry, effort tiers, `$subagent-models` mapping and local model lifecycle

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-models` | L3 | lib | (top-level) | P3 |

**Purpose.** Single source of truth for which model serves which purpose: the verified Token Factory ids, the effort-to-model mapping (low..max or 1..5), user overrides from `$subagent-models`, the vision model, the embedding model, and custom OpenAI-compatible entries (the local Ollama/llama.cpp endpoint used for the P3 walking skeleton). It also owns the lifecycle of local model slots: lazy load, RAII leases, idle eviction and forced eviction before heavy builds, so a 16 GB machine can compile and run a model in turn.

**Verified catalog (live Token Factory `api/public/models_info`, 2026-10-20; VERIFIED).**
| Role | Model id | Context | Default effort |
|---|---|---|---|
| fast | `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B` | 262K | low (1) |
| balanced | `nvidia/Nemotron-3_5-Lightning` (display name "Nemotron-3.5-Lightning") | 1M | medium (2) |
| strong | `nvidia/nemotron-3-super-120b-a12b` | 256K | high (3); xhigh (4) = same model with a high reasoning budget |
| max | `nvidia/Nemotron-3-Ultra-550b-a55b` | 1M | max (5) |
| vision | `nvidia/Nemotron-3.5-Super-VL` | 1M per `context_window_k` | image tasks |
| embeddings | `Qwen/Qwen3-Embedding-8B` | tags say 32K, `context_window_k` says 41 (UNVERIFIED which is right) | D8 |

**API sketch.**
```rust
pub enum ModelKind { Chat, Vision, Embedding }
pub enum ProviderKind { TokenFactory, OpenAiCompatible }                       // adapters chosen at the root by enum dispatch
pub struct ModelSpec { pub id: ModelId, pub kind: ModelKind, pub provider: ProviderKind, pub context_tokens: u32,
                       pub tools: bool, pub reasoning: bool, pub price: Option<PriceTable> }   // price is None until configured (not in the public catalog)
pub enum ReasoningBudget { Default, High }                                     // how to request "high" is UNVERIFIED (S0-5)
pub struct ModelChoice { pub model: ModelId, pub reasoning: ReasoningBudget }

pub struct ModelRegistry { /* specs, effort table, aliases */ }
impl ModelRegistry {
    pub fn builtin() -> ModelRegistry;                                         // the verified catalog above
    pub fn with_config(cfg: &ModelsConfig) -> Result<ModelRegistry, ModelError>; // custom entries: id, base_url key reference, kind, context
    pub fn get(&self, id: &ModelId) -> Option<&ModelSpec>;
    pub fn resolve(&self, name: &str) -> Result<ModelId, ModelError>;           // aliases: nano, lightning, super, ultra, vl, embed; or an exact id
    pub fn model_for(&self, e: Effort) -> Result<ModelChoice, ModelError>;
    pub fn apply_subagent_models(&mut self, v: &[ModelEffort]) -> Result<(), ModelError>; // from $subagent-models; first match in declared order, gaps use defaults
    pub fn cascade_for(&self, e: Effort) -> Result<CascadePlan, ModelError>;    // $nebius-speculative: cheap draft, stronger verify
}
pub struct CascadePlan { pub draft: ModelChoice, pub verify: ModelChoice }

// Local model lifecycle (cold -> active -> draining -> cold); generic, no Box<dyn Fn>.
pub trait ModelLoader<M>: Send + Sync {
    fn load(&self) -> impl Future<Output = Result<M, LoadError>> + Send;
    fn unload(&self, m: M) -> impl Future<Output = ()> + Send;                  // e.g. ask the s1 worker to drop its ONNX session, or stop the worker
}
pub enum SlotState { Cold, Loading, Active { leases: u32 }, Draining }
pub struct EvictableSlot<M, L: ModelLoader<M>> { /* state machine, last-used (atomic), idle_ttl */ }
impl<M: Send + Sync + 'static, L: ModelLoader<M>> EvictableSlot<M, L> {
    pub fn new(name: &'static str, idle_ttl: core::time::Duration, loader: L) -> Self;
    pub async fn acquire(&self) -> Result<ModelLease<M>, ModelError>;           // lazy load; concurrent callers share one load
    pub async fn evict_if_idle(&self) -> EvictOutcome;
    pub async fn force_evict(&self, deadline: core::time::Duration) -> EvictOutcome; // before heavy builds
    pub fn state(&self) -> SlotState;
}
pub struct ModelLease<M> { /* Arc<M> + lease counter; updates last-used on drop */ }
pub enum EvictOutcome { Evicted, NotLoaded, Draining { leases: u32 }, StillInUse }
pub trait Evictable { fn evict(&self, reason: EvictReason) -> impl Future<Output = EvictOutcome> + Send; }
pub enum EvictReason { Idle, MemoryPressure, HeavyBuild, Shutdown }
pub struct EvictionManager<E: Evictable> { /* slots, policy */ }                 // E is the root's enum of slot kinds (enum dispatch)
pub enum ModelError { UnknownModel(String), BadAlias(String), EffortGap, Load(LoadError), Closed, Config(&'static str) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`. External: `serde = 1.0.229` (features `derive`), `tokio = 1.54.0`; dev: `proptest = 1.12.0`, `loom = 0.7.2`, `tokio` feature `test-util`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `model_for` mapping, `apply_subagent_models`, alias resolution, `cascade_for`, `EvictableSlot` state machine, `EvictionManager` triggers.
- [ ] Encode the verified catalog as `builtin()` with ids copied from the live response (note the underscore in `Nemotron-3_5-Lightning`) and a test that fails if an id is not in the committed catalog snapshot `tests/fixtures/token_factory/models_info.json` (provenance: fetched 2026-10-20, public endpoint).
- [ ] Implement `Effort` conversions (`FromStr` for names and numbers, `TryFrom<u8>`, ordering) in coordination with sdk and share them with the directives parser.
- [ ] Implement `apply_subagent_models`: validate every model resolves, ranges are within 1..5, report gaps (efforts with no model use the default mapping) and overlaps (first declared wins), deterministic.
- [ ] Implement custom OpenAI-compatible entries from config so the P3 skeleton can run against a local Ollama/llama.cpp server on the GTX 1650 (a real endpoint, not a stub).
- [ ] Implement `EvictableSlot` with an atomic last-used timestamp, a single shared load for concurrent callers, and `Draining` semantics: eviction while leases exist waits until the last lease drops or the deadline passes.
- [ ] Implement `EvictionManager` triggers: idle ticker, memory-pressure signal (from sdk `PlatformOps::memory_pressure`), and an explicit `evict_all(HeavyBuild)` called by the root when the shell tool classifies a heavy build.
- [ ] Implement `cascade_for` for `$nebius-speculative` as a plan only (draft on a cheaper tier, verify on the next tier); no claim of speculative-decoding speed-ups.
- [ ] Expose a price-table hook (configured prices only; telemetry computes cost) and a `ModelSpec` serialisation for `/providers` and `/model` listings.
- [ ] Windows specifics: loaders must not rely on `madvise`/`mmap` semantics; document that RAM is returned reliably only by dropping the session inside the worker or exiting the worker process.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | defaults: 1 to Nano, 2 to Lightning, 3 to Super, 4 to Super with `High` reasoning, 5 to Ultra; numeric and named efforts equal; unknown alias errors; `$subagent-models` overlap resolution |
| proptest | `model_for` is total over 1..=5; with the default table the tier is monotone in effort; `apply_subagent_models` is deterministic and never leaves an effort unmapped; slot state machine: leases never negative, `Cold` implies zero leases |
| loom | concurrent `acquire` / `evict_if_idle` / lease drop: at most one load in flight, no use-after-evict, no lost wake-up |
| smoke | fake loader: acquire loads once, idle ttl evicts, re-acquire reloads; contract test that every `builtin()` id exists in the committed catalog snapshot |
| stress | memory-pressure eviction with 8 concurrent users and a 10 ms fake load for 5 minutes: no deadlock, load count equals evict count plus one |
| security | custom entries reference secrets by name only (no key in config or logs) |
| fixtures | `models_info.json` snapshot (provenance and date recorded); a synthetic list with missing fields to test tolerant parsing |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Effort to model mapping | method | `ModelRegistry::model_for` | `crates/polaris-models/src/registry.rs` | `effort_1_maps_to_nano`, `effort_4_is_super_with_high_reasoning`, `effort_5_maps_to_ultra`, `numeric_and_named_efforts_agree`, `override_from_subagent_models_wins` |
| Subagent model overrides | method | `ModelRegistry::apply_subagent_models` | `crates/polaris-models/src/overrides.rs` | `first_declared_range_wins`, `gaps_fall_back_to_defaults`, `unknown_model_name_is_error`, `range_outside_1_to_5_is_error` |
| Evictable slot | struct | `EvictableSlot` | `crates/polaris-models/src/lifecycle.rs` | `concurrent_acquire_loads_once`, `idle_ttl_evicts_unused_model`, `eviction_waits_for_active_lease`, `force_evict_respects_deadline`, `reacquire_after_evict_reloads` |
| Eviction manager | struct | `EvictionManager` | `crates/polaris-models/src/evict.rs` | `heavy_build_evicts_all_slots`, `memory_pressure_evicts_least_recently_used_first`, `shutdown_evicts_everything` |

**Risks & open questions.**
- VERIFIED (live catalog): ids and context sizes above; the chatlog Nemotron-70B/340B ids are stale and are not used. UNVERIFIED: per-model prices (absent from the public catalog), reasoning-budget control, tool-call quality per model (S0-5).
- VERIFIED (plan): speculative decoding is not client-controllable, so `$nebius-speculative` is cascade drafting; the extra draft cost may exceed the saving, UNVERIFIED until measured.
- VERIFIED (plan): GLiNER2.5-Decide ships as 1.95 GB fp32 safetensors; the chatlog "about 700 MB" and "mmap reload under 8 ms" do not hold. The chatlog lifecycle code used a `ModelLease` with a dummy `Arc<()>` counter, a `std::sync::Mutex` held across `.await` and `unwrap()` calls; these are replaced by an atomic timestamp and a real lease count.
- LIKELY: the s1 model lives in the `polaris-worker` process, so "evict" means telling the worker to drop its session (or stopping the worker); the exact RPC lives in `polaris-system-one-*`.
- UNVERIFIED: embedding dimension and context of `Qwen/Qwen3-Embedding-8B`; read from the first real response (S0-5) rather than hard-coded.
- OPEN: heavy-build detection (`cargo build`, `nvcc`, `npm run build`) is a shell-tool classification (flag in builtin), not a models concern.

**Chatlog sources.** msg 39 (Functional operators plus adaptive model lifecycle: three-phase cold/active/idle, `ModelLease`, `EvictableModelSlot`, idle 45 s, `force_evict` before compilers), msg 1 and msg 3 (effort levels 1 to 5, `$subagent-models`, `EffortLevel`), msg 29 (single S2 and single S1 instance), msg 23 (prefix-cache friendliness, informing model selection per frame). Corrected: model ids (live catalog), effort mapping per PLAN core (xhigh = Super with a high reasoning budget), speculative decoding replaced by cascade drafting, mmap/madvise claims, memory sizes and timings.

---

### `polaris-tools` — facade over the agent tool crates (builtin, math, rtk)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-tools` | L3 | facade | (top-level) | P1 |

**Purpose.** Pure re-export crate; no logic. Publishes the tool catalog (name, input schema, effects, execution tier, phase) so the prompt assembler, the S1 tool pruner and the docs list the same tools. Children never depend on this facade; the registry that combines them is assembled at the composition root because `math` and `rtk` do not depend on `builtin` (flag 1).

**Re-exports.** `pub use polaris_tools_builtin as builtin;` `polaris_tools_math as math;` `polaris_tools_rtk as rtk;` (workspace.toml: `pub use polaris_tools_*`).

**TODO.**
- [ ] Re-export the three children; compile-only doctest naming one item per child.
- [ ] Generate `docs/tool-catalog.md` from the registered `ToolSpec`s (name, effects, tier, phase) as part of `cargo doc`/xtask so it cannot drift.
- [ ] Add a `check-layers` expectation that no tool child depends on `polaris-tools`.
- [ ] Record which tools are E0 read-only (P1), proposal-producing writes (P2), sandboxed exec (P2) and network (P5).

---

### `polaris-tools-builtin` — built-in agent tools and the `ToolRegistry`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-tools/crates/builtin` | L3 | lib | `polaris-tools` | P1 |

**Purpose.** The agent's everyday tools with JSON-Schema inputs and typed handlers: read-only tools run in-process (tier E0) in P1; every mutation is a `ProposeDiff` command that the verifier and the lease manager gate, never a direct write; shell execution only ever goes through a sandbox backend. Path handling is the main security surface: all paths are resolved against the workspace root with Windows rules, and secret-bearing files are denied by default.

**API sketch.**
```rust
// In polaris-sdk (flag 1): trait Tool { const NAME; const EFFECTS: Effects; type Input: DeserializeOwned + JsonSchema; type Output: Serialize;
//                                        fn call(&self, i: Self::Input, c: &ToolContext<'_>) -> impl Future<Output = Result<Self::Output, ToolError>> + Send; }
pub struct ToolRegistry<S: SandboxBackend> { /* enum-dispatch table sorted by name, limits, sandbox handle */ }
impl<S: SandboxBackend> ToolRegistry<S> {
    pub fn builtin(cfg: &ToolsConfig, sandbox: S) -> Result<ToolRegistry<S>, ToolError>;
    pub fn specs(&self) -> &[ToolSpec];                                        // stable, sorted: part of the cached prompt prefix
    pub fn get(&self, name: &str) -> Option<&ToolSpec>;
    pub async fn invoke(&self, name: &str, args: &serde_json::Value, ctx: &ToolContext<'_>) -> Result<ToolOutput, ToolError>;
}
pub enum BuiltinTool { ReadFile, ListDir, Glob, Grep, RepoMap, Skeleton, JsonQuery, RegexQuery, Expand, Recall,   // P1, E0 read-only
                       WriteFile, EditFile, ApplyPatch,                                                          // P2, produce ProposeDiff
                       Shell, Git,                                                                                // P2, sandbox / vcs
                       Fetch }                                                                                    // P5, network via search providers
pub struct ToolContext<'a> { pub frame: FrameId, pub token: &'a CapabilityToken, pub root: &'a WorkspaceRoot, pub limits: &'a ToolLimits, pub cancel: &'a CancellationToken }
pub struct ToolLimits { pub max_read_bytes: u64, pub max_results: u32, pub max_output_bytes: u64, pub regex_size_limit: usize, pub timeout: core::time::Duration }
pub struct ToolOutput { pub value: serde_json::Value, pub truncated: bool, pub bytes: u64 }
pub struct EditFile { pub path: String, pub old: String, pub new: String, pub replace_all: bool, pub base: Option<Blake3Hash> } // returns a Proposal, never writes
pub struct Shell { pub command: String, pub cwd: Option<String>, pub timeout_s: u32 }                                  // class: Light | HeavyBuild | Destructive (feeds eviction and blast radius)
pub enum CommandClass { Light, HeavyBuild, Destructive, Unknown }
pub fn classify_command(cmd: &str) -> CommandClass;
pub enum ToolError { InvalidArgs(String), Path(PathError), Denied(&'static str), TooLarge { limit: u64 }, Regex(RegexError), Timeout, Capability(CapabilityError), Sandbox(SandboxError) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-code-intel`, `polaris-security-automata`. External: `ignore = 0.4.33`, `globset = 0.4.20`, `regex = 1.13.1`, `serde_json = 1.0.154`, `schemars = 1.2.3`; `+proposed serde = 1.0.229`, `tokio = 1.54.0` (timeouts only), `encoding_rs = 0.8.42` (UTF-16 detection for Windows tool output); dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: path resolution/guard, `read_file`, `grep`, `glob`, `edit_file` proposal, `classify_command`, registry spec stability.
- [ ] Move the `Tool` trait, `ToolSpec`, `Effects` into sdk (flag 1) and implement `ToolRegistry` with enum dispatch (no `dyn Tool`); specs sorted and byte-stable.
- [ ] Implement `WorkspacePath` resolution (shared with leases via sdk): reject `..` escapes, absolute paths outside the root, UNC and verbatim prefixes, ADS names, reserved device names, trailing dots and spaces; resolve symlinks/junctions then re-check the prefix; reads open the file once to limit time-of-check/time-of-use gaps.
- [ ] Implement read-only tools: `read_file` (offset/limit, binary detection by NUL in the first 8 KiB, BOM handling, UTF-16 detection, byte-exact line endings reported), `list_dir`, `glob`, `grep` (regex with `size_limit`, HIR guard from automata, result and time caps, `ignore`-aware traversal honouring `.gitignore` and `.polarisignore`), `repo_map`, `skeleton`, `json_query`, `regex_query`.
- [ ] Deny secret-bearing paths by default for reads (`.env`, `*.pem`, `*.key`, `.git/config`; same deny-list as sync) unless a human-approved override is present in the token facts.
- [ ] Implement `polaris_expand` / `polaris_recall` schemas and dispatch to `RefResolver` / `RecallBackend` traits (flag 6).
- [ ] Implement write tools as proposals (P2): `edit_file` with exact-match uniqueness and OCC base hash, `write_file`, `apply_patch`; line endings preserved; the proposal goes to the bus.
- [ ] Implement `Shell` over `SandboxBackend` only; `classify_command` for heavy builds and destructive commands; large outputs are returned in full to the folder (context crate), not truncated here.
- [ ] Keep the builtin tool list in the stable prompt prefix and let S1 prune only MCP tools into fixed profiles (see context risks).
- [ ] Windows specifics: `\` and `/` both accepted in args, outputs use `/`; long paths with `\\?\` handled internally only; CRLF files edited without converting endings.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | read with offset/limit; grep honours ignore files; edit with 2 matches is an error unless `replace_all`; stale base hash rejected; denied path returns `Denied` |
| proptest | `WorkspacePath::resolve` result is always inside the root or an error, for arbitrary strings; `grep` never returns more than `max_results`; `classify_command` never panics |
| fuzz | `fuzz_tool_args_json` (arbitrary JSON per tool schema), `fuzz_grep_pattern`, `fuzz_path_resolve` |
| security | hostile paths generated at test time (reserved names, case clashes, ADS, junction escape, `..\..`, `C:\Windows\System32`, UNC); regex bombs (`(a{1000}){1000}`) rejected by size limit; secret files denied; shell tool never spawns on host (fake backend asserts) |
| load | grep over 10 000 files under 1 s and 100 000 files under 10 s on this machine (targets; scalability 1k/10k/100k repos) |
| smoke | each read-only tool invoked once on a temp workspace; schemas validate with `jsonschema` in tests (dev-dependency `jsonschema = 0.59.0`) |
| fixtures | temp workspaces from `polaris-tests` (generated; hostile names created at test time, never committed) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Workspace path guard | function | `resolve_in_workspace` | `crates/polaris-tools/crates/builtin/src/path.rs` | `dotdot_escape_rejected`, `reserved_device_name_rejected`, `junction_escape_rejected`, `unc_path_rejected`, `case_variant_resolves_to_same_file` |
| Grep tool | struct | `Grep` | `crates/polaris-tools/crates/builtin/src/grep.rs` | `respects_gitignore_and_polarisignore`, `result_cap_sets_truncated`, `regex_bomb_rejected`, `binary_files_skipped`, `timeout_returns_partial_with_flag` |
| Read file tool | struct | `ReadFile` | `crates/polaris-tools/crates/builtin/src/read.rs` | `secret_file_denied`, `utf16_bom_decoded`, `offset_limit_window`, `crlf_reported_not_converted`, `too_large_returns_error_with_limit` |
| Edit file proposal | struct | `EditFile` | `crates/polaris-tools/crates/builtin/src/edit.rs` | `unique_match_required`, `replace_all_counts_matches`, `stale_base_hash_rejected`, `crlf_preserved_in_proposal`, `edit_never_writes_to_disk` |
| Command classification | function | `classify_command` | `crates/polaris-tools/crates/builtin/src/shell.rs` | `cargo_build_is_heavy`, `nvcc_is_heavy`, `rm_rf_is_destructive`, `unknown_is_unknown`, `powershell_remove_item_is_destructive` |

**Risks & open questions.**
- VERIFIED (read in `workspace.toml`): builtin lists neither `tokio`, `serde`, `encoding_rs` nor vcs/sync deps; the `Git` tool and patch application need primitives from vcs or code-intel (`imara-diff`); the builtin crate may only call them through sdk traits (open: confirm owner of the patch applier, which the plan lists as a fuzz target).
- VERIFIED (plan): Windows pitfalls apply (reserved names, case clashes, long paths); hostile-path corpora are generated at test time, not committed, to avoid AV quarantine.
- LIKELY: S1 per-step tool pruning breaks prefix caching if it changes the tool list; mitigated as stated in the context crate.
- UNVERIFIED: whether `regex` size limits fully bound compile memory for adversarial patterns; the HIR guard from `polaris-security-automata` is the first line and the fuzz target checks it.
- UNVERIFIED: `schemars 1.2.3` output dialect (JSON Schema 2020-12) is accepted by Nemotron tool calling on Token Factory (S0-5); a schema simplifier may be needed.

**Chatlog sources.** msg 1 (tool spawner: `cargo test`, `git diff`, MCP tools), msg 3 (zero-token tool selection by S1), msg 11 (`polaris_expand`), msg 23 (`polaris_expand(CID)`, repo map, tool folding), msg 27 (Rust `regex` guarantees: linear-time matching, no catastrophic backtracking; user-supplied pattern sizes still need limits), msg 31 (Blast Radius for shell, Proof-of-Done). Corrected: tools write nothing directly (CQRS proposals); shell never on the host; `regex` linear-time guarantee does not remove the memory/size risk.

---

### `polaris-tools-math` — deterministic math tools: exact decimal calculator, interpolation, statistics, GPU occupancy and bandwidth

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-tools/crates/math` | L3 | lib | `polaris-tools` | P3 |

**Purpose.** Models guess at arithmetic, interpolation and GPU launch maths; this crate gives the agent a deterministic alternative with checked arithmetic and explicit error values. The calculator evaluates decimal expressions exactly within `rust_decimal`'s range (28 to 29 significant digits; this is exact decimal, not arbitrary precision). Interpolation and statistics operate on validated finite floats, and the CUDA occupancy and bandwidth calculators read versioned spec tables that must cite an NVIDIA source for every row.

**API sketch.**
```rust
pub struct CalcLimits { pub max_len: usize /*1024*/, pub max_depth: u8 /*64*/, pub max_pow_exponent: u32 /*1000*/ }
pub fn calc(expr: &str, limits: &CalcLimits) -> Result<rust_decimal::Decimal, MathError>;   // + - * / % ^ ( ), unary minus, whitelisted functions (abs, min, max, round, floor, ceil); no variables, no side effects
pub fn calc_f64(expr: &str, limits: &CalcLimits) -> Result<f64, MathError>;                  // exmex-backed float path (evalexpr ≥ 12 is AGPL) for transcendental functions; result must be finite

pub struct Finite(f64);                                  // validated: not NaN, not infinite
pub enum Interp { Linear, Step, CatmullRom, Bezier }     // backed by `splines`; variant set to be confirmed at implementation
pub struct Knot { pub x: Finite, pub y: Finite }
pub fn interpolate(knots: &[Knot], kind: Interp, x: Finite) -> Result<Finite, MathError>;   // knots sorted ascending by x, checked
pub fn lerp(a: Finite, b: Finite, t: Finite) -> Result<Finite, MathError>;
pub fn bilinear(grid: &Grid2, x: Finite, y: Finite) -> Result<Finite, MathError>;
pub fn bezier(control: &[Point2], t: Finite) -> Result<Point2, MathError>;                   // t in 0..=1, checked

pub struct Summary { pub n: u64, pub mean: f64, pub std_dev: f64, pub min: f64, pub max: f64, pub p50: f64, pub p95: f64, pub p99: f64 }
pub fn summarize(xs: &[Finite]) -> Result<Summary, MathError>;                               // statrs-backed; empty input is an error

// GPU calculus: data tables carry provenance; no numbers are hard-coded in code.
pub struct GpuSpecTable { /* rows keyed by ComputeCapability, each with source_url + retrieved date */ }
impl GpuSpecTable { pub fn from_toml(s: &str) -> Result<GpuSpecTable, MathError>; }           // rejects a row without source_url
pub struct ComputeCapability { pub major: u8, pub minor: u8 }
pub struct LaunchConfig { pub threads_per_block: u32, pub regs_per_thread: u32, pub shared_bytes_per_block: u32 }
pub struct Occupancy { pub blocks_per_sm: u32, pub warps_per_sm: u32, pub fraction_milli: u16, pub limiter: Limiter }
pub enum Limiter { Threads, Registers, SharedMemory, BlockSlots }
pub fn occupancy(t: &GpuSpecTable, cc: ComputeCapability, l: &LaunchConfig) -> Result<Occupancy, MathError>;
pub struct GbPerSec(Decimal);                            // decimal gigabytes (1e9) per second; GiB is a different type
pub fn effective_bandwidth(bytes: u64, elapsed_s: Decimal) -> Result<GbPerSec, MathError>;
pub fn peak_bandwidth(data_rate_gtps: Decimal, bus_bits: u32) -> Result<GbPerSec, MathError>; // data rate x bus width / 8
pub enum MathTool { Calc, Interpolate, Summarize, Occupancy, Bandwidth }                    // implements the sdk Tool trait (flag 1)
pub enum MathError { Parse { at: usize }, DivideByZero, Overflow, NonFinite, TooLong, TooDeep, EmptyInput, BadKnots(&'static str), MissingSource(String), UnknownCapability }
```

**Dependencies.** Internal: `polaris-sdk`. External: `exmex = 0.21.0` (replaces `evalexpr = 13.1.0`, AGPL-3.0-only since 12), `rust_decimal = 1.43.0`, `splines = 5.0.0`, `statrs = 0.19.1`; `+proposed serde = 1.0.229`, `toml = 1.1.8` (spec tables); dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `calc` grammar and limits, `interpolate`, `bilinear`, `summarize`, `occupancy`, `effective_bandwidth`, `GpuSpecTable::from_toml` provenance rule.
- [ ] Decide the decimal evaluator by a short in-crate spike: exmex custom operator factories over `Decimal` (if supported) versus a small Pratt parser over `Decimal`; choose the one that passes the fuzz and exactness tests with less code (API to confirm at implementation).
- [ ] Implement `calc` with `checked_*` operations only; overflow, divide-by-zero and exponent limits are errors; reject non-whitelisted identifiers; document the 28-digit limit in the tool description so the model does not assume arbitrary precision.
- [ ] Decide on exact fractions: `rust_decimal` cannot represent 1/3 exactly, so either add a small in-crate `Ratio` with `i128` checked arithmetic or declare rationals out of scope in the tool description (chatlog promised "rational fractions").
- [ ] Implement `Finite` and the interpolation functions; no `as` casts (use `f64::from` and `TryFrom`), no float equality (use `total_cmp` and explicit tolerances), sorted-knot validation.
- [ ] Implement `summarize` with a documented percentile definition and checks for NaN/empty input.
- [ ] Implement `GpuSpecTable` loading and the provenance rule (every row has `source_url` and `retrieved`); the table is filled from NVIDIA docs by `polaris-nvidia-docs` or by hand with citations, never from memory.
- [ ] Implement `occupancy` (limiter analysis for threads, registers, shared memory, block slots) and unit-typed bandwidth functions that return GB/s (decimal), fixing the chatlog's TB/s label.
- [ ] Implement `MathTool` schemas (JSON Schema via schemars in sdk) and golden `insta` snapshots of the tool descriptions.

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | `0.1 + 0.2 == 0.3` exactly in `calc`; `1/0` errors; `2^1001` over exponent limit errors; knots unsorted error; occupancy limiter identified for hand-computed cases |
| proptest | for decimal operands `calc("a op b")` equals the `Decimal` operation or the same error; `interpolate` at a knot returns the knot's y (Linear, CatmullRom); Bezier output stays inside the control-point bounding box; `summarize` min <= p50 <= p95 <= p99 <= max; occupancy fraction never exceeds 1000 milli |
| fuzz | `fuzz_calc_expr` (arbitrary text; never panics or hangs), `fuzz_gpu_table_toml` |
| kani | occupancy integer arithmetic (blocks per SM) has no overflow for any `u32` inputs within hardware limits |
| miri | whole crate (pure) |
| security | expression bombs (`((((...))))` x 10 000, `9^9^9^9`, 1 MB string) rejected in bounded time and memory |
| load | 1 M `calc` evaluations/s for short expressions (criterion, target) |
| fixtures | `tests/fixtures/gpu_specs.toml` with real source URLs once captured (a synthetic table is labelled synthetic and cannot be used by the shipping binary) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Exact decimal calculator | function | `calc` | `crates/polaris-tools/crates/math/src/calc.rs` | `point_one_plus_point_two_is_exact`, `divide_by_zero_is_error`, `overflow_is_error_not_wrap`, `deep_nesting_rejected`, `unknown_identifier_rejected`, `huge_exponent_rejected` |
| Interpolation | function | `interpolate` | `crates/polaris-tools/crates/math/src/interp.rs` | `linear_hits_knots`, `catmull_rom_passes_through_control_points`, `unsorted_knots_rejected`, `x_outside_range_is_error_or_clamped_per_policy`, `non_finite_input_rejected` |
| Statistics | function | `summarize` | `crates/polaris-tools/crates/math/src/stats.rs` | `empty_input_is_error`, `percentiles_are_ordered`, `nan_input_rejected`, `single_value_has_zero_std_dev` |
| CUDA occupancy | function | `occupancy` | `crates/polaris-tools/crates/math/src/gpu.rs` | `register_limited_launch_reports_registers`, `shared_memory_limited_launch_reports_shared`, `unknown_capability_is_error`, `row_without_source_url_rejected` |
| Bandwidth units | function | `effective_bandwidth` | `crates/polaris-tools/crates/math/src/bandwidth.rs` | `returns_gigabytes_per_second`, `zero_elapsed_is_error`, `gib_and_gb_are_distinct_types` |

**Risks & open questions.**
- VERIFIED (PLAN core): evalexpr has no arbitrary precision, hence `rust_decimal`. Stated precisely here: `rust_decimal` is a 96-bit-mantissa fixed-scale decimal (about 28 digits), also not arbitrary precision; overflow must be an explicit error.
- VERIFIED (read in chatlog msg 27): the chatlog `calculate_h100_bandwidth` divided by `1e9` (GB/s) while documenting "TB/s", used `as f64`, and `lerp` had no input checks; fixed by typed units, checked ops and `Finite`.
- UNVERIFIED: GPU per-SM limits for sm_75 (local GTX 1650) and sm_90 (H100) are not asserted here; they must be captured from the CUDA programming guide tables with URL and date before use (the `nvidia-docs` ingestion in P5 can supply them).
- UNVERIFIED: `splines 5.0.0` and `exmex 0.21.0` exact APIs (interpolation enum, custom value types); confirm at implementation. (evalexpr was dropped for its AGPL-3.0-only licence from 12.0.)
- OPEN: `MathTool` needs the `Tool` trait in sdk (flag 1).

**Chatlog sources.** msg 27 (section 2: deterministic math tooling, exact evaluator, interpolation engine (spline, linear/bilinear, Bezier), NVIDIA hardware calculus incl. grid alignment, `MathEngine`, Rust `regex` guarantees). Corrected: arbitrary precision claim; units; casts; `Value` returned from evalexpr replaced by typed results; rational fractions flagged.

---

### `polaris-tools-rtk` — RTK diagnostic engine: compiler/linter stream filter and atomic issue ledger

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-tools/crates/rtk` | L3 | lib | `polaris-tools` | P1 |

**Purpose.** A Rust port of the reference RTK engine (`@prime/tmp/setup/pipeline-harness/src/tools/rtk/rtk_engine.cpp` v3.1.0, read in full for this plan; not executed). It reads a compiler or linter stream, auto-detects cargo JSON versus text, drops registry notes, macro-expansion spam and duplicate spans, keeps first-party errors, and reconciles them against an atomic ledger `.polaris/issues.md` with stable message-derived ids. RTK is separate from `§ref` folding: it produces the clean diagnostic view that context folding, Proof-of-Done (open issue count) and the self-correction loop consume. The ledger reflects only the last stream piped in; it is not a statement about the current tree.

**Reference behaviour to preserve (VERIFIED by reading the C++ source and the SKILL.md / cli_options.md; none of it was run).**
- Modes: `--auto` (default: detect, print filtered diagnostics, reconcile, summary), `--json` (print structured report, ledger untouched), `--filter` (clean text, ledger untouched), `--issues` (reconcile and print summary only); `--issues-file <path>`; exit 0 when no issue is open after reconciliation, 1 otherwise. The exit code reflects the ledger, not the build's own exit code.
- Detection: first non-whitespace character `{` means JSON.
- JSON mode: only lines with `reason == "compiler-message"` and `message.level == "error"`; code from `message.code.code`; the primary span (`is_primary`) is followed through `expansion.span` chains to the outer call site; file, line, column from that span; `children[].message` become details; report `{status SUCCESS|FAILED, count, errors[{code, message, location{file,line,column}, details[]}]}` (the reference names the count field `vulnerabilities_count`; the port uses `error_count`).
- Text mode: split into blocks starting at `error[E`, `error:` or `warning:` (`error: could not compile` is not a block start); if any error block exists, warning-only blocks are dropped; lines mentioning `.cargo/registry` or `.cargo/git` are dropped; `help:` and `note:` parts and `-->` paths are kept with consecutive-duplicate suppression; code-frame lines containing `|` or `^` are dropped.
- Ledger lines: open `- [ ] <text> | ID: <id> | Opened: <ts>`, closed `- [x] ~~<text>~~ | ID: <id> | Opened: <ts> | Closed: <ts>`; id derived from the message text (reference: FNV-1a 64 as `#ERR-` plus 16 hex); unchanged errors stay open; vanished errors are closed with a timestamp; new errors are appended; non-issue lines are preserved verbatim; write to a temporary file in the same directory then rename; a legacy root-level `ISSUES.md` is migrated once.

**API sketch.**
```rust
pub enum RtkMode { Auto, Json, Filter, Issues }
pub enum StreamFormat { CargoJson, Text, Mixed }                   // Mixed: stderr text interleaved with JSON lines (see risks)
pub fn detect_format(stream: &[u8]) -> StreamFormat;
pub struct Diagnostic { pub level: Level, pub code: Option<String>, pub message: String, pub primary: Option<Location>, pub details: Vec<String>, pub first_party: bool }
pub struct Location { pub file: String, pub line: u32, pub column: u32 }
pub fn parse_diagnostics(stream: &[u8], fmt: StreamFormat) -> Result<Vec<Diagnostic>, RtkError>;
pub fn filter_text(stream: &[u8]) -> Result<String, RtkError>;      // --filter
pub fn render_json_report(d: &[Diagnostic]) -> Result<String, RtkError>; // --json

pub struct IssueId(String);                                         // "#ERR-" + 16 hex from blake3 of the normalised message
pub struct Ledger { /* ordered lines: Issue(open|closed) or Raw(String) */ }
pub struct LedgerSummary { pub open: Vec<IssueLine>, pub newly_closed: u32, pub newly_opened: u32 }
impl Ledger {
    pub fn parse(text: &str) -> Ledger;                              // tolerant: unknown lines kept as Raw
    pub fn reconcile(&mut self, current: &[Diagnostic], now: jiff::Timestamp) -> LedgerSummary;
    pub fn render(&self) -> String;                                  // LF line endings
}
pub struct LedgerFile { /* resolved path */ }
impl LedgerFile {
    pub fn resolve(explicit: Option<&std::path::Path>, workspace: &std::path::Path) -> LedgerFile; // env POLARIS_ISSUES_FILE (proposed name; reference uses HARNESS_ISSUES_FILE), then explicit, then <root>/.polaris/issues.md
    pub async fn load(&self) -> Result<Ledger, RtkError>;
    pub async fn save_atomic(&self, l: &Ledger) -> Result<(), RtkError>; // unique tmp in the same dir, rename, bounded backon retry on Windows PermissionDenied
}
pub fn exit_code(s: &LedgerSummary) -> i32;                          // 0 if no open issues, else 1
pub async fn run(mode: RtkMode, stream: &[u8], file: &LedgerFile, now: jiff::Timestamp, store: &impl ContentStore) -> Result<RtkOutput, RtkError>; // raw stream is stored in CAS first (nothing is lost)
pub struct RtkOutput { pub filtered: String, pub summary: Option<LedgerSummary>, pub raw: Cid, pub exit_code: i32 }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-cas`. External: `serde_json = 1.0.154` (with serde_json's default recursion limit, replacing the reference's unbounded recursive DOM parser), `blake3 = 1.8.7`, `backon = 1.6.0`, `jiff = 0.2.38`; `+proposed tempfile = 3.27.0` (unique temp file in the ledger directory), `tokio = 1.54.0` or sync `std::fs` behind `spawn_blocking`; dev: `proptest = 1.12.0`, `insta = 1.49.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `detect_format`, `parse_diagnostics` (JSON and text), `filter_text`, `Ledger::reconcile`, `LedgerFile::save_atomic`, `exit_code`.
- [ ] Port the JSON path with `serde_json` line by line (skip non-JSON lines instead of failing), including the `expansion.span` chain walk with a depth cap, and test against recorded cargo output (`--message-format=json`) for E0425, E0308, macro-expanded errors and multi-span errors.
- [ ] Port the text segmenter and filters; normalise path separators before matching `.cargo/registry` and `.cargo/git` so Windows paths (`C:\Users\x\.cargo\registry\...`) are dropped too (the reference matches forward slashes only); honour a custom `CARGO_HOME`.
- [ ] Add `Mixed` detection: JSON lines are parsed, other lines go through the text path, so `2>&1` output that starts with a stderr `Compiling ...` line is not misclassified as plain text (the reference decides on the first character only).
- [ ] Port ledger parse / reconcile / render: id from the normalised message, open/closed transitions, timestamp on close, `Raw` lines preserved, new errors appended; exclude `error: could not compile ...` and `error: aborting due to ...` lines from the ledger (the reference text extractor includes them although its block segmenter excludes them).
- [ ] Implement atomic save: unique temp file in the ledger directory, flush, rename over the target, bounded `backon` retry for transient `PermissionDenied` (antivirus or indexer locks), cleanup of the temp file on failure; detect a concurrent writer by comparing the ledger hash read before with the file at rename time and retry reconcile once.
- [ ] Store the raw stream in CAS before filtering and return its `Cid` so the filtered view is never the only copy.
- [ ] Implement ledger path resolution (`POLARIS_ISSUES_FILE` as a proposed name, explicit path, then workspace root found by `.git` or `.polaris`), and the one-time migration of a legacy `ISSUES.md`.
- [ ] Expose a stable text rendering for the agent: `{open count, first-party errors with file:line, ledger summary, honesty note "reflects the last stream piped in"}`.
- [ ] Windows specifics: CRLF and CR-only input lines, UTF-8 BOM at stream start, UTF-16 stream from PowerShell redirection (detect BOM, transcode), ledger written with LF (the repository `.gitattributes` uses LF).

**Tests.**
| Category | Cases / budgets (targets) |
|---|---|
| unit | JSON error with macro expansion reports the outer call site; registry note dropped on Windows-style paths; consecutive duplicate `note:` dropped; reconcile opens, keeps and closes issues; `Raw` lines preserved byte-for-byte; exit code 1 with one open issue |
| proptest | `render(parse(x))` is a fixed point for ledgers produced by the engine; reconciling the same diagnostics twice is idempotent (ids and open timestamps unchanged); issue id depends only on the normalised message; filtering never invents text (every output line is a substring of some input line) |
| fuzz | `fuzz_rtk_filter_text`, `fuzz_rtk_filter_json` (deeply nested JSON must not overflow the stack), `fuzz_rtk_ledger_parse` |
| smoke | pipe a real `cargo build` failure from a fixture crate through `run` in a temp workspace: ledger created, exit 1; fix the code, rerun: issue closed with a timestamp, exit 0 |
| load | filter 100 MB of log text at >= 200 MB/s and parse 100 MB of cargo JSON at >= 100 MB/s (targets, criterion); not part of the zero-alloc closed set |
| stress | two concurrent writers on one ledger: no corrupt file, no lost closed history (checked by hash comparison) |
| security | malicious compiler output (very long lines, control characters, terminal escape sequences, fake `ID:` / `Closed:` fields in a message) cannot forge ledger metadata or corrupt the table; escape sequences stripped from the filtered view |
| fixtures | `tests/fixtures/rtk/`: cargo JSON and text from real runs (recorded with rustc version, labelled), CRLF/BOM/UTF-16 variants generated at test time, a hand-written ledger with notes |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | tests |
|---|---|---|---|---|
| Format detection | function | `detect_format` | `crates/polaris-tools/crates/rtk/src/detect.rs` | `leading_brace_is_json`, `leading_whitespace_then_brace_is_json`, `stderr_noise_then_json_is_mixed`, `plain_text_is_text`, `empty_input_is_text` |
| Cargo JSON diagnostics | function | `parse_diagnostics` | `crates/polaris-tools/crates/rtk/src/json.rs` | `error_with_code_and_primary_span`, `macro_expansion_resolves_to_call_site`, `warnings_are_not_issues`, `non_json_lines_are_skipped`, `deeply_nested_json_is_rejected_not_overflowed` |
| Text filter | function | `filter_text` | `crates/polaris-tools/crates/rtk/src/text.rs` | `registry_paths_dropped_on_windows_separators`, `warning_blocks_dropped_when_errors_exist`, `could_not_compile_line_not_a_block`, `consecutive_duplicates_collapsed`, `code_frames_dropped` |
| Ledger reconcile | method | `Ledger::reconcile` | `crates/polaris-tools/crates/rtk/src/ledger.rs` | `new_error_appended_open`, `vanished_error_closed_with_timestamp`, `unchanged_error_keeps_original_open_time`, `reconcile_is_idempotent`, `non_issue_lines_preserved`, `abort_summary_lines_are_not_issues` |
| Atomic ledger write | async_function | `LedgerFile::save_atomic` | `crates/polaris-tools/crates/rtk/src/file.rs` | `interrupted_write_leaves_old_ledger`, `transient_permission_denied_is_retried`, `temp_file_removed_on_failure`, `concurrent_writer_detected` |

**Risks & open questions.**
- VERIFIED (read in source): the reference decides JSON versus text by the first non-whitespace character only, matches registry markers with forward slashes only, includes `error: could not compile` lines in ledger extraction, writes one fixed temp name (`issues.md.tmp`), writes LF regardless of input, uses local time `%Y-%m-%d %H:%M:%S` in code but ISO-8601 `...Z` in the SKILL.md example, and documents ids as 8 hex in SKILL.md but derives 16 hex (`#ERR-` + FNV-1a 64) in code. The port fixes the first four and must pick one timestamp format (proposal: RFC 3339 UTC, second precision, injectable clock) and one id format (proposal: `#ERR-` + 16 hex of blake3).
- VERIFIED (design property of the reference): the id depends on the message text only, so identical messages in different files collapse into one ledger entry; keeping this keeps ids stable when lines shift, at the cost of merging same-text errors. OPEN: include the file path in the id?
- UNVERIFIED: that `std::fs::rename` replaces an existing file on Windows under all conditions (LIKELY yes); a rename failure is retried and then reported.
- UNVERIFIED: cargo's exact JSON shape across rustc versions (fields beyond those the reference reads); the parser ignores unknown fields.
- No spike covers RTK; reference parity is checked by running both implementations on the same recorded fixtures (the C++ reference binary is not built here; UNVERIFIED that it builds on this machine).

**Chatlog sources.** msg 23 (section 2.1: deterministic folding of compiler logs to unique error codes, files and spans; `compress_compiler_diagnostics`), msg 31 (S1 post-flight uses the compressed diagnostics), msg 1 (section 9: compile loop feeding stderr back to Nemotron). The RTK spec itself comes from the rtk-engine skill and its source under `@prime/tmp/setup`, not from the chatlog. Corrected: the chatlog's line-substring filter (`contains("error[")`) is replaced by the reference's block segmentation; the ledger is a persistent state machine, not a per-call summary.

---

---

## 13.3 Group G3 — Execution plane & cloud

**Group conventions (apply to every crate below; not repeated per crate).**
- Safe Rust only; no `unwrap` / `expect` / `panic!` / slice indexing; every fallible fn returns `Result<_, <Crate>Error>` where `<Crate>Error` is a `thiserror` enum; no lint attributes anywhere (D10).
- Versions are copied from `@prime/tools/workspace.toml` `[[ext]]`. Where a sketch needs a crate that the crate's `ext = [...]` list in `workspace.toml` does not yet contain, it is marked `+proposed` (parent must reconcile `workspace.toml` before `gen`). The commonly missing ones are `thiserror = 2.0.21`, `serde = 1.0.229` (features `derive`), `tokio = 1.54.0`.
- Layer rule used throughout: an L2 adapter depends only on `polaris-sdk`, L1 crates and its own external library, never on a sibling adapter of another parent. Cross-adapter wiring (for example "run a command in WSL2") is done with sdk traits and generics, and bound at the composition root (`polaris-kernel` facade, L5).
- Label vocabulary: VERIFIED (checked this session against live output or docs), LIKELY (consistent with context, not checked), UNVERIFIED (open; covered by the named spike). Nothing in this file has been run or tested; every test row is a plan, and every number is a target to measure.
- Descriptor file paths are the planned implementation paths. Generated test files go to `<crate>/tests/<logic_name_snake_case>.rs` (prompt.md §6.3) and start `#[ignore]`d until implemented.

---

### `polaris-platform` — facade over the OS-facing adapters (WSL2 management, process/PTY/fs-watch)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-platform` | L2 | facade | (top-level) | P2 |

**Purpose.** Pure re-export crate; contains no logic. Owns the documentation of how Windows, Linux and macOS differ for every capability the children expose, so the later macOS/Linux port (and the Linux `cargo check` leg in WSL2 from P1) has one checklist.
**Re-exports.** `pub use polaris_platform_wsl as wsl;` `pub use polaris_platform_process as process;` (names per `workspace.toml`: `pub use polaris_platform_*`). Children never depend on this facade.
**Behavioral-difference table to keep in the crate docs.**

| Capability | Windows (primary, native) | Linux / WSL2 | macOS (later) |
|---|---|---|---|
| Process-tree kill | Job Object (`KILL_ON_JOB_CLOSE`) via `process-wrap` | process group / (cgroup.kill only inside sandboxes, not on the Windows host) | process group |
| Graceful stop | CTRL_BREAK only for processes created in a new process group and sharing a console (otherwise kill); there is no SIGTERM | SIGTERM, grace, SIGKILL | same as Linux |
| PTY | ConPTY via `portable-pty` | openpty | openpty |
| FS watch | `ReadDirectoryChangesW` (buffer overflow possible) | inotify (does not see Windows-side changes under `/mnt/c`) | FSEvents |
| Distro management | `wsl.exe` (UTF-16LE list output) | n/a | n/a |

**TODO.**
- [ ] Re-export both children; add a compile-only doctest that names one public item from each child.
- [ ] Write the difference table above into `//!` docs and keep it in sync with child TODOs when behavior is confirmed (mark each cell VERIFIED / LIKELY).
- [ ] Add a `check-layers` expectation: no child depends on `polaris-platform` (mechanical gate, `xtask`).
- [ ] Record in the docs which child owns each sdk `PlatformOps` method (process child owns it; wsl child does not implement `PlatformOps`).

---

### `polaris-platform-wsl` — WSL2 distro detection/provisioning, exec in distro, path translation, gateway keepalive

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-platform/crates/wsl` | L2 | lib | `polaris-platform` | P2 |

**Purpose.** Everything the Windows host needs to know about and do with WSL2: detect or provision Ubuntu 24.04, validate `.wslconfig`, run commands in the distro without shell-quoting accidents, translate Windows and Linux paths, report `/dev/kvm` for the OpenShell `vm` driver, and keep the WSL2 VM (and thus the OpenShell gateway) alive. It never installs or reboots anything without an explicit, typed user confirmation. Current machine state (VERIFIED earlier this session): WSL2 has only `docker-desktop`, no Ubuntu distro, no `.wslconfig`.

**API sketch.**
```rust
pub struct DistroName(/* validated: 1..=64 of [A-Za-z0-9._-] */ String);
pub struct LinuxPath(/* absolute POSIX path, no NUL */ String);
pub struct WinPath(std::path::PathBuf);

pub struct Stopped; pub struct Running;               // typestate markers
pub struct Distro<S> { /* name, wsl handle */ _s: core::marker::PhantomData<S> }

pub struct Wsl { /* absolute path of wsl.exe resolved via `which` */ }
impl Wsl {
    pub fn locate() -> Result<Wsl, WslError>;
    pub async fn doctor(&self) -> Result<WslReport, WslError>;
    pub async fn list(&self) -> Result<Vec<DistroInfo>, WslError>;
    /// Never elevates and never reboots; returns NeedsElevation / NeedsReboot as data.
    pub async fn provision_ubuntu(&self, plan: ProvisionPlan, consent: UserConsent) -> Result<ProvisionOutcome, WslError>;
    pub async fn open(&self, name: &DistroName) -> Result<Distro<Stopped>, WslError>;
}
impl Distro<Stopped> { pub async fn start(self, ka: KeepAlivePolicy) -> Result<Distro<Running>, WslError>; }
impl Distro<Running> {
    pub async fn exec(&self, spec: &ExecSpec) -> Result<ExecOutput, WslError>;   // argv form, no shell
    pub async fn kvm(&self) -> Result<KvmStatus, WslError>;
    pub fn keepalive(&self) -> KeepAliveStatus;
    pub async fn stop(self) -> Result<Distro<Stopped>, WslError>;
}
pub fn to_linux_path(p: &WinPath) -> Result<LinuxPath, WslError>;
pub fn to_windows_path(p: &LinuxPath, d: &DistroName) -> Result<WinPath, WslError>;
pub fn validate_wslconfig(text: &str) -> Result<WslConfigReport, WslError>;
pub fn decode_wsl_output(bytes: &[u8]) -> Result<String, WslError>;              // UTF-16LE or UTF-8
pub enum KeepAlivePolicy { ChildProcess, SystemdUnit, Both }
pub enum WslError { NotInstalled, DistroMissing(DistroName), Decode, Exit { code: i32, stderr: String },
                    Timeout, KvmUnavailable, NeedsElevation, NeedsReboot, ConfigInvalid { key: String }, Io(std::io::Error) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `process-wrap = 10.0.1` (feature `tokio1`), `which = 8.0.6`; `+proposed`: `thiserror = 2.0.21`, `dunce = 1.0.5` (verbatim `\\?\` prefix handling).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `decode_wsl_output`, `to_linux_path`/`to_windows_path`, `validate_wslconfig`, `Wsl::doctor`, `Distro::exec` argv builder, `KeepAlive` supervisor.
- [ ] Implement `decode_wsl_output` (UTF-16LE with BOM/NULs, UTF-8 when `WSL_UTF8=1` is honored) and parse `wsl.exe -l -v` / `--status` / `--version` into typed values; fixtures are synthetic until captured on the real machine.
- [ ] Implement path translation: `C:\a\b` <-> `/mnt/c/a/b`, UNC `\\wsl.localhost\<distro>\...`, reject verbatim/device paths, trailing dots/spaces, drive-relative paths, and embedded NUL; case handling documented (drvfs case-insensitivity).
- [ ] Implement `validate_wslconfig`: parse `%USERPROFILE%\.wslconfig`, check `[wsl2] memory` (target about 6 GB on this 16 GB machine) and `nestedVirtualization=true`; propose a diff, write only with explicit consent plus a backup copy.
- [ ] Implement `exec` with `wsl.exe -d <distro> --cd <dir> --exec <argv...>` (no shell), timeouts, output caps, and child-tree kill through `process-wrap`.
- [ ] Implement `kvm()` (`/dev/kvm` exists, readable by the sandbox user, `kvm` group) feeding the E2 decision (spike S0-2).
- [ ] Implement the keepalive: held `wsl.exe --exec sleep infinity` child (kill-on-drop) and/or the systemd unit for the gateway; periodic health probe; restart with backoff.
- [ ] Implement `provision_ubuntu` as a plan-then-confirm flow (`wsl.exe --install -d Ubuntu-24.04` is an interactive/elevated operation: surface `NeedsElevation`/`NeedsReboot`, never auto-elevate). BLOCKED-ON-ACCESS (admin rights on this machine to install the Ubuntu 24.04 distro; not done yet).
- [ ] Warn before any `wsl --shutdown` that it also stops the `docker-desktop` distro (Docker Desktop's engine) and every other distro.
- [ ] Spike S0-1 support: expose a `probe_gateway_reachability(host_port)` helper that tests Windows -> WSL2 TCP forwarding (localhost forwarding vs mirrored networking mode).
- [ ] Doctor output feeds `polaris doctor` / `/onboarding` through `WslReport` (serializable, no secrets).

**Tests.**

| Category | Cases |
|---|---|
| unit | UTF-16LE list parsing with BOM, default-distro `*` marker, `Stopped`/`Running` states; `.wslconfig` key validation (unknown keys, units `GB`/`MB`, duplicate sections); error mapping from exit codes |
| proptest | `to_windows_path(to_linux_path(p)) == p` for generated drive/UNC paths; no output ever contains `..` escape from `/mnt/<drive>`; argv builder never produces a shell metacharacter interpretation |
| fuzz | `decode_wsl_output` and `.wslconfig` parser (WSL2 nightly) |
| smoke | on this Windows machine: `doctor()` returns `NoUbuntu` today and `Healthy` after P0 setup; `exec echo` round-trip (needs Ubuntu: BLOCKED-ON-ACCESS) |
| contract | recorded real `wsl.exe` output fixtures (replace synthetic ones once captured; label which are synthetic) |
| spike | S0-1 (gateway reachability), S0-2 (`/dev/kvm` under nested virtualization on Ryzen 7 2700) |
| stress | 50 sequential `exec` calls with 1 MB output; kill `wsl.exe` mid-exec and confirm no orphan; WSL VM idle-shutdown with and without keepalive (measure) |
| security | hostile argv (spaces, quotes, `$()`, unicode, trailing backslash, 32 KB args); path with `\\?\`; `.wslconfig` write refuses symlinked target |
| fixtures | hostile Windows paths generated at test time (never committed) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| WSL Output Decoding | function | `decode_wsl_output` | `crates/polaris-platform/crates/wsl/src/decode.rs` | `decodes_utf16le_with_bom`; `decodes_utf8_when_flagged`; `rejects_odd_length_utf16`; `strips_embedded_nul_noise`; `never_panics_on_arbitrary_bytes` |
| WSL Path Translation | function | `to_linux_path` / `to_windows_path` | `.../wsl/src/paths.rs` | `drive_path_maps_to_mnt`; `unc_wsl_localhost_roundtrips`; `rejects_verbatim_prefix`; `rejects_drive_relative`; `roundtrip_property_holds` |
| WSL Config Validation | function | `validate_wslconfig` | `.../wsl/src/config.rs` | `accepts_memory_6gb_and_nested_true`; `flags_missing_nested_virtualization`; `rejects_unknown_unit`; `preserves_comments_in_diff`; `empty_file_is_valid_with_warnings` |
| WSL Keepalive | struct | `KeepAlive` | `.../wsl/src/keepalive.rs` | `restarts_child_after_exit`; `backoff_is_bounded`; `drop_kills_child_tree`; `reports_degraded_after_n_failures` |

**Risks & open questions.**
- UNVERIFIED: whether `nestedVirtualization` exposes a usable `/dev/kvm` on this Ryzen 7 2700 under Windows 11 LTSC 2024 (S0-2). Fallback is E4 (remote gateway).
- UNVERIFIED: exact WSL idle-shutdown behavior and the `.wslconfig` `vmIdleTimeout` key on this WSL build; keepalive design is a hedge (S0-1 measures it).
- LIKELY: `wsl.exe` list output is UTF-16LE on this build; `--exec` skips the shell. Both must be confirmed from fixtures captured on the machine.
- VERIFIED (earlier this session): only `docker-desktop` is installed; changing `.wslconfig` needs `wsl --shutdown`, which interrupts Docker Desktop.
- UNVERIFIED: whether the OpenShell docker driver inside Ubuntu can use Docker Desktop's WSL integration or needs its own engine (decide in P0; affects `doctor`).
- Windows Defender / EDR may slow `wsl.exe` spawns; budget tests use medians over runs, not single samples.

**Chatlog sources.** msg 1 §10 (cross-platform table: Windows sandbox "Job Objects / AppContainer / WSL2"); msg 1 §4.1 (Windows sandbox mechanisms). Corrected by D1: the Windows execution plane is an OpenShell gateway in WSL2 Ubuntu 24.04, not native Job-Object sandboxing; AppContainer is only the optional E1w `mxc` tier (S0-11). msg 27 §5 (reaper: teardown of VM handles) adopted for the keepalive/stop path.

---

### `polaris-platform-process` — process trees, PTY, fs watching and memory pressure; implements `PlatformOps`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-platform/crates/process` | L2 | lib | `polaris-platform` | P2 |

**Purpose.** The single implementation point (prompt.md §6.2) of the sdk `PlatformOps` trait: spawn child processes inside kill-on-drop trees, stop them gracefully where Windows allows it and forcefully otherwise, run PTY sessions for the terminal tool and `AttachTerminal`, watch the workspace (treating watcher overflow as a forced rescan), and sample memory pressure for model eviction. It is the only place that touches OS process APIs; everything else uses the trait.

**API sketch.**
```rust
pub struct EnvPolicy { /* allow-list; secret-looking names (NEBIUS_API_KEY, TAVILY_API_KEY, *_TOKEN) scrubbed by default */ }
pub struct SpawnSpec { pub program: AbsProgram, pub args: Vec<OsString>, pub cwd: PathBuf, pub env: EnvPolicy, pub stdio: StdioMode, pub stop: StopPolicy }
pub struct AbsProgram(PathBuf);                    // produced by `resolve_program` (uses `which`); never a bare name
pub struct Spawned; pub struct Exited;             // typestate for ProcessTree<S>
pub struct ProcessTree<S> { /* job object / process group handle */ _s: core::marker::PhantomData<S> }
impl ProcessTree<Spawned> {
    pub fn pid(&self) -> u32;
    pub async fn wait(self) -> Result<(ProcessTree<Exited>, ExitInfo), ProcessError>;
    pub async fn stop(self, policy: StopPolicy) -> Result<(ProcessTree<Exited>, StopOutcome), ProcessError>;
}
pub enum StopOutcome { Graceful(ExitInfo), Killed, AlreadyExited }
pub struct Pty { /* ConPTY / openpty master */ }
pub fn spawn_pty(spec: &SpawnSpec, size: PtySize) -> Result<Pty, ProcessError>;
impl Pty { pub fn output(&mut self) -> PtyOutput /* tokio mpsc receiver */; pub async fn write(&self, b: &[u8]) -> Result<(), ProcessError>; pub fn resize(&self, s: PtySize) -> Result<(), ProcessError>; }
pub fn watch(root: &Path, cfg: WatchConfig) -> Result<FsWatcher, ProcessError>;
pub enum FsEvent { Changed(Vec<PathBuf>), RescanRequired { reason: RescanReason } }
pub struct MemorySampler; impl MemorySampler { pub fn sample(&mut self) -> Result<MemorySnapshot, ProcessError>; }
pub enum Pressure { Normal, Elevated, Critical }
pub struct ProcessPlatform; /* impl polaris_sdk::PlatformOps for ProcessPlatform */
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-log`. External: `process-wrap = 10.0.1` (feature `tokio1`), `portable-pty = 0.9.0`, `notify = 8.2.0`, `notify-debouncer-full = 0.7.0`, `sysinfo = 0.39.6` (`default-features = false`, features `system`, `disk`), `tokio = 1.54.0`; `+proposed`: `thiserror = 2.0.21`, `which = 8.0.6`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `resolve_program`, `EnvPolicy::apply`, `ProcessTree::stop`, `spawn_pty`, `watch` overflow handling, `MemorySampler::pressure`.
- [ ] Implement `resolve_program` (absolute path via `which`; reject relative names and paths in the current directory; flag `.bat`/`.cmd` targets and refuse untrusted args for them unless explicitly allowed).
- [ ] Implement `EnvPolicy`: inherit an allow-list only; scrub Polaris and cloud secrets from every child by default; sandbox-bound commands never inherit host secrets.
- [ ] Implement spawn inside a Job Object with kill-on-close (`process-wrap`), plus the Unix process-group variant for the Linux leg; keep the process-wrap API usage behind one private adapter.
- [ ] Implement `stop`: grace period then kill; try CTRL_BREAK only where `process-wrap` offers a safe path (see open question about `unsafe`); otherwise kill the Job Object. Report which path was taken.
- [ ] Implement PTY sessions over ConPTY (`portable-pty`), moving blocking reads onto dedicated threads that feed a bounded async channel; resize; UTF-8 boundary-safe chunking.
- [ ] Implement `watch` with debouncing; map watcher overflow and "rescan" flags to `FsEvent::RescanRequired`; default-ignore `.git/`, `target/`, `.polaris/`.
- [ ] Implement `MemorySampler` and `Pressure` thresholds (config-driven); optionally track the WSL VM's host-side memory footprint (process name to be confirmed on the machine).
- [ ] Implement `impl PlatformOps for ProcessPlatform` and a `FakePlatform` for tests in `polaris-tests` (declared test double, not a stub in production code).
- [ ] Add the Linux leg (`cargo check` in WSL2 Ubuntu) once Ubuntu exists. BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 not installed yet).

**Tests.**

| Category | Cases |
|---|---|
| unit | `EnvPolicy` scrubbing; `StopPolicy` transitions; pressure threshold edges; `resolve_program` rejection table |
| proptest | env names matching secret patterns are always scrubbed; arg round-trip through spawn preserves bytes for generated unicode/quote args (Windows quoting) |
| smoke | spawn `cmd /c exit 3`, read exit code; spawn a PTY and echo a line; watch a temp dir and see a create event |
| stress | 1,000 spawn/stop cycles with no leaked handles (measure handle count before/after); grandchild tree of depth 5 killed on drop; flood of 100k file events forces `RescanRequired` rather than silent loss |
| load | PTY throughput with 100 MB of output (target to measure; bounded memory) |
| scalability | watcher on 1k / 10k / 100k-file trees: startup time and event latency (targets to measure) |
| security | child env never contains `NEBIUS_API_KEY`; `.bat` argument injection corpus rejected; zombie/orphan check after parent crash simulation |
| spike | feeds the S0-10 decision indirectly (ConPTY input path); no dedicated spike |
| fixtures | process-tree fixtures built at test time; no binaries committed |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Program Resolution | function | `resolve_program` | `crates/polaris-platform/crates/process/src/resolve.rs` | `resolves_cargo_to_absolute_path`; `rejects_relative_name`; `rejects_cwd_shadowing_binary`; `flags_cmd_and_bat_targets`; `missing_program_is_typed_error` |
| Environment Policy | struct | `EnvPolicy` | `.../process/src/env.rs` | `scrubs_api_key_names`; `inherits_only_allow_list`; `explicit_override_wins_but_is_logged_redacted`; `case_insensitive_on_windows` |
| Process Tree Stop | method | `ProcessTree::stop` | `.../process/src/tree.rs` | `graceful_exit_within_grace_reports_graceful`; `unresponsive_child_is_killed`; `grandchildren_die_with_parent`; `double_stop_is_already_exited` |
| FS Watch Overflow | function | `watch` | `.../process/src/watch.rs` | `create_event_is_debounced`; `overflow_emits_rescan_required`; `ignored_dirs_produce_no_events`; `rename_is_reported_as_single_change` |

**Risks & open questions.**
- UNVERIFIED: `process-wrap` 10.0.1 may not expose a safe graceful CTRL_BREAK; sending it normally needs a Win32 call (`unsafe`). The ADR unsafe-exception list (ConPTY Shift+Enter, named-pipe descriptor, memory locking, fuzz macros) does not cover it, so the default here is kill-after-grace and a recorded decision if graceful stop proves necessary. Confirm at implementation.
- LIKELY: `portable-pty` blocking readers require thread bridging; the exact async story is to confirm at implementation.
- LIKELY: `inotify` inside WSL2 does not observe Windows-side edits under `/mnt/c`, so the workspace is watched on Windows only; sandbox-side changes return through the sync protocol.
- VERIFIED (plan): notify overflow must force a rescan; the "treat overflow as rescan" test is mandatory.
- Windows batch-file argument handling is a known injection class; treated as a security test, not assumed safe.

**Chatlog sources.** msg 1 §10 (PTY via `windows-sys` ConPTY; watcher `ReadDirectoryChangesW`; sandbox via Job Objects) - adopted but `windows-sys` and `nix` are replaced by `portable-pty` / `process-wrap` to keep our own `unsafe` at zero. msg 27 §5 (Reaper matrix: `TerminateJobObject` on Windows, SIGTERM -> 200 ms -> kill) - adopted; "cgroup.kill" corrected as Linux-sandbox-only. msg 17 (EMFILE / disk I/O semaphore) - watch and walk limits fed to the disk semaphore. msg 35 (`AttachTerminal` full-duplex PTY stream) - this crate provides the PTY. Chatlog "`catch_unwind` for isolation" corrected: OS-level process isolation and tokio `JoinError` handle crashes (D3).

---

### `polaris-vcs` — git integration (gix with git CLI fallback): status, diff, worktrees, `/rollback`, `/undo`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-vcs` | L2 | lib | (top-level) | P2 |

**Purpose.** All repository-level version control for the agent: compute status and unified diffs for `/diff` and the verifier, give each speculative frame an isolated worktree, and provide two rollback granularities: `/undo` (last agent action, from pre-edit CAS snapshots, works on a dirty tree) and `/rollback <step>` (restore a Polaris checkpoint). It never rewrites the user's HEAD, index or branches unless a user-approved commit/merge command says so. Inside sandboxes git runs with `core.autocrlf=false` so bytes survive the sync.

**API sketch.**
```rust
pub struct BranchName(/* validated ref name, never starts with '-' */ String);
pub struct RevSpec(String);
pub struct CheckpointSeq(u64);

pub struct Repo { /* gix repository handle + CAS handle */ }
impl Repo {
    pub fn open(root: &Path) -> Result<Repo, VcsError>;
    pub fn status(&self) -> Result<StatusReport, VcsError>;
    pub fn diff(&self, spec: DiffSpec) -> Result<DiffSet, VcsError>;            // text + binary-aware, rename detection
    pub fn unified(&self, d: &DiffSet, ctx: u8) -> Result<String, VcsError>;
    pub async fn snapshot_before_edit(&self, paths: &[RepoPath]) -> Result<Checkpoint, VcsError>;   // into CAS
    pub async fn undo(&self) -> Result<UndoReport, VcsError>;
    pub async fn rollback(&self, to: CheckpointSeq) -> Result<UndoReport, VcsError>;
    pub async fn worktree_add(&self, name: &BranchName, base: &RevSpec) -> Result<Worktree<Clean>, VcsError>;
    pub async fn commit(&self, msg: &CommitMessage, approval: ApprovalToken) -> Result<CommitId, VcsError>;
}
pub struct Clean; pub struct Dirty; pub struct Removed;
pub struct Worktree<S> { /* path under .polaris/worktrees/<id>, branch */ _s: core::marker::PhantomData<S> }
impl Worktree<Clean> { pub fn mark_dirty(self, change: &ChangeSet) -> Worktree<Dirty>; }
impl Worktree<Dirty> { pub async fn remove(self, force: ForceDirty) -> Result<Worktree<Removed>, VcsError>; }
pub trait GitRunner { async fn run(&self, args: &[GitArg]) -> Result<GitOutput, VcsError>; }   // CLI fallback seam; impl'd over sdk PlatformOps
pub enum VcsError { NotARepo, DirtyTree, InvalidRef(String), Conflict(Vec<RepoPath>), CasMissing(Cid), GitCli { code: i32 }, Gix(String), Io(std::io::Error) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-cas`. External: `gix = 0.89.0` (`default-features = false`, features `max-performance-safe`, `status`, `revision`, `worktree-mutation`, `sha1`), `imara-diff = 0.2.0`, `similar = 3.2.0`; `+proposed`: `tokio = 1.54.0` (git CLI via sdk `PlatformOps`, no direct dependency on `polaris-platform-process`), `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `BranchName::parse`, `Repo::status`, `Repo::diff`, `snapshot_before_edit` + `undo`, `rollback`, `worktree_add`/`remove`, `GitRunner` argument builder.
- [ ] Implement `BranchName`/`RevSpec` validation (git ref rules, leading `-`, `..`, control chars, Windows-reserved names) so no user/model string reaches the CLI unvalidated; always pass `--` before paths.
- [ ] Implement `status` and `diff` with gix (status, rename detection); choose `imara-diff` vs `similar` per hot path by benchmark and record the decision.
- [ ] Implement pre-edit snapshots into CAS keyed by blake3 and an undo stack persisted via the event log (seq numbers owned by the kernel, passed in).
- [ ] Implement `/undo` (single action) and `/rollback <step>` (restore listed files byte-exactly, with a dry-run plan first); never restore over user edits made after the checkpoint without a conflict report.
- [ ] Implement worktrees under `.polaris/worktrees/<id>`: try gix (`worktree-mutation`); if linked-worktree creation is not supported at this version, fall back to the git CLI behind `GitRunner`. Verify what gix 0.89 supports before choosing.
- [ ] Implement the git CLI fallback runner with a fixed config prelude (`-c core.autocrlf=false -c core.longpaths=true -c core.protectNTFS=true`), scrubbed environment, timeouts and output caps.
- [ ] Handle Windows specifics: reserved names (CON, NUL, ...), case-only collisions, long paths (`core.longpaths`), `safe.directory` / "dubious ownership" for repos reached through WSL mounts.
- [ ] Optional hidden-ref checkpoints (`refs/polaris/<session>/<seq>`) written with a temporary index so the user's index and HEAD are never touched; keep only if the plumbing is feasible in gix, else CLI. UNVERIFIED design.
- [ ] Wire `/diff` to a `DiffSet` that the sdk verifier can consume (same type used for churn/AST metrics input).

**Tests.**

| Category | Cases |
|---|---|
| unit | ref validation table; diff of add/modify/delete/rename/binary/mode-change; CRLF files stay byte-identical across snapshot/restore |
| proptest | `restore(snapshot(x)) == x` for arbitrary bytes and path sets; `BranchName::parse` never accepts a string beginning with `-` or containing `..` |
| fuzz | unified-diff rendering/parsing of generated trees; ref-name parser |
| smoke | init temp repo, edit, `/diff`, `/undo`, `/rollback` round trip; worktree add/remove |
| scalability | status/diff on 1k / 10k / 100k-file repos (targets to measure; report gix vs CLI) |
| stress | 32 concurrent frames each creating a worktree; crash mid-rollback and recover from the journal; disk full during snapshot |
| security | malicious branch names, paths with `..`, symlink escaping the worktree, hostile `.gitattributes`/hooks not executed by the fallback runner (hooks disabled via `core.hooksPath`) |
| fixtures | hostile-path trees and malicious-diff corpus generated at test time (never committed; AV and secret scanners) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Ref Name Validation | function | `BranchName::parse` | `crates/polaris-vcs/src/refname.rs` | `accepts_plain_feature_branch`; `rejects_leading_dash`; `rejects_dotdot_and_control_chars`; `rejects_windows_reserved_names`; `never_panics_on_arbitrary_input` |
| Pre-edit Snapshot and Undo | method | `Repo::snapshot_before_edit` / `Repo::undo` | `.../polaris-vcs/src/undo.rs` | `undo_restores_bytes_exactly`; `undo_preserves_crlf`; `undo_on_dirty_tree_keeps_unrelated_edits`; `second_undo_walks_the_stack`; `missing_cas_blob_is_typed_error` |
| Checkpoint Rollback | method | `Repo::rollback` | `.../polaris-vcs/src/rollback.rs` | `dry_run_lists_files_without_writing`; `rollback_detects_post_checkpoint_user_edit`; `rollback_to_unknown_seq_errors`; `rollback_is_idempotent` |
| Worktree Lifecycle | method | `Repo::worktree_add` | `.../polaris-vcs/src/worktree.rs` | `add_creates_isolated_checkout`; `add_existing_branch_errors`; `remove_dirty_requires_force`; `cleanup_after_crash_finds_orphans` |

**Risks & open questions.**
- UNVERIFIED: gix 0.89 support for linked-worktree creation and tree-to-working-directory writes under the pinned feature set; the CLI fallback exists for this reason. Cover with a P2 mini-spike (no S0 id; record in PHASES).
- LIKELY: gix status/diff on large repos is fast enough; measure against `git status` on this machine rather than assume.
- VERIFIED (plan): git runs with `autocrlf=false` in sandboxes; the Windows workspace stays authoritative and is mirrored byte-exact.
- Security: attacker-controlled repos can ship hooks/filters/attributes; the fallback runner must neutralize them and never run user hooks implicitly.
- S0-7 (sync fidelity) covers EOL/case behavior at the sync boundary; this crate must not "fix" line endings.

**Chatlog sources.** msg 1 §4.1 ("copy-on-write source overlay") and §5.2 (`/diff`, `/rollback`, `/quit` commit) - adopted; the CoW overlay is replaced by worktrees plus CAS snapshots (no overlay filesystem on a native Windows host). msg 1 §9 (unified diff, ask confirmation or auto-apply). msg 13 (`RollbackBranch`, `DiffApproved{commit_id}`). msg 19 (`/session replay [step]`, registry stores `git_commit_base`, branch). msg 37 (swipe-left rolls back the branch; merge as a voice intent) - deferred to v2 UI, the `commit` API is the hook. msg 11 (cycle breaker: more than 3 edits to one file) - the per-file edit counter is read from snapshot history.



---

### `polaris-openshell` — facade over the OpenShell integration (client, policy, nemoclaw); home of the execution-tier and WSL2-topology docs

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-openshell` | L2 | facade | (top-level) | P2 |

**Purpose.** Pure re-export crate with no logic. It owns the crate-level documentation of the execution tiers E0-E4/E1w and of the WSL2 topology, because those cross three children and two other crates (`polaris-platform-wsl`, `polaris-nebius-cloud-*`).
**Re-exports.** `pub use polaris_openshell_client as client;` `pub use polaris_openshell_policy as policy;` `pub use polaris_openshell_nemoclaw as nemoclaw;` (`pub use polaris_openshell_*`). Children never depend on this facade.

**Execution tiers (facts from the PLAN core (§0–§12); status per tier is a plan, nothing here has been run).**

| Tier | What | Isolation | Reached through | Gate / spike |
|---|---|---|---|---|
| E0 | in-process read-only operations (read file, grep, repo map) | none needed (no writes, no exec) | direct, inside the kernel | none |
| E1 | OpenShell `docker` driver in WSL2 Ubuntu 24.04 | container + OpenShell policy (Landlock etc.) | `polaris-openshell-client` -> gateway | P0 setup, S0-1 |
| E2 | OpenShell `vm` driver (libkrun, experimental, needs `/dev/kvm`, image `nvcr.io/nvidia/base/ubuntu:24.04`; GPU would need VFIO and root, not a goal on the GTX 1650) | microVM | same client, driver = vm | S0-2 (nested KVM); fallback E4 |
| E3 | Token Factory Sandboxes (Contree) | cloud VM | `polaris-nebius-cloud-sandboxes` (rmcp client to `contree-mcp`), not an OpenShell child | S0-13, keyed |
| E4 | remote OpenShell gateway on a Nebius AI Cloud VM | per remote driver; mTLS endpoint | client + `polaris-nebius-cloud-ai-cloud` | P6, keyed (AI Cloud quota) |
| E1w | OpenShell `mxc` driver (Windows AppContainer via `wxc-exec`): no process policy, no interactive exec, no OCI images | AppContainer | client, driver = mxc | S0-11 (optional) |

Selection (`$sandbox auto|openshell|microvm|contree|remote`, alias `$nebius-sandbox`) is an enum-dispatch `SandboxBackend` built at the composition root; `dyn` is not used. UNVERIFIED: whether nested KVM works for E2 on this machine, and whether Nebius VMs offer nested virtualization for an E4 `vm` driver (S0-2).

**WSL2 topology (who connects to whom).**
```
Windows host (native)                      WSL2 VM (Ubuntu 24.04, ext4 vhdx)           Nebius AI Cloud (optional)
+----------------------------+  host-initiated  +-------------------------------+  mTLS  +----------------------+
| polaris core + TUI/GUI     |----------------->| OpenShell gateway (systemd)   |<-------| remote OpenShell     |
|  - openshell-client (gRPC) | 127.0.0.1:<port> |  drivers: docker | vm(/dev/kvm)| host-  | gateway VM (E4)      |
|  - platform-wsl (wsl.exe)  | localhost fwd    |  sandboxes -> polaris-guest   | initiated (ai-cloud lifecycle) |
|  - storage-sync (blake3)   |---- sync ------->|  sandbox volume (byte-exact)  |        +----------------------+
|  - keepalive child process |-- wsl.exe exec ->|  keepalive: sleep infinity    |
+----------------------------+                  +-------------------------------+
```
- Only the Windows host opens connections: core -> gateway (gRPC over the WSL2 localhost forward), core -> `wsl.exe` (management), core -> remote gateway (mTLS). The gateway, sandboxes and `polaris-guest` never dial back into Windows; the guest `protocol_version` handshake is initiated by the host after it starts the guest (transport inside the sandbox - exec stream vs forwarded port - is UNVERIFIED, S0-12).
- Workspace stays authoritative on Windows; a blake3-incremental, byte-exact sync mirrors it into the sandbox volume on the Linux filesystem (not `/mnt/c`), governed by `.polarisignore` (`.env`, `*.pem`, `*.key`, `.git/config` denied by default).
- Keepalive: `polaris-platform-wsl` holds a supervised `wsl.exe -d Ubuntu-24.04 --exec sleep infinity` child and/or a systemd unit for the gateway, with a periodic gateway health probe. Reason: the WSL2 VM can shut down when idle (exact behavior UNVERIFIED on this build, S0-1).
- Sandbox egress is deny-by-default through OpenShell policy; secrets reach sandboxes only as provider-profile substitution per endpoint and binary.

**TODO.**
- [ ] Re-export the three children; compile-only doctest naming one item from each.
- [ ] Put the tier table and topology above into `//!` docs and mark each row with the spike result and PASS evidence once recorded in `PHASES_STATUS.md`.
- [ ] Document the fallback ladder: E2 fails -> E1 or E4; WSL2 unavailable on a judge's machine -> E3 or E4 (MVP requirement).
- [ ] Document the pinned OpenShell rev (`5a7728c304b5528e954e9ae900cf21f84e1a12b1`, tag v0.1.4) and the rule that upgrades go through a spike.


---

### `polaris-openshell-client` — adapter over `openshell-sdk` (pinned rev of v0.1.4): sandbox create/exec/stream/destroy, provider attach, driver selection

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-openshell/crates/client` | L2 | lib | `polaris-openshell` | P2 |

**Purpose.** Implements the sdk `SandboxBackend` trait over a local (WSL2) or remote (Nebius VM) OpenShell gateway. It is the single implementation point for OpenShell: no other crate imports `openshell-sdk`. It creates sandboxes with a chosen driver and policy, runs commands with streamed output, moves bytes for the sync protocol, attaches the Polaris provider profile, and destroys sandboxes through the Reaper. OpenShell is alpha (v0.1.4), so the client checks the gateway version and fails closed on mismatch.

**API sketch.**
```rust
pub struct GatewayEndpoint { /* sdk type: url, tls (loopback or mTLS material), expected_version */ }
pub enum DriverKind { Docker, Vm, Mxc /* Podman and Kubernetes exist upstream; not selectable in Polaris */ }
pub struct SandboxSpec { pub driver: DriverKind, pub image: ImageRef, pub policy: PolicyHandle, pub providers: Vec<ProviderRef>,
                         pub quota: ResourceQuota, pub workdir: LinuxPath }
pub struct Created; pub struct Running; pub struct Destroyed;                    // typestate markers
pub struct Sandbox<S> { /* id, gateway handle */ _s: core::marker::PhantomData<S> }
pub struct OpenShellBackend { /* gateway channel, version info */ }
impl OpenShellBackend {
    pub async fn connect(ep: &GatewayEndpoint) -> Result<OpenShellBackend, OpenShellError>;   // version + driver capability check
    pub async fn create(&self, spec: SandboxSpec) -> Result<Sandbox<Created>, OpenShellError>;
    pub async fn health(&self) -> Result<GatewayHealth, OpenShellError>;
}
impl Sandbox<Created> { pub async fn start(self) -> Result<Sandbox<Running>, OpenShellError>; }
impl Sandbox<Running> {
    pub async fn exec(&self, cmd: &ExecRequest) -> Result<ExecStream, OpenShellError>;    // Stream<Item = Result<ExecEvent, _>>
    pub async fn put_blob(&self, path: &LinuxPath, bytes: bytes::Bytes) -> Result<(), OpenShellError>;
    pub async fn get_blob(&self, path: &LinuxPath) -> Result<bytes::Bytes, OpenShellError>;
    pub async fn update_network_policy(&self, p: &NetworkPolicies) -> Result<(), OpenShellError>;   // the only hot-reloadable section
    pub async fn destroy(self) -> Result<Sandbox<Destroyed>, OpenShellError>;
}
/* impl polaris_sdk::SandboxBackend for OpenShellBackend */
pub enum OpenShellError { VersionMismatch { expected: String, found: String }, DriverUnavailable(DriverKind), Transport(String),
                          PolicyRejected(String), ExecFailed { code: i32 }, Timeout, SandboxGone, Io(std::io::Error) }
```
All signatures are Polaris-side; the shape of the `openshell-sdk` types is API to confirm at implementation (S0-1).

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-log`. External: `openshell-sdk` (git `https://github.com/NVIDIA/OpenShell`, `rev = "5a7728c304b5528e954e9ae900cf21f84e1a12b1"`, tag v0.1.4), `tokio = 1.54.0`, `tonic = 0.14.6`; `+proposed`: `thiserror = 2.0.21`, `bytes = 1.12.1`, `futures = 0.3.34`. Note: openshell-sdk brings tonic 0.14, reqwest 0.12.28 and rustls (aws-lc-rs); our own code uses reqwest 0.13.5 (accepted duplicate, reviewed in `cargo tree -d`).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `OpenShellBackend::connect` (version gate), `Sandbox` lifecycle typestate, `exec` stream mapping, `put_blob`/`get_blob`, driver capability matrix, provider attach.
- [ ] Spike S0-1: add `openshell-sdk` at the pinned rev and run `cargo check` on windows-msvc (NASM and cmake are present; aws-lc-rs needs them); record build time and RAM. Kill criterion: does not build, or exceeds the P0 build budget.
- [ ] Spike S0-1 fallback prepared in parallel: list the gateway `.proto` files in the OpenShell repo at the pinned rev and generate a minimal tonic client for create/exec/destroy; decide `polaris-proto` vs a local build script (consistency flag: `workspace.toml` deps for this crate do not list `polaris-proto`).
- [ ] Keep every `openshell-sdk` type inside one private module so the proto-generated fallback is a drop-in.
- [ ] Implement the version gate and a `DriverCapabilities` table: docker (full), vm (experimental, requires `/dev/kvm`), mxc (no process policy, no interactive exec, no OCI images; reject unsupported requests early and by type where possible).
- [ ] Implement `exec` with streamed stdout/stderr, exit code, wall-clock and output caps, cancellation that stops the remote process, and reconnect with resume-or-fail semantics.
- [ ] Implement `put_blob`/`get_blob` as the transport used by `polaris-storage-sync` (chunking, blake3 verification on both ends).
- [ ] Attach the Token Factory provider profile only when a frame needs inference from inside the sandbox (default off); verify no secret value reaches the host log or event stream.
- [ ] Register every created sandbox with the Reaper (sdk teardown callback); implement orphan discovery by label on startup.
- [ ] Remote path: accept a `GatewayEndpoint` with mTLS material from `polaris-nebius-cloud-ai-cloud`; verify server identity; refuse plaintext for non-loopback. BLOCKED-ON-ACCESS (Nebius AI Cloud account/quota) for the live check.
- [ ] Live local check against the WSL2 gateway. BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 and OpenShell v0.1.4 not installed yet).

**Tests.**

| Category | Cases |
|---|---|
| unit | capability matrix per driver; typestate (`Destroyed` has no `exec`, checked with a negative compile test); error mapping |
| contract | synthetic in-process tonic server for the subset (labelled synthetic) vs live gateway in WSL2 (SKIPPED until the distro exists); both run the same suite |
| smoke | create -> exec `echo hi` -> destroy on docker driver (live, after P0); vm driver if S0-2 passes |
| load | 100 sequential create/exec/destroy cycles; 10 MB output stream; targets to measure (create latency, exec overhead) |
| scalability | 1 -> 32 concurrent sandboxes (frames) up to the WSL VM memory cap; record the knee |
| stress | gateway restart mid-exec; WSL VM shutdown mid-stream; sandbox OOM; network partition on the remote path; sandbox volume full |
| spike | S0-1 (build + reachability), S0-2 (vm driver), S0-11 (mxc), S0-12 (guest handshake transport) |
| security | no env/secret leakage into logs or events; sandbox cannot reach Windows host ports (host-initiated only); policy rejection is surfaced, never silently widened |
| fuzz | exec event decoder / framing if the proto fallback is used |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Gateway Version Gate | async_function | `OpenShellBackend::connect` | `crates/polaris-openshell/crates/client/src/connect.rs` | `matching_version_connects`; `mismatched_version_fails_closed`; `unreachable_gateway_is_transport_error`; `plaintext_remote_is_refused` |
| Sandbox Lifecycle | struct | `Sandbox<S>` | `.../client/src/sandbox.rs` | `create_start_exec_destroy_order`; `destroy_is_idempotent_via_reaper`; `exec_after_destroy_does_not_compile`; `orphan_scan_finds_labelled_sandboxes` |
| Exec Streaming | async_function | `Sandbox::exec` | `.../client/src/exec.rs` | `streams_stdout_and_stderr_in_order`; `reports_exit_code`; `cancel_stops_remote_process`; `output_cap_truncates_with_marker` |
| Driver Capability Matrix | function | `DriverKind::capabilities` | `.../client/src/driver.rs` | `docker_supports_policy_and_exec`; `mxc_rejects_process_policy`; `vm_requires_kvm_flag`; `unknown_request_is_typed_error` |

**Risks & open questions.**
- UNVERIFIED: `openshell-sdk` builds on windows-msvc and its API (create/exec/stream/file transfer) matches the sketch (S0-1; fallback: proto-generated client).
- UNVERIFIED: reachability of the WSL2 gateway from Windows (localhost forwarding vs mirrored networking) and gateway auth/TLS defaults on loopback (S0-1).
- VERIFIED: OpenShell v0.1.4 is alpha; Windows support is WSL2-only and experimental; `inference.local` and `openshell inference set` were removed (provider profiles replace them).
- UNVERIFIED: per-sandbox CPU/memory quotas for child frames (workspace.toml responsibility). If drivers do not expose them, quotas fall back to the WSL VM cap and frame-count limits.
- UNVERIFIED: how to reach `polaris-guest` inside the sandbox from the host (S0-12).
- Three reqwest majors coexist (0.12 here via openshell-sdk, 0.13 ours, 0.11 dagger-sdk behind a feature); keep them out of public types.

**Chatlog sources.** msg 1 §4 and §8.3 (`OpenShellSandbox::create/execute_tool`, `SandboxPolicy` with allowed hosts, read-only binds, writable overlay, memory/CPU) - adopted as `SandboxSpec`; "OpenShell C-FFI / CLI RPC bindings" corrected to the Rust `openshell-sdk` over gRPC (git pin) with a proto fallback. msg 1 §4.1 (child sub-agents inherit hard limits, no network) - quota + policy in `SandboxSpec`. msg 9 §5 (local OpenShell vs cloud Contree hybrid) - tiers E1/E2/E3. msg 25 (Firecracker tier, guest daemon, vsock) - corrected: Firecracker replaced by the OpenShell `vm` driver (D2); vsock is not assumed. msg 27 §5 (VM teardown SIGTERM -> 200 ms -> kill) - mapped to `destroy` via the Reaper. msg 35 (gRPC to the guest) - guest transport is S0-12.


---

### `polaris-openshell-policy` — generate and lint OpenShell policy YAML and provider profiles

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-openshell/crates/policy` | L2 | lib | `polaris-openshell` | P2 |

**Purpose.** Pure, deterministic library (no I/O) that derives least-privilege OpenShell policies per frame role and execution tier, lints them against the policy schema limits before they reach a gateway, and emits provider profiles (notably the custom Nebius Token Factory profile). Being pure, it is the best candidate for proptest, fuzz, Kani and Miri. VERIFIED (OpenShell v0.1.4 docs): policy YAML top-level fields are `version`, `filesystem_policy`, `landlock`, `process`, `network_policies`; only the network section hot-reloads; inference routes were removed in favour of provider profiles.

**API sketch.**
```rust
pub struct Policy { pub version: PolicyVersion, pub filesystem_policy: FilesystemPolicy, pub landlock: LandlockPolicy,
                    pub process: Option<ProcessPolicy> /* None for mxc */, pub network_policies: NetworkPolicies }
pub struct NetworkPolicies(/* named rule groups, each with endpoints[] and binaries[] */ Vec<NetworkRule>);
pub struct Endpoint { pub host: HostPattern, pub port: Port, pub protocol: Protocol, pub access: Access, pub enforcement: Enforcement }
pub struct ProviderProfile { pub id: ProfileId, pub display_name: String, pub description: String, pub category: String,
                             pub inference_capable: bool, pub credentials: Vec<CredentialSlot>,
                             pub endpoints: Vec<Endpoint>, pub binaries: Vec<BinaryPath> }
pub fn policy_for(role: &AgentRole, tier: ExecTier, quota: &ResourceQuota) -> Result<Policy, PolicyError>;
pub fn provider_profile_token_factory(cfg: &TokenFactoryProfileCfg) -> Result<ProviderProfile, PolicyError>;
pub fn lint(p: &Policy, tier: ExecTier) -> Result<LintReport, PolicyError>;      // limits, bare-root rw, wildcard hosts, mxc restrictions
pub fn to_yaml(p: &Policy) -> Result<String, PolicyError>;                        // canonical ordering, stable bytes
pub fn from_yaml(s: &str) -> Result<Policy, PolicyError>;                         // size and depth limited
pub enum PolicyError { SchemaLimit { what: &'static str, max: usize }, BareRootWrite, PathNotAbsolute(String), HostWildcardDenied(String),
                       UnsupportedForTier { tier: ExecTier, field: &'static str }, Yaml(String), TooLarge }
```

**Example Polaris policy skeleton.** Only the five top-level keys are VERIFIED. Every nested field name and value below is ILLUSTRATIVE and must be confirmed against `openshell-policy-schema` at v0.1.4 before implementation.
```yaml
#### to confirm against openshell-policy-schema at v0.1.4 (nested names and values are illustrative)
version: 1
filesystem_policy:
  read_only:
    - /usr
    - /lib
    - /etc/ssl/certs
  read_write:
    - /workspace              # the synced sandbox volume; never "/" (bare-root rw rejected by lint)
    - /tmp
  # limit: <= 256 paths in total (workspace.toml responsibility; confirm exact limit and semantics)
landlock:
  compatibility: best_effort  # illustrative; confirm allowed values
process:                      # absent for the mxc driver (no process policy)
  run_as_user: sandbox
  no_new_privileges: true
network_policies:             # the only hot-reloadable section
  crates_io:
    endpoints:
      - { host: "index.crates.io",  port: 443, protocol: https, access: read_only, enforcement: enforce }
      - { host: "static.crates.io", port: 443, protocol: https, access: read_only, enforcement: enforce }
    binaries: [ /usr/local/cargo/bin/cargo ]
  # no rule for api.tokenfactory.nebius.com unless the frame attaches the Token Factory provider profile
```

**Custom Token Factory provider profile skeleton.** Field list per the verified OpenShell docs (`id`, `display_name`, `description`, `category`, `inference_capable`, `credentials`, `endpoints[host,port,protocol,access,enforcement]`, `binaries`); field-level details, value vocabularies and the `credentials` shape are to confirm against `openshell-policy-schema` at v0.1.4.
```yaml
#### to confirm against openshell-policy-schema at v0.1.4
id: nebius-token-factory
display_name: Nebius Token Factory
description: OpenAI-compatible inference at https://api.tokenfactory.nebius.com/v1/ (key substituted at the egress proxy; never visible to sandboxed code)
category: inference            # illustrative value
inference_capable: true
credentials:
  - name: NEBIUS_API_KEY       # reference only; the value is given to the gateway out of band, never written by Polaris into YAML
    # substitution header/format (Authorization: Bearer ...) to confirm
endpoints:
  - { host: "api.tokenfactory.nebius.com", port: 443, protocol: https, access: full, enforcement: enforce }
binaries:
  - /usr/local/bin/polaris-guest   # illustrative; list only binaries that really call inference
```
Design note: S2 inference calls originate from the Polaris core on Windows, not from sandboxes, so this profile is attached only for frames that run code needing inference (default off, least privilege).

**Dependencies.** Internal: `polaris-sdk`. External: `serde = 1.0.229` (feature `derive`), `serde-saphyr = 2.0.0`; `+proposed`: `thiserror = 2.0.21`, `blake3 = 1.8.7` (policy digest; or let the nemoclaw crate, which already lists blake3, compute it over `to_yaml` output).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `lint` (limits and bare-root rw), `policy_for` (role x tier matrix), `provider_profile_token_factory`, `to_yaml` canonicalization, `from_yaml` size/depth limits.
- [ ] Fetch `openshell-policy-schema` at the pinned rev (and the docs `.md` pages) and replace every ILLUSTRATIVE field above with the real schema; keep a snapshot in `tests/fixtures` with provenance and licence.
- [ ] Confirm that `serde-saphyr` 2.0.0 can serialize (not only deserialize); if not, decide between a hand-written canonical emitter and another YAML crate (needs an ADR; do not add a crate silently).
- [ ] Implement typed newtypes (`Port`, `HostPattern`, `AbsPath`) with validation and the schema limits (path count, no bare-root rw, absolute paths only, no `..`).
- [ ] Implement `policy_for` for roles (reader, builder, tester, browser, adversary) x tiers (E1, E2, E4, E1w); deny network by default; E1w yields `process = None` plus a lint note.
- [ ] Implement per-frame network allow-list deltas as `NetworkPolicies` patches matching the "only network hot-reloads" rule; any other change requires a new sandbox.
- [ ] Implement `provider_profile_token_factory` with the host taken from config (default `api.tokenfactory.nebius.com`); never embed secret values.
- [ ] Implement canonical YAML output with stable key order (digest is computed over it by the blueprint).
- [ ] Add the fuzz target for policy YAML (WSL2 nightly) and the proptest property "lint(policy_for(x)) is always clean".
- [ ] Validate generated policies against a real gateway. BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 + OpenShell v0.1.4 not installed yet).

**Tests.**

| Category | Cases |
|---|---|
| unit | each lint rule positive and negative; role x tier matrix snapshot (insta) |
| proptest | any `policy_for` result passes `lint`; `from_yaml(to_yaml(p)) == p`; the path-count limit holds for arbitrary inputs; no emitted policy has a rw path equal to `/` |
| fuzz | `from_yaml` on arbitrary bytes (depth and size bombs, anchors/aliases, huge scalars) |
| kani/miri | Kani on limit arithmetic (path counting, no overflow); Miri over the pure crate |
| contract | schema-conformance against the vendored schema snapshot (synthetic until the schema is pulled); live gateway acceptance later |
| security | wildcard-host and CIDR-widening attempts rejected; Token Factory profile never contains a credential value; adversary role has no network |
| fixtures | recorded upstream sample policies with provenance |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Policy Lint | function | `lint` | `crates/polaris-openshell/crates/policy/src/lint.rs` | `rejects_bare_root_read_write`; `rejects_more_than_max_fs_paths`; `rejects_relative_and_dotdot_paths`; `mxc_with_process_policy_is_unsupported`; `wildcard_host_denied_by_default` |
| Policy Derivation | function | `policy_for` | `.../policy/src/derive.rs` | `every_role_tier_pair_lints_clean`; `adversary_role_has_no_network`; `builder_allows_only_listed_registries`; `e1w_has_no_process_section` |
| Token Factory Profile | function | `provider_profile_token_factory` | `.../policy/src/profile.rs` | `profile_has_required_fields`; `endpoint_is_443_https_only`; `no_credential_value_in_output`; `custom_base_host_is_validated` |
| Canonical YAML | function | `to_yaml` / `from_yaml` | `.../policy/src/yaml.rs` | `output_is_byte_stable`; `roundtrip_preserves_policy`; `rejects_oversized_document`; `rejects_alias_bombs` |

**Risks & open questions.**
- UNVERIFIED: every nested policy field name, value vocabulary and limit above except the five top-level keys; the `<= 256` fs path rule comes from the workspace table and must be re-read from the schema.
- UNVERIFIED: whether profile credentials are referenced by name or loaded through a gateway command (affects who stores the Nebius key and how it is passed; ties to the secrets vault).
- UNVERIFIED: `serde-saphyr` serialization support (see TODO).
- VERIFIED: only the network section hot-reloads; other policy changes require a new sandbox.
- A lint that is too permissive gives false assurance; the lint is defence in depth and the gateway remains the enforcement point.

**Chatlog sources.** msg 1 §4.1 and §8.3 (`SandboxPolicy`: egress allow-list, read-only binds, memory/CPU; "Credential Custody" with tokens injected only at verified egress for `api.tokenfactory.nebius.ai`) - adopted; domain corrected to `.com`; `inference set` / `inference.local` were removed upstream and replaced by provider profiles. msg 1 §4.1 (child sub-agents inherit hard limits, no network) - role-based policy derivation. msg 11 §2 (command injection, path traversal, symlink classes) - lint rules and security tests. msg 19 (Biscuit attenuation bounds child scope) - policy tightening mirrors capability attenuation at the composition root.

---

### `polaris-openshell-nemoclaw` — Polaris blueprint (versioned YAML) plus onboarding planner and doctor for WSL2

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-openshell/crates/nemoclaw` | L2 | lib | `polaris-openshell` | P2 |

**Purpose.** NemoClaw (VERIFIED) is a reference stack - a versioned YAML blueprint (sandbox shape, policies, inference profiles), onboarding, lifecycle and the `ghcr.io/nvidia/nemoclaw/sandbox-base` image - not a sandbox runtime; it ships OpenClaw, Hermes and DeepAgents, and bring-your-own-agent is undocumented (UNVERIFIED, S0-3). This crate defines the Polaris blueprint, verifies its digest, produces a pure onboarding plan (install OpenShell in WSL2, build the Polaris sandbox image containing `polaris-guest`, start the gateway) and evaluates doctor probes. It does not execute host commands itself: the composition root runs the plan through `polaris-platform-wsl` and feeds probe results back, which keeps this crate pure and layer-clean.

**API sketch.**
```rust
pub struct BlueprintVersion(u32);
pub struct Blueprint { pub version: BlueprintVersion, pub sandbox: SandboxShape, pub policies: Vec<PolicyRef>,
                       pub provider_profiles: Vec<ProfileRef>, pub guest_protocol_version: u32,
                       pub pinned: PinnedVersions /* openshell tag+rev, image digests */ }
pub struct SandboxShape { pub image: ImageRef, pub driver: DriverHint, pub workdir: LinuxPath, pub resources: ResourceQuota }
pub fn blueprint_default(cfg: &BlueprintCfg) -> Result<Blueprint, NemoclawError>;
pub fn parse_blueprint(yaml: &str) -> Result<Blueprint, NemoclawError>;
pub fn blueprint_digest(b: &Blueprint) -> BlueprintDigest;                         // blake3 over canonical YAML
pub fn verify(b: &Blueprint, expected: &BlueprintDigest) -> Result<(), NemoclawError>;
pub fn plan_onboard(b: &Blueprint, host: &HostFacts) -> Result<OnboardPlan, NemoclawError>;   // ordered typed steps: check, timeout, undo note
pub fn evaluate_doctor(b: &Blueprint, probes: &ProbeResults) -> DoctorReport;      // per item PASS / FAIL / UNKNOWN
pub async fn apply<B: polaris_sdk::SandboxBackend>(backend: &B, b: &Blueprint) -> Result<AppliedBlueprint, NemoclawError>;
pub fn dockerfile_for(b: &Blueprint) -> Result<String, NemoclawError>;            // sandbox image recipe incl. polaris-guest
pub struct Staged<S> { /* blueprint */ _s: core::marker::PhantomData<S> }          // S in {Pending, Verified, Applied}
pub enum NemoclawError { Yaml(String), DigestMismatch, UnsupportedVersion(u32), MissingPrerequisite(String), Policy(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-openshell-policy`. External: `serde = 1.0.229`, `serde-saphyr = 2.0.0`, `blake3 = 1.8.7`; `+proposed`: `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `blueprint_default`, `parse_blueprint`, `blueprint_digest`/`verify`, `plan_onboard`, `evaluate_doctor`, `dockerfile_for`, `apply`.
- [ ] Spike S0-3 (read-only first): read the NemoClaw blueprint schema and CLI docs (`.md` pages, docs MCP server at `docs.nvidia.com/nemoclaw/_mcp/server`) and record whether a custom agent blueprint is supported. The native applier stays the default path either way.
- [ ] Define the blueprint schema (versioned, forward-compatible `version`), independent of upstream NemoClaw's schema unless S0-3 shows compatibility is worthwhile.
- [ ] Implement `plan_onboard` as ordered typed steps: prerequisites (WSL2, Ubuntu 24.04, `.wslconfig`, KVM if E2), install pinned OpenShell v0.1.4, build the Polaris sandbox image (needs the Rust toolchain inside WSL2 to build `polaris-guest`), start the gateway/systemd unit, smoke sandbox, record digests. Each step has a check, a timeout and an undo note.
- [ ] Implement `dockerfile_for` with a pinned base (`nvcr.io/nvidia/base/ubuntu:24.04` for the vm driver; sandbox-base as an option for the docker driver) and no secrets in layers.
- [ ] Implement `evaluate_doctor` over probe results (OpenShell version, driver availability, gateway health, KVM, image present and digest, guest handshake version) and expose it to `polaris doctor`.
- [ ] Implement `apply` over the sdk `SandboxBackend` (generic), verifying the blueprint digest before any use.
- [ ] Wire the guest handshake: `guest_protocol_version` is injected by the composition root from `polaris-proto`; this crate only compares numbers.
- [ ] Execute the plan on the real machine. BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 not installed; admin install pending).
- [ ] NemoClaw CLI interoperability check (S0-3). BLOCKED-ON-ACCESS (NemoClaw install inside WSL2 after Ubuntu exists).

**Tests.**

| Category | Cases |
|---|---|
| unit | blueprint parse/validate; digest stability; doctor item matrix (every probe state -> report line) |
| proptest | `parse(canonical(b)) == b`; `plan_onboard` output is deterministic and every step has a check; the digest changes on any field change |
| fuzz | `parse_blueprint` on arbitrary YAML |
| kani/miri | Miri over the pure planner; Kani on version-compat comparison |
| smoke | onboarding plan dry-run printed on this machine (shows Ubuntu missing); real run after P0 (blocked) |
| contract | blueprint accepted by a live gateway via `apply` (after P0); NemoClaw CLI schema compatibility (S0-3) |
| security | blueprint with unpinned image or `latest` tag rejected; Dockerfile contains no secret material; a tampered blueprint fails `verify` |
| fixtures | golden blueprint and golden Dockerfile (insta) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Blueprint Digest | function | `blueprint_digest` / `verify` | `crates/polaris-openshell/crates/nemoclaw/src/digest.rs` | `digest_is_stable_across_runs`; `field_change_changes_digest`; `verify_rejects_mismatch`; `key_order_does_not_affect_digest` |
| Onboarding Plan | function | `plan_onboard` | `.../nemoclaw/src/onboard.rs` | `plan_orders_prereqs_before_install`; `kvm_step_only_for_vm_driver`; `every_step_has_check_and_timeout`; `missing_ubuntu_adds_provision_step` |
| Doctor Report | function | `evaluate_doctor` | `.../nemoclaw/src/doctor.rs` | `all_green_reports_pass`; `unknown_probe_is_unknown_not_pass`; `version_mismatch_is_fail`; `report_lists_remediation` |
| Sandbox Image Recipe | function | `dockerfile_for` | `.../nemoclaw/src/image.rs` | `uses_pinned_base_image`; `includes_guest_binary_copy`; `contains_no_secrets`; `rejects_latest_tag` |

**Risks & open questions.**
- UNVERIFIED: NemoClaw supports a custom agent blueprint (S0-3). Fallback: the native applier (the default).
- UNVERIFIED: how OpenShell v0.1.4 consumes a "blueprint"; it may only accept policies, providers and images, in which case `apply` decomposes the blueprint into those calls.
- LIKELY: building `polaris-guest` inside WSL2 requires Rust and a Linux linker in Ubuntu; S0-12 verifies the build and the handshake.
- VERIFIED: the sandbox-base image is `ghcr.io/nvidia/nemoclaw/sandbox-base`; its compatibility with the OpenShell docker driver is not verified.
- Privilege: onboarding touches `/etc` and systemd in the distro; every step runs with the least privilege that works and is shown to the user before execution.

**Chatlog sources.** msg 1 §1, §4 and §7 (`polaris-nemoclaw` as "policy and guard layer", credential custody, formal-verification ceilings for children) - corrected: NemoClaw is a blueprint/onboarding stack, not a policy engine or token vault; enforcement lives in OpenShell policies and Biscuit capabilities. msg 1 §8.3 ("security profile of NemoClaw") - becomes the blueprint. msg 9 §5 (NemoClaw proxy protects keys) - provider profiles do. The standalone `polaris-nemoclaw` crate of the chatlog is merged into `polaris-openshell-nemoclaw`.


---

### `polaris-nebius-cloud` — facade over the Nebius integrations (token-factory, sandboxes, data-lab, ai-cloud)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nebius-cloud` | L2 | facade | (top-level) | P3 |

**Purpose.** Pure re-export crate with no logic. It documents how the Nebius children map to the hackathon requirements. The hackathon text (`@prime/hackaton_content.md`) requires that every submission "run on either Nebius Token Factory or Nebius AI Cloud and use at least one NVIDIA open source model", and the Coding and Agentic Engineering track asks for "agents that write, run, and test code in Token Factory".
**Re-exports.** `pub use polaris_nebius_cloud_token_factory as token_factory;` `pub use polaris_nebius_cloud_sandboxes as sandboxes;` `pub use polaris_nebius_cloud_data_lab as data_lab;` `pub use polaris_nebius_cloud_ai_cloud as ai_cloud;` (`pub use polaris_nebius_cloud_*`). Children never depend on this facade.

**Requirement mapping to keep in the crate docs.**

| Hackathon requirement / criterion | Child | Evidence needed in `PHASES_STATUS.md` |
|---|---|---|
| Runs on Token Factory, uses an NVIDIA open model (Nemotron) | `token-factory` | recorded keyed run using `nvidia/...` model ids (BLOCKED-ON-ACCESS until the key arrives) |
| "write, run, test code in Token Factory" | `sandboxes` (Contree) | keyed S0-13 run: edit, `run` tests in Token Factory Sandboxes, result back |
| Runs on Nebius AI Cloud (alternative) | `ai-cloud` (remote OpenShell gateway VM, E4) | keyed VM run (P6) |
| Potential impact / idea quality: improvement flywheel | `data-lab` | JSONL export of verified trajectories (keyless part) + upload (keyed, API unverified) |

**TODO.**
- [ ] Re-export the four children; compile-only doctest naming one item from each.
- [ ] Write the requirement-mapping table into the `//!` docs and keep the evidence column linked to PHASES task ids.
- [ ] Document the key/access inventory per child (Token Factory key, Contree auth, AI Cloud quota, Data Lab access) as the single place the demo-risk list reads from.
- [ ] Document the fallback story if only one Nebius surface is available at demo time.

---

### `polaris-nebius-cloud-token-factory` — OpenAI-compatible Token Factory client: SSE chat streaming, tool calls, embeddings, model list, usage accounting

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nebius-cloud/crates/token-factory` | L2 | lib | `polaris-nebius-cloud` | P3 |

**Purpose.** Implements the sdk `ChatModel` and `EmbeddingProvider` traits for `https://api.tokenfactory.nebius.com/v1/` (Bearer `NEBIUS_API_KEY`) and for any other OpenAI-compatible base URL, so the P3 walking skeleton can run first against a local Ollama/llama.cpp server on the GTX 1650 (a real endpoint, not a stub) and switch to Token Factory when the key arrives. It parses SSE frames on the hot path, surfaces content, reasoning and tool-call deltas, and records usage for telemetry and rate-limit reconciliation. VERIFIED (live `models_info`): the NVIDIA models are `nvidia/NVIDIA-Nemotron-3-Nano-30B-A3B` (262K), `nvidia/Nemotron-3_5-Lightning` (1M), `nvidia/nemotron-3-super-120b-a12b` (256K), `nvidia/Nemotron-3-Ultra-550b-a55b` (1M), `nvidia/Nemotron-3.5-Super-VL` (vision); embeddings `Qwen/Qwen3-Embedding-8B`; rate limits are soft caps in 15-minute windows.

**API sketch.**
```rust
pub struct BaseUrl(url::Url);                         // https required unless loopback
pub struct ApiKey(secrecy::SecretString);             // never Debug/Display; scrubbed from errors
pub struct TokenFactoryClient { /* reqwest::Client (HTTP/2, rustls), BaseUrl, ApiKey, RetryPolicy */ }
impl TokenFactoryClient {
    pub fn new(base: BaseUrl, key: ApiKey, cfg: ClientConfig) -> Result<Self, TfError>;
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, TfError>;
}
/* impl polaris_sdk::ChatModel for TokenFactoryClient */
pub trait SseSink { fn on_event(&mut self, ev: SseEvent<'_>) -> Result<(), TfError>; }     // borrows from the input buffer
pub struct SseParser { /* incremental, bounded frame size, handles CRLF/LF/CR, split UTF-8 */ }
impl SseParser { pub fn feed<S: SseSink>(&mut self, chunk: &[u8], sink: &mut S) -> Result<(), TfError>; }
pub struct ChatRequestBuilder<S> { /* messages, tools, model, effort, sampling */ _s: core::marker::PhantomData<S> }   // S in {NoMessages, Ready}
impl ChatRequestBuilder<Ready> { pub fn build(self) -> Result<ChatRequest, TfError>; }
pub enum ChatDelta<'a> { Content(&'a str), Reasoning(&'a str), ToolCall { index: u32, id: Option<&'a str>, name: Option<&'a str>, args_fragment: &'a str },
                         Finish(FinishReason), Usage(UsageReport) }
pub struct UsageReport { pub prompt: u32, pub completion: u32, pub cached: Option<u32> /* only if the API reports it: S0-5 */ }
pub enum TfError { Http { status: u16, retry_after: Option<std::time::Duration> }, RateLimited, Unauthorized, ContextOverflow,
                   StreamTruncated, SseFrameTooLarge, Decode(String), Cancelled, Tls(String), Io(std::io::Error) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-log`, `polaris-ratelimit`, `polaris-telemetry`. External: `reqwest = 0.13.5` (`default-features = false`, features `rustls`, `http2`, `json`, `stream`, `charset`), `eventsource-stream = 0.2.3`, `serde = 1.0.229`, `serde_json = 1.0.154`, `bytes = 1.12.1`, `tokio = 1.54.0`, `secrecy = 0.10.3`; `+proposed`: `thiserror = 2.0.21`, `url = 2.5.8`, `futures = 0.3.34`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `SseParser::feed`, `ChatRequestBuilder`, delta decoding (content/reasoning/tool-call/usage), retry/`Retry-After` policy, `list_models`, embeddings batching, `BaseUrl` validation.
- [ ] Decide the SSE implementation: the hot-path closed set requires zero per-frame allocation, which `eventsource-stream` (allocates per event) cannot provide. Plan: own borrowing `SseParser`; keep `eventsource-stream` as the differential-test oracle and as a cold-path fallback. Record as a one-line ADR.
- [ ] Implement `SseParser` (comments/keepalive lines, multi-line `data:`, `[DONE]`, CRLF/LF/CR, frames split at any byte including inside UTF-8, size cap) and prove invariance under arbitrary chunk splitting with proptest.
- [ ] Implement request builders for `/chat/completions` (messages, tools, tool_choice, stream, sampling, stop) with `Effort` -> `ModelId` mapping supplied by `polaris-models`; reasoning-control parameters are added only after S0-5 confirms them.
- [ ] Implement tool-call delta accumulation by `index`, partial-JSON arguments assembled outside the hot path, and a strict end-of-stream completeness check (`StreamTruncated` instead of returning half a tool call).
- [ ] Implement embeddings (`Qwen/Qwen3-Embedding-8B`), `list_models`, and base-URL selection (Token Factory vs local Ollama/llama.cpp) via config only.
- [ ] Implement retry with jittered backoff honoring `Retry-After`; coordinate with `polaris-ratelimit` (reserve before, reconcile after) rather than re-implementing limiting here.
- [ ] Walking-skeleton run against a local OpenAI-compatible server on the GTX 1650 (small tool-capable GGUF model, choice at implementation); this needs no API key.
- [ ] Contract tests with wiremock using synthetic SSE fixtures, clearly labelled synthetic; re-record from the live API when the key exists.
- [ ] Live S0-5 probe: streaming shape, tool calls, `usage` and cached-token reporting, rate-limit headers, batch API, reasoning controls. BLOCKED-ON-ACCESS (NEBIUS_API_KEY).
- [ ] Installed TLS provider check: client must work with aws-lc-rs installed by `main`; gate is `cargo tree -i ring` empty (owned by the app crates, verified here in a smoke test).

**Tests.**

| Category | Cases |
|---|---|
| unit | SSE edge cases (keepalive comments, `[DONE]`, CRLF, empty `data:`, unknown fields); delta decoding; error mapping (401/429/5xx/ContextOverflow); `BaseUrl` rejects plain http for non-loopback |
| proptest | splitting any valid SSE byte stream at arbitrary offsets yields identical events (including multibyte UTF-8); parser output equals the `eventsource-stream` oracle on generated streams |
| fuzz | `SseParser::feed` (WSL2 nightly); delta JSON decoder |
| kani/miri | Kani on frame-size bound arithmetic; Miri on the parser (pure) |
| contract | wiremock synthetic SSE (always runs); live Token Factory (SKIPPED: no key); local llama.cpp/Ollama (runs if the server is up) |
| load | SSE parse throughput in MB/s and frames/s with hdrhistogram latency (targets to measure); counting-allocator test asserts zero allocations in `feed` over a fixed corpus |
| scalability | 1 -> 32 concurrent streams on one HTTP/2 connection pool (targets to measure) |
| stress | 429 storm (burst of 429 with and without `Retry-After`); mid-stream connection reset; slow-loris SSE (bytes every second); 10 MB single frame |
| security | `Authorization` never in `Debug`, logs, errors or panics; redirects disabled (a redirect must not forward the key to another host); key not present in wiremock-recorded request logs outside the header |
| spike | S0-5 feeds back usage/cache/rate-limit header facts |
| fixtures | synthetic SSE corpus with provenance "synthetic"; recorded corpus added later with the model id and date |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| SSE Frame Parsing | struct | `SseParser` | `crates/polaris-nebius-cloud/crates/token-factory/src/sse.rs` | `parses_simple_data_frame`; `handles_crlf_and_split_chunks`; `ignores_comment_keepalive`; `stops_at_done_sentinel`; `rejects_oversized_frame`; `chunk_split_invariance_property` |
| Chat Delta Decoding | function | `decode_delta` | `.../token-factory/src/delta.rs` | `decodes_content_delta`; `decodes_reasoning_delta`; `accumulates_tool_call_by_index`; `truncated_tool_call_is_error`; `reads_usage_when_present` |
| Request Building | struct | `ChatRequestBuilder` | `.../token-factory/src/request.rs` | `build_requires_messages`; `serializes_tools_and_choice`; `omits_unconfirmed_reasoning_params`; `rejects_empty_model_id` |
| Retry and Error Mapping | function | `classify_response` | `.../token-factory/src/retry.rs` | `429_with_retry_after_waits_that_long`; `429_without_header_uses_jittered_backoff`; `401_is_not_retried`; `5xx_retries_bounded`; `context_overflow_is_typed` |

**Risks & open questions.**
- UNVERIFIED: whether Token Factory streams `usage` (needs a `stream_options`-style flag?), cached-token fields, rate-limit headers, a Responses API, a batch API, and reasoning-control parameters (all S0-5; keyed). Telemetry cache numbers stay empty until proven; the keep-warm ping is disabled until S0-5 shows cache behavior.
- UNVERIFIED: the name of the reasoning delta field and tool-call delta layout for Nemotron models served by Token Factory; synthetic fixtures follow the common OpenAI shape and are labelled as such.
- LIKELY: the public `models_info` listing used to verify model ids is a website API, not the stable `/v1/models`; the catalog source of truth at runtime is `list_models`, and the five ids above are fixtures.
- VERIFIED: the chatlog's `api.tokenfactory.nebius.ai` and Nemotron-70B/340B ids are stale; base URL is `.com` and ids are as listed.
- Zero-allocation claim is scoped to SSE framing and routing of the `data:` slice; serde_json decoding into borrowed structs is measured, not claimed allocation-free.
- Local-model walking skeleton exercises the protocol but not Nemotron behavior; do not present it as S2 evidence.

**Chatlog sources.** msg 1 §3.2 and §8.2 (`NebiusClient::stream_chat`, HTTP/2 SSE, `reqwest` + Bearer) - adopted; model ids and `.ai` domain corrected; its `HeaderValue::from_str(...)?` key handling replaced by `SecretString`. msg 9 §1.2 (zero-copy SSE parsing, FinOps budgeting) - adopted as scoped zero-alloc and `$budget` hooks via telemetry; "speculative decoding acceleration" corrected: not client-controllable, `$nebius-speculative` becomes cascade drafting (cheap draft, stronger verify) done in `polaris-runtime`, not here. msg 11 §3 (prefix caching, request coalescing) - prefix-stable prompt assembly is in the context crate; cache-hit reporting depends on S0-5. msg 19 §2 (`x-ratelimit-*` and `retry-after` header parsing, reconcile actual usage) - header names are UNVERIFIED, kept configurable until S0-5. msg 17 (`TokenFactorySemaphore` 4-8 permits) - enforced by the kernel concurrency mesh; this crate exposes no semaphore.


---

### `polaris-nebius-cloud-sandboxes` — Token Factory Sandboxes ("Contree") through the `contree-mcp` sidecar: run, sync, images, `directory_state_id` branching, SWE env resolver

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nebius-cloud/crates/sandboxes` | L2 | lib | `polaris-nebius-cloud` | P3 |

**Purpose.** Implements the sdk `SandboxBackend` for execution tier E3 (the track-literal "write, run, test code in Token Factory"). VERIFIED: Token Factory Sandboxes are VM-isolated, ship 7,000+ preloaded SWE environments (SWE-bench Verified, SWE-rebench, SWE-rebench-V2) and support branching; the SDK is Python-only, so Polaris runs the official MCP server as a supervised sidecar (`pip install contree-mcp`, then `contree-mcp --mode http --http-port 9452`) and talks to it with an `rmcp` streamable-HTTP client. VERIFIED: the server exposes 18 tools: `run`, `rsync`, `import_image`, `list_images`, `get_image`, `set_tag`, `upload`, `download`, `list_files`, `read_file`, `grep`, `get_operation`, `list_operations`, `wait_operations`, `cancel_operation`, `registry_token_obtain`, `registry_auth`, `get_guide`. Branching is by `directory_state_id`; the tree-search strategy on top ("MCTS-style" exploration of N branches) is Polaris's own algorithm over those ids, not a Contree feature. Authentication of the sidecar to Nebius is UNVERIFIED (S0-13).

**API sketch.**
```rust
pub struct StateId(/* directory_state_id as returned by the server */ String);
pub struct ImageRef(String);                               // image tag or id
pub struct OperationId(String);
pub struct Stopped; pub struct Starting; pub struct Ready;               // sidecar typestate
pub struct Sidecar<S> { /* port, venv path, endpoint */ _s: core::marker::PhantomData<S> }
pub struct SidecarCommand { pub program: AbsProgram, pub args: Vec<OsString>, pub env: EnvSpec, pub bind: LoopbackAddr, pub port: u16 }   // handed to kernel supervisor
impl Sidecar<Stopped> { pub fn command(cfg: &SidecarCfg) -> Result<SidecarCommand, SandboxError>; }
impl Sidecar<Starting> { pub async fn wait_ready(self, t: std::time::Duration) -> Result<Sidecar<Ready>, SandboxError>; }
pub struct ContreeBackend { /* rmcp client session, tool inventory (checked against the pinned 18) */ }
impl ContreeBackend {
    pub async fn connect(ep: &SidecarEndpoint) -> Result<ContreeBackend, SandboxError>;                 // tools/list vs pinned inventory
    pub async fn run(&self, r: &RunRequest) -> Result<RunOutcome, SandboxError>;                      // command, image, wait, from: Option<StateId>
    pub async fn branch_run(&self, from: &StateId, cmds: &[RunRequest], budget: BranchBudget) -> Result<Vec<BranchResult>, SandboxError>;
    pub async fn sync_up(&self, plan: &SyncPlan) -> Result<StateId, SandboxError>;                    // only paths approved by storage-sync
    pub async fn read_file(&self, s: &StateId, p: &RemotePath) -> Result<bytes::Bytes, SandboxError>;
    pub async fn wait(&self, ops: &[OperationId]) -> Result<Vec<OperationResult>, SandboxError>;
    pub async fn cancel(&self, op: &OperationId) -> Result<(), SandboxError>;
}
/* impl polaris_sdk::SandboxBackend for ContreeBackend */
pub fn resolve_env(m: &RepoManifest, catalog: &[ImageCandidate]) -> Result<ResolvedImage, SandboxError>;   // exact-match first; ranker injected
pub trait ImageRanker { fn rank(&self, m: &RepoManifest, c: &[ImageCandidate]) -> Result<Vec<Scored>, SandboxError>; }   // S1 wired at composition root
pub enum SandboxError { SidecarUnavailable, ToolInventoryDrift { missing: Vec<String> }, AuthRequired, Remote { op: OperationId, message: String },
                        Timeout, Cancelled, NoMatchingImage, BudgetExceeded, Transport(String) }
```
Tool-name strings and request/response field names above come from the verified tool list; schemas of each tool are API to confirm at implementation (S0-13, `get_guide`).

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-log`. External: `rmcp = 3.5.1` (`default-features = false`, features `client`, `macros`, `transport-child-process`, `transport-streamable-http-client-reqwest`, `reqwest-tls-no-provider`), `tokio = 1.54.0`, `serde_json = 1.0.154`; `+proposed`: `thiserror = 2.0.21`, `serde = 1.0.229`, `bytes = 1.12.1`. Process spawn goes through the sdk `PlatformOps` trait (generic), not through `polaris-platform-process` directly; restart policy is the kernel supervisor's.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Sidecar::command`, `ContreeBackend::connect` (inventory check), `run` (wait / async operation modes), `branch_run` (budgeted fan-out), `sync_up` (approved paths only), `resolve_env`, operation `wait`/`cancel`.
- [ ] Spike S0-13 (keyed): start the sidecar, call `get_guide`, `list_images(tag_prefix)`, `run`, and a branch from a `directory_state_id`; record the real auth mechanism, tool schemas, whether `run` returns a new state id, operation lifetimes and limits. BLOCKED-ON-ACCESS (Contree/Token Factory Sandboxes access and key).
- [ ] Prepare the sidecar install in an isolated venv under `~/.polaris/sidecars/contree` (needs Python and `pip`/`uv`; `uv` is not installed on this machine yet); pin the `contree-mcp` version once S0-13 shows which one works.
- [ ] Bind the sidecar to loopback only (verify the flag/behavior of `--mode http`; if it binds all interfaces, wrap or firewall it) and treat the unauthenticated local MCP port as a credential-bearing endpoint; make the port configurable (default 9452).
- [ ] Implement the tool-inventory check: compare `tools/list` with the pinned 18 and fail closed on missing tools, ignore extras with a warning (drift detection; the server is third-party and may change).
- [ ] Implement `SandboxBackend` mapping: exec -> `run`; file transfer -> `upload`/`download`/`rsync`; reads -> `list_files`/`read_file`/`grep`; long runs -> `get_operation`/`wait_operations`/`cancel_operation`; cancellation must call `cancel_operation`, not just drop the future.
- [ ] Implement `StateId` branching helpers and `BranchBudget` (max concurrent branches 3-5, from the chatlog's `ConTreeSandboxesSemaphore` default; max total VM time); the search policy itself (which branch to expand) stays in `polaris-runtime`.
- [ ] Implement `resolve_env`: exact repo/commit match against catalog entries from `list_images`; then deterministic heuristics; then an injected `ImageRanker` (S1) for ambiguity. Never trust the chatlog's invented `nebius.cr/swe-bench/...` naming; image names come from `list_images`.
- [ ] Treat `registry_token_obtain` / `registry_auth` outputs as secrets (redaction layer, never logged, never in events).
- [ ] Contract tests against a fake MCP server built in-test (needs rmcp `server` features as a dev-dependency; flag for workspace.toml) and later against the live sidecar.
- [ ] Windows specifics: supervise the Python child as a Job-Object tree (kill-on-drop), handle the Windows Firewall prompt for the first listen, use `python -I`-style isolation for the venv, and make sure a stale sidecar from a crashed session is detected by port and PID file.

**Tests.**

| Category | Cases |
|---|---|
| unit | inventory diff (missing / extra tools); request building per tool; `resolve_env` ordering; error mapping; `BranchBudget` enforcement |
| proptest | `resolve_env` is deterministic and total over generated manifests/catalogs; remote path handling never escapes the sandbox root |
| fuzz | MCP result decoding (tool result JSON) with arbitrary shapes |
| contract | fake in-test MCP server (always runs, labelled synthetic) vs live `contree-mcp` (SKIPPED: no key) |
| smoke | start sidecar, `get_guide`, `run echo`, tear down (keyed) |
| load | 5 concurrent branches x 20 runs (keyed; measure latency per run, error rate) |
| scalability | branch fan-out 1 -> 8 against the concurrency budget; record the cost/latency knee |
| stress | kill sidecar mid-run; server returns operation ids after restart; operation timeout; very large `read_file`; network drop to Nebius |
| security | sync of a `.env` / `*.pem` is refused by construction (only `SyncPlan` items accepted); sidecar port not reachable from another host; registry tokens redacted in logs and events |
| spike | S0-13 (auth, branching semantics); S0-5 shares the key |
| fixtures | recorded `get_guide` and tool schemas once available, with provenance |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Tool Inventory Check | function | `check_inventory` | `crates/polaris-nebius-cloud/crates/sandboxes/src/inventory.rs` | `exact_18_tools_pass`; `missing_run_tool_fails_closed`; `extra_tool_is_warning_only`; `empty_list_is_sidecar_error` |
| Sidecar Lifecycle | struct | `Sidecar<S>` | `.../sandboxes/src/sidecar.rs` | `command_binds_loopback_only`; `wait_ready_times_out`; `stale_pid_file_is_detected`; `port_conflict_is_typed_error` |
| Branched Execution | method | `ContreeBackend::branch_run` | `.../sandboxes/src/branch.rs` | `respects_max_concurrent_branches`; `failed_branch_does_not_cancel_siblings`; `cancel_calls_cancel_operation`; `returns_state_id_per_branch` |
| Environment Resolution | function | `resolve_env` | `.../sandboxes/src/resolve.rs` | `exact_commit_match_wins`; `ambiguous_uses_ranker`; `no_candidate_is_typed_error`; `deterministic_for_same_input` |

**Risks & open questions.**
- UNVERIFIED: Contree/sidecar authentication, tool schemas, whether `run` returns a new `directory_state_id`, operation retention, concurrency limits and pricing (S0-13).
- UNVERIFIED: whether `contree-mcp --mode http` binds loopback by default; local port exposure is a credential risk either way.
- UNVERIFIED: `rmcp` 3.5.1 streamable-HTTP client API details (API to confirm at implementation); reqwest in rmcp matches our 0.13 major per the version audit.
- Data egress: files uploaded to Token Factory Sandboxes leave the machine. Consent is explicit (`$sandbox contree` / `$nebius-sandbox`), `.polarisignore` secrets are never sent, and the chatlog's "Zero Data Retention / SOC 2 / HIPAA" statements are UNVERIFIED and must not appear as claims.
- Python dependency: the sidecar needs a Python runtime on the user's machine; this is a deployment burden for judges. Fallback ladder: E1/E2 locally, E4 remote.
- Chatlog's "mount VM in 1.2 s" and "rollback instantly" timings are not evidence; measure them in S0-13.

**Chatlog sources.** msg 9 §1.1, §1.5, §3-§5 (ConTree tree-search with 3 branches, 7,000+ environments, official MCP server, `ContreeCheckpoint` / `fork_branch` / `rollback_branch`, `$nebius-sandbox`, `$nebius-branching`, `/contree-branches`, hybrid local/cloud table) - adopted. Corrected: there is no REST `sandboxes/v1` API in our design (invented URL); integration is via the MCP server; "MCTS" is Polaris's strategy over `directory_state_id`; rollback is "discard the branch state id", not an API call; compliance claims dropped as unverified. msg 11 §1 (System 1 `contree_matcher`, `ContreeEnvironmentResolver` with sled/redb catalog) - adopted via the injected `ImageRanker` and a catalog cache in `polaris-storage-cache` (sled rejected, unmaintained). msg 17 (`ConTreeSandboxesSemaphore` 3-5 permits) - adopted as `BranchBudget` plus the kernel semaphore. msg 19 (registry stores ConTree checkpoint id per session) - `StateId` is persisted by the kernel.


---

### `polaris-nebius-cloud-data-lab` — trajectory warehouse export (JSONL / ShareGPT / ChatML) of verified solutions; fine-tune trigger

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nebius-cloud/crates/data-lab` | L2 | lib | `polaris-nebius-cloud` | P6 |

**Purpose.** Turns sessions that passed Proof-of-Done into training-ready trajectories (prompt, diff, test run, verdict), writes them as JSONL, and uploads them to Nebius Data Lab for later LoRA/full fine-tuning (`/datalab-sync`, `/datalab-finetune`). Export is opt-in (`$nebius-datalab`), only ever carries PII-masked data, and fails closed if a post-mask scan still finds secrets. The whole Data Lab API surface is UNVERIFIED; the local collector and exporter are keyless and buildable first, the upload and the fine-tune trigger are gated on Data Lab documentation and access.

**API sketch.**
```rust
pub struct Draft; pub struct Verified; pub struct Masked; pub struct Exported;          // typestate markers
pub struct Trajectory<S> { /* steps, diff cid, test-run cid, verdict, provenance */ _s: core::marker::PhantomData<S> }
impl Trajectory<Draft> { pub fn verified(self, proof: &ProofOfDoneRecord) -> Result<Trajectory<Verified>, DataLabError>; }
impl Trajectory<Verified> { pub fn mask<R: polaris_sdk::Redactor>(self, r: &R) -> Result<Trajectory<Masked>, DataLabError>; }
pub enum ExportFormat { ShareGptJsonl, ChatMlJsonl }
pub struct TrajectoryExporter { /* output dir, CAS handle, consent flag */ }
impl TrajectoryExporter {
    pub fn new(cfg: ExportConfig, optin: OptIn) -> Result<Self, DataLabError>;
    pub async fn write_local(&self, t: &Trajectory<Masked>, f: ExportFormat) -> Result<ExportReceipt, DataLabError>;
    pub async fn sync<T: DataLabTransport>(&self, tx: &T, batch: &[ExportReceipt]) -> Result<SyncReport, DataLabError>;
}
pub trait DataLabTransport { async fn upload(&self, name: &str, bytes: bytes::Bytes) -> Result<UploadId, DataLabError>;
                             async fn start_finetune(&self, req: &FinetuneRequest) -> Result<JobId, DataLabError>; }   // concrete HTTP impl after docs are verified
pub fn residual_secret_scan(t: &Trajectory<Masked>) -> Result<(), DataLabError>;       // second pass; fails closed
pub enum DataLabError { NotOptedIn, NotVerified, ResidualSecret, SchemaViolation(String), Transport(String), ApiUnverified, Io(std::io::Error) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-storage-cas`. External: `reqwest = 0.13.5` (feature set as in workspace), `serde_json = 1.0.154`; `+proposed`: `serde = 1.0.229`, `thiserror = 2.0.21`, `bytes = 1.12.1`, `tokio = 1.54.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Trajectory` typestate transitions, ShareGPT and ChatML serializers, `residual_secret_scan`, `OptIn` consent gate, batching/rotation of JSONL files.
- [ ] Define the trajectory schema and versioning (messages, tool calls, diff cid, test results, verdict, model id, effort, tool versions, timestamps) and how large blobs are referenced from CAS instead of inlined.
- [ ] Implement ShareGPT-style (`conversations` with from/value) and ChatML-style (`messages` with role/content) JSONL writers; the exact format Data Lab expects is UNVERIFIED, so keep the serializer pluggable.
- [ ] Implement the opt-in gate: nothing is collected without `$nebius-datalab` and a persisted consent record; `/datalab-sync` shows a preview (count, size, sample masked line) before any upload.
- [ ] Implement `mask` via the sdk `Redactor` trait (the PII vault lives at L3 and is injected); then `residual_secret_scan` as a second DFA/entropy pass that blocks export on any hit.
- [ ] Record licence/provenance per trajectory (source repo licence if known, model id) so users can exclude non-redistributable code; flag that training on model outputs may carry licence terms that must be read before use (UNVERIFIED legal point).
- [ ] Read the Data Lab documentation (storage target, auth, formats, fine-tune API, whether LoRA fine-tuning of Nemotron is offered) and write the real `DataLabTransport` impl. BLOCKED-ON-ACCESS (Data Lab docs/access and API key).
- [ ] Wire `/datalab-finetune` only once the API is verified; until then it reports "API unverified" instead of simulating a job.
- [ ] Local-only mode: export to `~/.polaris/datalab/` without any network, usable and testable now.

**Tests.**

| Category | Cases |
|---|---|
| unit | typestate transitions (no `mask` before `verified`, no export before `mask`: negative compile tests); serializer golden outputs |
| proptest | `parse(serialize(t)) == t` for both formats; masked trajectories never contain any of the generated secrets/PII tokens |
| fuzz | JSONL line decoder used by `/datalab-sync` preview |
| security | PII-egress capture test (a fake transport records bytes; assert no planted fake key/email/IP appears); opt-in absent -> zero bytes written; residual-secret corpus blocks export |
| contract | fake `DataLabTransport` (synthetic) now; live API test (SKIPPED: no access) |
| load | export 10k trajectories (targets to measure: throughput, memory) |
| stress | disk full mid-write (atomic temp+rename, no partial files); crash between mask and write |
| fixtures | synthetic trajectories with fake secrets generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Trajectory Typestate | struct | `Trajectory<S>` | `crates/polaris-nebius-cloud/crates/data-lab/src/trajectory.rs` | `unverified_cannot_be_masked`; `mask_replaces_planted_secrets`; `export_requires_masked_state`; `failed_proof_is_rejected` |
| JSONL Serialization | function | `to_sharegpt` / `to_chatml` | `.../data-lab/src/format.rs` | `sharegpt_roundtrip`; `chatml_roundtrip`; `tool_calls_preserved`; `large_blob_is_cas_reference` |
| Residual Secret Scan | function | `residual_secret_scan` | `.../data-lab/src/scan.rs` | `clean_trajectory_passes`; `planted_api_key_blocks`; `high_entropy_string_blocks`; `scan_never_panics_on_binary_like_text` |
| Opt-in Consent Gate | struct | `OptIn` | `.../data-lab/src/consent.rs` | `absent_consent_denies_all`; `consent_is_session_scoped`; `revocation_stops_collection` |

**Risks & open questions.**
- UNVERIFIED: Data Lab's API, storage target ("S3-compatible bucket" in the chatlog), formats, and whether fine-tuning Nemotron via Token Factory is offered; do not implement the upload before reading the docs.
- UNVERIFIED (legal): licence terms for training on model outputs and on third-party repo code.
- Privacy: exporting code is irreversible; default off, preview before upload, per-trajectory exclusion.
- The "flywheel" is a roadmap value, not a measured result; no performance claims until a fine-tune is actually run.

**Chatlog sources.** msg 9 §1.3 and architecture box (trajectory collection, JSONL ShareGPT/ChatML, export to Data Lab bucket, LoRA/full fine-tuning via Token Factory), §4.1-§4.2 (`$nebius-datalab`, `/datalab-sync`) - adopted; the claim "agents forget experience" is marketing, and the upload/fine-tune parts are deferred until the API is verified. msg 19 §1.1 (Trajectory Warehouse buffer) - the buffer is CAS + local JSONL. msg 1 and msg 21 (PII masking before cloud) - enforced through `Redactor` and the residual scan.

---

### `polaris-nebius-cloud-ai-cloud` — Nebius AI Cloud: remote OpenShell gateway VM lifecycle (E4) and cloud bursting

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nebius-cloud/crates/ai-cloud` | L2 | lib | `polaris-nebius-cloud` | P6 |

**Purpose.** Provisions, starts, stops and deletes a Linux VM on Nebius AI Cloud that runs a remote OpenShell gateway (execution tier E4), and hands connection details (mTLS endpoint) to `polaris-openshell-client` through an sdk type. E4 is the fallback when nested KVM or WSL2 is unavailable (S0-2) and the path for heavy builds. It is cost-sensitive: a forgotten VM bills, so every VM has an idle auto-stop, a hard TTL and a budget check. Nothing about the Nebius AI Cloud API is verified yet (UNVERIFIED); no official Rust SDK is known to us (LIKELY), so the integration path is decided by a keyed spike.

**API sketch.**
```rust
pub struct Absent; pub struct Provisioning; pub struct Running; pub struct Stopped; pub struct Deleted;   // typestate markers
pub struct GatewayVm<S> { /* project, vm id, region, created_at, ttl */ _s: core::marker::PhantomData<S> }
pub struct VmSpec { pub platform: PlatformName, pub preset: PresetName, pub disk_gib: u32, pub image: ImageRef, pub ttl: std::time::Duration,
                    pub idle_stop: std::time::Duration, pub ingress: IngressRule /* allow-list of operator IPs */ }
pub struct AiCloud { /* api client (gRPC via tonic or CLI runner: decided by spike), credentials handle, project id */ }
impl AiCloud {
    pub async fn connect(cfg: &AiCloudConfig) -> Result<AiCloud, CloudError>;
    pub async fn quota(&self) -> Result<QuotaReport, CloudError>;
    pub async fn ensure_gateway_vm(&self, spec: &VmSpec, budget: &BudgetGuard) -> Result<GatewayVm<Running>, CloudError>;
}
impl GatewayVm<Running> {
    pub fn endpoint(&self) -> Result<polaris_sdk::GatewayEndpoint, CloudError>;       // consumed by openshell-client (mTLS material attached)
    pub async fn stop(self) -> Result<GatewayVm<Stopped>, CloudError>;
    pub async fn delete(self) -> Result<GatewayVm<Deleted>, CloudError>;
}
impl GatewayVm<Stopped> { pub async fn start(self) -> Result<GatewayVm<Running>, CloudError>; }
pub struct BudgetGuard { /* max hourly cost, max total, price table from config */ }
pub enum CloudError { Unauthorized, QuotaExceeded(String), BudgetExceeded, NotFound, Timeout, ApiUnverified, Transport(String), Config(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`. External: `reqwest = 0.13.5`, `tonic = 0.14.6`; `+proposed`: `thiserror = 2.0.21`, `tokio = 1.54.0`, `serde = 1.0.229`, `serde_json = 1.0.154`, possibly `rcgen = 0.14.10` if Polaris must mint client certificates (decide after reading the gateway's TLS model).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `BudgetGuard` checks, `VmSpec` validation, `GatewayVm` state transitions, TTL/idle auto-stop scheduler, endpoint hand-off (`GatewayEndpoint` construction).
- [ ] Research spike (keyless first): read the Nebius AI Cloud API/CLI documentation and decide among (a) gRPC with generated clients if public protos exist (tonic), (b) shelling out to the `nebius` CLI through the sdk `PlatformOps` trait, (c) REST if available. Record the decision; do not hand-write unverified endpoints.
- [ ] Read how authentication works (service account / IAM token exchange are LIKELY but unverified) and store credentials only via the secrets vault references.
- [ ] Define the VM image strategy: Ubuntu 24.04 + OpenShell v0.1.4 + Polaris sandbox image via the same blueprint/onboard plan as local (`polaris-openshell-nemoclaw`), applied over SSH or cloud-init. BLOCKED-ON-ACCESS (Nebius AI Cloud account, project, quota; request on day 1 per D9).
- [ ] Decide the network exposure: gateway port reachable only from the operator's IP with mTLS, or via an SSH tunnel; no public unauthenticated gateway ever. (WireGuard mesh is v2.)
- [ ] Implement the cost controls: idle auto-stop, absolute TTL, `BudgetGuard` that refuses creation above the budget, and a startup sweep that lists and offers to delete leftover Polaris-tagged VMs.
- [ ] Implement `ensure_gateway_vm` idempotently (tag-based lookup before create) and the endpoint/cert hand-off to the client.
- [ ] Check whether the chosen VM platform offers nested virtualization (E4 with the `vm` driver) or only the docker driver. BLOCKED-ON-ACCESS (live VM).
- [ ] Optional later: GPU preset for CUDA builds (H100-class per chatlog; availability and price UNVERIFIED), batch jobs; keep out of the MVP.

**Tests.**

| Category | Cases |
|---|---|
| unit | `BudgetGuard` arithmetic (no overflow, rounding up); state transitions (negative compile tests: no `endpoint()` unless `Running`); TTL scheduler with a fake clock |
| proptest | cost estimate is monotonic in duration and preset price; sweep never selects untagged VMs |
| contract | synthetic fake API (wiremock or in-test gRPC) always; live API (SKIPPED: no account) |
| smoke | create a smallest CPU VM, run `echo` via the remote gateway, delete (keyed; records cost) |
| stress | API timeouts mid-provision; VM stuck in provisioning; leftover VM after a crash of Polaris; quota exhausted |
| security | no credentials in logs or errors; ingress rule rejects `0.0.0.0/0` by default; client refuses a gateway without mTLS |
| spike | S0-2 fallback path (E4) and the P6 cloud-bursting decision |
| fixtures | synthetic API responses; recorded ones later with provenance |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Budget Guard | struct | `BudgetGuard` | `crates/polaris-nebius-cloud/crates/ai-cloud/src/budget.rs` | `refuses_creation_above_budget`; `accumulates_running_cost`; `rounds_partial_hours_up`; `zero_budget_denies_all` |
| VM Lifecycle Typestate | struct | `GatewayVm<S>` | `.../ai-cloud/src/vm.rs` | `endpoint_only_when_running`; `stop_then_start_roundtrip`; `delete_is_terminal`; `ensure_is_idempotent_by_tag` |
| Auto-stop Scheduler | struct | `LifetimeGuard` | `.../ai-cloud/src/lifetime.rs` | `idle_timeout_stops_vm`; `ttl_overrides_activity`; `activity_resets_idle_timer`; `crash_recovery_finds_leftovers` |
| Ingress Rules | function | `validate_ingress` | `.../ai-cloud/src/ingress.rs` | `rejects_open_world_cidr`; `accepts_single_operator_ip`; `requires_mtls_flag`; `ipv6_is_validated` |

**Risks & open questions.**
- UNVERIFIED: the entire Nebius AI Cloud control-plane API, auth model, quotas, presets, nested-virtualization support and prices; the chatlog's `polaris-nebius-cloud::iaas` / Slurm client is not evidence.
- Cost risk: unattended VMs; mitigations are structural (TTL, idle stop, budget guard, tagged sweep), and tests assert them.
- Security risk: a remote gateway that executes arbitrary code is a high-value target; mTLS, IP allow-list and no secrets stored on the VM beyond what provider profiles need.
- The E4 fallback for S0-2 only helps if the cloud gateway can run an acceptable isolation driver; check that before promising it.
- reqwest 0.13 and tonic 0.14 are listed; whether both are really needed depends on the spike decision (CLI-runner path needs neither).

**Chatlog sources.** msg 9 §1.4 and §2 (cloud bursting: ephemeral H100 VM for heavy builds, `nebius compute instance create`, MK8s, Slurm submit, `iaas` module) - adopted as the E4 gateway VM and optional bursting; Slurm/MK8s deferred (no verified API). msg 35 (remote headless daemon on Nebius AI Cloud 8x H100, mTLS) - maps to E4 now and the v2 self-hosting daemon later. msg 37 (sovereign self-host on a Nebius node, WireGuard) - v2. msg 1 §11 (hackathon alignment) - AI Cloud is the stated alternative to Token Factory for the "must run on Nebius" requirement.


---

### `polaris-devenv` — facade over the development-environment adapters (nix, docker, cicd)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-devenv` | L2 | facade | (top-level) | P8 |

**Purpose.** Pure re-export crate with no logic. It documents the `$env(nix|docker|host)` and `$ci(dagger)` selection semantics and how they combine with `$sandbox`; the selection itself is performed at the composition root.
**Re-exports.** `pub use polaris_devenv_nix as nix;` `pub use polaris_devenv_docker as docker;` `pub use polaris_devenv_cicd as cicd;` (`pub use polaris_devenv_*`). Children never depend on this facade.

**Selection semantics to keep in the crate docs.**

| `$env` | Meaning | Where commands run | Notes |
|---|---|---|---|
| `host` (default) | no environment wrapping | whatever `$sandbox` selects | Windows toolchain only when no sandbox (E0 read-only) |
| `nix` | hermetic toolchain from `flake.nix` | Linux only: inside WSL2/OpenShell sandbox or Contree image; Nix is not native on Windows | needs Nix installed in the execution environment |
| `docker` | pre-warmed container from the host Docker engine | Docker Desktop engine via named pipe | a container, not an OpenShell policy envelope; weaker guarantees than E1/E2 |

`$ci(dagger)` switches build/test runs to the CI engine (feature-gated). `$env` and `$sandbox` are independent axes: `$sandbox` picks where code is isolated; `$env` picks how its toolchain is provided.

**TODO.**
- [ ] Re-export the three children; compile-only doctest naming one item from each.
- [ ] Write the selection table above into `//!` docs, including the unsupported combinations (nix on native Windows, docker env inside E3).
- [ ] Specify the `$env` x `$sandbox` resolution function signature for the composition root and add its test list to the kernel crate's descriptors (cross-reference only; no logic here).
- [ ] Record the reqwest-0.11 feature-gate rule for `dagger-sdk` in the crate docs so the default feature set stays free of it.

---

### `polaris-devenv-nix` — Nix integration: `nix print-dev-env --json`, hermetic environments for `$env(nix)`, `/nix-flake` generation via S2

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-devenv/crates/nix` | L2 | lib | `polaris-devenv` | P8 |

**Purpose.** Evaluates a flake dev shell into an environment map that can be injected into sandbox commands, and validates `flake.nix` drafts. Nix is Linux/macOS software, so on this Windows machine it runs inside WSL2 or inside a sandbox image, never natively; the crate therefore builds command specs and parses results while the composition root executes them through `SandboxBackend`/`PlatformOps`. Flake synthesis is S2's job (S1 cannot write a `flake.nix`); this crate only validates the draft by evaluation. Correction to the chatlog: `nix print-dev-env --json` is run through the Nix CLI, not by talking to the daemon socket.

**API sketch.**
```rust
pub struct Unevaluated; pub struct Evaluated;                                 // typestate markers
pub struct DevEnv<S> { /* flake ref, lock digest, env map when evaluated */ _s: core::marker::PhantomData<S> }
pub struct EnvMap(std::collections::BTreeMap<EnvName, EnvValue>);               // sanitized
pub fn eval_command(flake: &FlakeRef, opts: &NixOpts) -> Result<NixCommand, NixError>;         // argv for `nix print-dev-env --json` (flags to confirm)
pub fn parse_dev_env(json: &[u8]) -> Result<RawDevEnv, NixError>;                               // size-limited
pub fn sanitize(raw: RawDevEnv, policy: &EnvSanitizePolicy) -> Result<EnvMap, NixError>;        // drops HOME/TMP*/NIX_BUILD_*/SHELL etc.
pub fn cache_key(flake_lock: &[u8], system: &SystemTriple, nix_version: &str) -> CacheKey;      // blake3 (via sdk helper)
pub fn check_command(flake_dir: &LinuxPath) -> NixCommand;                                       // eval/check of a drafted flake
pub fn flake_draft_prompt(manifests: &ProjectManifests) -> Result<String, NixError>;            // prompt for S2; no model call here
pub fn validate_flake_text(text: &str) -> Result<FlakeLint, NixError>;                           // static checks before evaluation
pub enum NixError { NotInstalled, EvalFailed { stderr: String }, JsonTooLarge, JsonShape(String), UnsafeFlake(String), Timeout, Io(std::io::Error) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `serde_json = 1.0.154`, `tokio = 1.54.0`; `+proposed`: `serde = 1.0.229`, `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `eval_command`, `parse_dev_env`, `sanitize`, `cache_key`, `validate_flake_text`, `flake_draft_prompt`.
- [ ] Install Nix inside WSL2 (after Ubuntu exists) and capture the real `nix print-dev-env --json` output of a trivial and a CUDA/Python flake as fixtures; confirm the JSON shape (variables map with per-variable type is LIKELY, not verified) and required flags (`--extra-experimental-features nix-command flakes` is LIKELY). BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 + Nix install).
- [ ] Implement `parse_dev_env` with strict size limits and tolerance for array/associative variable kinds; only exported scalar variables are injected by default.
- [ ] Implement `sanitize`: drop build-sandbox leftovers (`HOME=/homeless-shelter`, `TMP*`, `NIX_BUILD_*`, `SHELL`), never forward host secrets, keep `PATH` entries that point into `/nix/store`.
- [ ] Decide how `/nix/store` paths are made valid in a sandbox (bind-mount the store read-only, or run `nix` inside the sandbox image); a `PATH` into `/nix/store` is meaningless in a container without the store. Write the decision as an ADR.
- [ ] Implement `validate_flake_text` (static rejects: `builtins.exec`, impure `getEnv`/`currentSystem` misuse, unpinned `fetchurl` without hash, absolute host paths) and require evaluation to run inside a sandbox with network policy, because evaluating or building an untrusted flake executes arbitrary code and fetches inputs.
- [ ] Implement the S2 flake-draft prompt builder from project manifests (`Cargo.toml`, `requirements.txt`, CUDA sources); the draft is validated, shown to the user, and only then written.
- [ ] Implement result caching keyed by `flake.lock` digest + system + nix version (cache storage lives in `polaris-storage-cache`, wired by the composition root).
- [ ] `/nix-flake` command smoke on a real repo. BLOCKED-ON-ACCESS (Nix in WSL2; S2 access for the draft: Token Factory key or local model).

**Tests.**

| Category | Cases |
|---|---|
| unit | JSON parse of recorded outputs (scalar, array, unset, unicode values); sanitize table; command argv construction; cache key stability |
| proptest | `sanitize(parse(x))` never contains denied names; parser total on arbitrary JSON of bounded size (returns Err, never panics) |
| fuzz | `parse_dev_env` and `validate_flake_text` (WSL2 nightly) |
| smoke | `nix --version` and `print-dev-env` for a minimal flake inside WSL2 (blocked until Nix installed) |
| contract | recorded real outputs (SKIPPED until captured; synthetic fixtures labelled) |
| security | hostile flake corpus (`builtins.exec`, network fetch without hash, path traversal inputs) rejected statically or confined; env output with secrets stripped; evaluation requires a sandbox |
| load | dev-shell evaluation latency cold vs cached (targets to measure; Nix evaluation is typically seconds, not milliseconds) |
| fixtures | recorded JSON outputs with the nix version and date |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Dev-Env JSON Parsing | function | `parse_dev_env` | `crates/polaris-devenv/crates/nix/src/parse.rs` | `parses_exported_scalars`; `skips_functions_and_arrays_by_default`; `rejects_oversized_json`; `malformed_json_is_typed_error` |
| Environment Sanitization | function | `sanitize` | `.../nix/src/sanitize.rs` | `drops_homeless_shelter_home`; `drops_nix_build_vars`; `keeps_store_path_entries`; `never_forwards_host_secret_names` |
| Flake Static Validation | function | `validate_flake_text` | `.../nix/src/flake.rs` | `rejects_builtins_exec`; `rejects_unhashed_fetch`; `accepts_pinned_inputs`; `rejects_host_absolute_paths` |
| Evaluation Cache Key | function | `cache_key` | `.../nix/src/cache.rs` | `same_lock_same_key`; `lock_change_changes_key`; `nix_version_is_part_of_key`; `system_triple_is_part_of_key` |

**Risks & open questions.**
- UNVERIFIED: JSON format of `print-dev-env --json` and the required flags on the Nix version we install; fixtures first.
- UNVERIFIED: making `/nix/store` available inside OpenShell sandboxes (mount vs in-image Nix). This decides whether `$env(nix)` is viable on E1/E2 or only on a dedicated image.
- UNVERIFIED: Nix installation inside WSL2 Ubuntu 24.04 (single-user vs daemon, systemd), disk and time cost (first evaluations download GBs).
- Security: evaluating an untrusted `flake.nix` can fetch and build; treat as code execution.
- The chatlog's "S1 generates a flake in 15 ms" is false: the draft comes from S2 and takes seconds. P8 priority is "could"; do not block the MVP on it.

**Chatlog sources.** msg 41 §1-§2, §5.3 and `$env(nix)`, `/nix-flake` (hermetic dev shells, flake auto-synthesis, `NixEnvironment::evaluate_dev_shell`) - adopted. Corrected: the CLI runs `nix print-dev-env --json` (no daemon-socket path); flake synthesis is S2's task (S1 is classification/extraction); Nix is Linux-side only on this machine; "byte-reproducible on any machine" is not guaranteed without a locked flake and is reported as LIKELY per lock. msg 37 §1.1 (NixOS flake for self-hosting) - v2, out of scope here.

---

### `polaris-devenv-docker` — Docker/Podman through `bollard`: image builds, warm container pool, exec

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-devenv/crates/docker` | L2 | lib | `polaris-devenv` | P8 |

**Purpose.** Talks to the host Docker engine (Docker Desktop on this machine, Docker 29.6.1 VERIFIED present) over the Windows named pipe with `bollard`, without parsing CLI output. It builds images, keeps a bounded pool of pre-started containers for fast `$env(docker)` test runs, and executes commands with streamed output. Correction: a Docker "pause/unpause" does not reset the writable (copy-on-write) layer, so reuse never relies on it; a used container is destroyed and the pool is replenished. This is a performance and reproducibility tool; the containers it creates are not inside an OpenShell policy envelope, so the crate hardens them itself and the docs say so.

**API sketch.**
```rust
pub struct DockerEndpoint { /* named pipe path on Windows, unix socket on Linux; Podman variants */ }
pub struct ContainerSpec { pub image: ImageRef, pub cmd: Vec<String>, pub env: EnvSpec, pub mem_limit: ByteSize, pub cpus: CpuQuota,
                           pub network: NetworkMode /* default None */, pub readonly_rootfs: bool, pub mounts: Vec<MountSpec> }
pub struct Docker { /* bollard client + negotiated API version */ }
impl Docker {
    pub async fn connect(ep: &DockerEndpoint) -> Result<Docker, DockerError>;
    pub async fn build_image(&self, ctx: &BuildContext, opts: &BuildOpts) -> Result<ImageId, DockerError>;   // BuildKit availability to confirm
    pub async fn create(&self, spec: &ContainerSpec) -> Result<Container<Created>, DockerError>;
}
pub struct Created; pub struct Started; pub struct Used; pub struct Removed;                                 // typestate markers
pub struct Container<S> { /* id */ _s: core::marker::PhantomData<S> }
impl Container<Created> { pub async fn start(self) -> Result<Container<Started>, DockerError>; }
impl Container<Started> {
    pub async fn exec(&self, req: &ExecRequest) -> Result<ExecStream, DockerError>;
    pub async fn finish(self) -> Result<Container<Used>, DockerError>;
}
impl Container<Used> { pub async fn remove(self) -> Result<Container<Removed>, DockerError>; }
pub struct WarmPool { /* target size, max age, memory budget, spec */ }
impl WarmPool {
    pub fn new(docker: Docker, spec: ContainerSpec, cfg: PoolConfig) -> Result<Self, DockerError>;
    pub async fn acquire(&self) -> Result<Container<Started>, DockerError>;              // single-use; pool refills in the background
    pub async fn drain(&self) -> Result<(), DockerError>;                                   // Reaper hook
}
pub enum DockerError { EngineUnavailable, ApiVersion(String), ImageNotFound(String), Exec { code: i64 }, PoolExhausted, Timeout, Build(String), Transport(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `bollard = 0.21.1` (default features `http`, `pipe`; no TLS, which avoids `ring`), `tokio = 1.54.0`, `futures = 0.3.34`; `+proposed`: `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Docker::connect` (endpoint autodetect + API negotiation), `ContainerSpec` hardening defaults, `WarmPool::acquire`/refill, `Container` lifecycle typestate, exec stream mapping, `build_image`.
- [ ] Implement connection to Docker Desktop's named pipe and a Podman variant; report engine/API versions and which backend (Docker Desktop vs an engine inside Ubuntu) is in use. UNVERIFIED which engine OpenShell's docker driver uses; do not assume they are the same daemon.
- [ ] Implement hardened defaults: no network, read-only rootfs plus tmpfs, `no-new-privileges`, all capabilities dropped, memory/CPU/pids limits, non-root user, never mount the Docker socket, no bind mount of the Windows workspace by default (use a volume plus the sync protocol; Windows bind mounts are slow and change line endings semantics).
- [ ] Implement `WarmPool` with single-use containers: pre-create/start N containers, hand one out, remove it after use, refill asynchronously within a memory budget aware of the WSL cap (about 6 GB planned). No unpause-reset assumption anywhere.
- [ ] Implement `exec` with streamed output, exit code, caps and cancellation (kills the exec process and verifies it is gone).
- [ ] Investigate BuildKit through `bollard` (cache mounts need BuildKit; bollard's BuildKit support is likely behind a non-default feature that the pinned feature set does not enable). Decide between enabling that feature, or building through the engine CLI via `PlatformOps`. Record as an ADR. UNVERIFIED.
- [ ] Register pool containers with the Reaper; detect and remove orphaned labelled containers on startup.
- [ ] Measure cold vs warm start latency on this machine (Docker Desktop) and record; do not assume the chatlog's near-instant figures.
- [ ] Rootless Podman path is Linux-only; keep behind the Linux leg and do not claim it on Windows.

**Tests.**

| Category | Cases |
|---|---|
| unit | `ContainerSpec` hardening defaults; typestate negative compile tests; pool sizing math; error mapping from engine responses |
| proptest | pool never exceeds memory budget for generated acquire/release sequences; container names/labels never collide |
| contract | fake engine (in-test HTTP server speaking a small subset of the Engine API, labelled synthetic) vs live Docker Desktop (runs here if the engine is up; this machine has Docker 29.6.1) |
| smoke | run `echo` in a pulled small image, check exit code, container removed (live) |
| load | 100 acquire/exec/remove cycles; time-to-first-exec warm vs cold (targets to measure) |
| scalability | pool sizes 1 -> 16 until the memory budget; observe Docker Desktop/WSL pressure |
| stress | engine restart mid-exec; Docker Desktop stopped; image pull failure; disk full; OOM-killed container |
| security | container cannot see host env or secrets; no socket mount; privileged flags impossible through the public API; labelled-orphan cleanup |
| spike | none of S0-1..S0-13; P8 mini-spike for BuildKit |
| fixtures | tiny image reference list; no large images committed |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Container Hardening | function | `ContainerSpec::hardened` | `crates/polaris-devenv/crates/docker/src/spec.rs` | `defaults_drop_all_caps`; `defaults_disable_network`; `socket_mount_is_rejected`; `memory_limit_is_required` |
| Warm Pool | struct | `WarmPool` | `.../docker/src/pool.rs` | `acquire_returns_started_container`; `used_container_is_removed_not_reused`; `refill_respects_memory_budget`; `drain_removes_all` |
| Container Lifecycle | struct | `Container<S>` | `.../docker/src/container.rs` | `create_start_exec_finish_remove_order`; `exec_before_start_does_not_compile`; `remove_is_idempotent`; `orphan_scan_finds_labelled` |
| Exec Streaming | async_function | `Container::exec` | `.../docker/src/exec.rs` | `streams_stdout_stderr`; `reports_exit_code`; `cancel_kills_exec_process`; `output_cap_truncates` |

**Risks & open questions.**
- UNVERIFIED: BuildKit and cache-mount support through `bollard` 0.21.1 with the pinned feature set.
- UNVERIFIED: Docker Desktop named-pipe behavior under the LTSC build and when the engine is stopped (error shape); Podman on Windows connectivity.
- VERIFIED correction: a paused container keeps its writable layer; "reset CoW on unpause" is false. Single-use containers are the only reset we trust.
- Security: weaker than OpenShell (no Landlock/policy engine); `$env(docker)` must not be presented as an equivalent isolation tier. The default for untrusted code remains E1/E2.
- Interaction with the WSL memory cap: Docker Desktop's own WSL VM competes with the Ubuntu distro for the 16 GB host.

**Chatlog sources.** msg 41 §3 and §5.1 (`FastDockerEngine` via `bollard`, named pipe / unix socket, warm container pool with `container_unpause` and "reset CoW layer", rootless Podman, `$env(docker)`) - adopted with corrections: unpause does not reset the CoW layer (use single-use containers); the chatlog's `connect_with_local_defaults` is replaced by explicit endpoint detection; rootless Podman is a Linux-only claim. msg 1 §4.1 (Docker/Podman as OpenShell back-ends) - lives in the OpenShell child, not here.

---

### `polaris-devenv-cicd` — built-in CI (`/ci`, `/ci-run`, `$ci(dagger)`): pipelines with persistent caches; feature-gated `dagger-sdk`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-devenv/crates/cicd` | L2 | lib | `polaris-devenv` | P8 |

**Purpose.** Defines Polaris CI pipelines (lint, build, test, matrix legs) as a typed model, runs them through an engine, and returns a `CiReport` that feeds Proof-of-Done. The default engine runs each step through the sdk `SandboxBackend` (OpenShell or Contree), so CI works without Dagger. The Dagger engine is an optional accelerator behind a cargo feature because `dagger-sdk` 0.21.10 pulls reqwest 0.11 (VERIFIED coupling) and needs a running Dagger engine: the SDK provisions/uses a Dagger engine (a container run by a Docker-compatible runtime), so "Dagger on BuildKit" requires Docker (Docker Desktop here) and is not serverless. Time-saving figures in the chatlog (180 s -> 2.4 s) are targets to measure, not facts.

**API sketch.**
```rust
pub struct Step { pub name: StepName, pub image: ImageRef, pub cmd: Vec<String>, pub cache_mounts: Vec<CacheMount>, pub env: EnvSpec, pub timeout: std::time::Duration }
pub struct Pipeline<S> { /* steps, matrix */ _s: core::marker::PhantomData<S> }                 // S in {Draft, Validated}
impl Pipeline<Draft> { pub fn validate(self) -> Result<Pipeline<Validated>, CiError>; }
pub struct Matrix { pub legs: Vec<Leg> }                                                         // e.g. linux-x86_64; cuda build-only leg; minimal-versions
pub trait CiEngine { async fn run(&self, p: &Pipeline<Validated>, m: &Matrix) -> Result<CiReport, CiError>; }
pub struct SandboxEngine<B: polaris_sdk::SandboxBackend> { /* default engine */ }
#[cfg(feature = "dagger")] pub struct DaggerEngine { /* dagger-sdk session */ }
pub struct CiReport { pub steps: Vec<StepReport>, pub verdict: Verdict, pub started: jiff::Timestamp, pub engine: EngineId }
pub struct StepReport { pub name: StepName, pub exit: i32, pub duration: std::time::Duration, pub cache: CacheStatus /* Hit | Miss | Unknown */, pub log_cid: Cid }
pub fn rust_test_pipeline(ws: &WorkspaceFacts) -> Result<Pipeline<Draft>, CiError>;
pub enum CiError { InvalidPipeline(String), EngineUnavailable, StepTimeout(StepName), Cancelled, EngineFailure(String) }
```

**Dependencies.** Internal: `polaris-sdk`. External: `dagger-sdk = 0.21.10` (optional, cargo feature `dagger`, pulls reqwest 0.11), `tokio = 1.54.0`; `+proposed`: `thiserror = 2.0.21`, `jiff = 0.2.38`. Default features: none of the Dagger stack; `cargo tree -d` must show reqwest 0.11 only with `--features dagger`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `Pipeline::validate`, `rust_test_pipeline`, `SandboxEngine::run` (step ordering, timeouts, report assembly), matrix expansion, `CiReport` verdict rules, cache-status mapping.
- [ ] Implement the pipeline model and validation (unique step names, acyclic dependencies, bounded timeouts, no secrets in `env`).
- [ ] Implement `SandboxEngine` over the sdk `SandboxBackend` with per-step logs stored in CAS and a verdict mapping that Proof-of-Done can consume (command, exit code, duration, engine, conditions).
- [ ] Implement matrix legs honestly: Linux x86_64 test leg; CUDA "Hopper target" can only be a build check here (the GTX 1650 is sm_75 and cannot run it); `minimal-versions` needs nightly tooling; mark unavailable legs as SKIPPED, not PASS.
- [ ] Add the `dagger` feature: spike whether `dagger-sdk` 0.21.10 compiles on windows-msvc, connects through Docker Desktop, supports cache volumes and reports cache hits per step; fall back to invoking the `dagger` CLI through `PlatformOps` or dropping the accelerator. UNVERIFIED; P8 "could".
- [ ] Define cache semantics: cargo registry and `target`/sccache directories as named cache mounts; document what a "cache hit" means per engine and report `Unknown` instead of guessing.
- [ ] Verify the reqwest-0.11 isolation: default build tree free of it (CI gate in `xtask`).
- [ ] Wire `/ci`, `/ci-run` output structure to `CiReport` (UI formatting lives in the UI crate).
- [ ] Run a real pipeline against this repo through an OpenShell sandbox. BLOCKED-ON-ACCESS (WSL2 Ubuntu + OpenShell); Dagger leg also needs a Docker-compatible runtime (Docker Desktop available).

**Tests.**

| Category | Cases |
|---|---|
| unit | pipeline validation (cycles, duplicate names, missing image); verdict rules (any failed step -> Fail; skipped legs reported SKIPPED); report serialization |
| proptest | matrix expansion size and uniqueness; validated pipelines always have a topological order |
| contract | fake `SandboxBackend` implementing exec results (declared test double); live OpenShell run later; Dagger engine tests only under the feature and when an engine is reachable |
| smoke | run a trivial two-step pipeline end to end (needs a sandbox: blocked until P0) |
| load | 50 pipeline runs, report overhead per step (targets to measure) |
| scalability | matrix with 1 -> 16 legs concurrently under the sandbox budget |
| stress | step timeout, backend crash mid-step, cancellation tears down sandboxes (Reaper) |
| security | secrets never passed via `env`; cache mounts are per-workspace (no cross-project cache poisoning); logs redacted before storage |
| fixtures | tiny sample project (generated at test time) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Pipeline Validation | method | `Pipeline::validate` | `crates/polaris-devenv/crates/cicd/src/pipeline.rs` | `rejects_duplicate_step_names`; `rejects_dependency_cycle`; `rejects_secret_looking_env`; `accepts_linear_pipeline` |
| Sandbox Engine Run | async_method | `SandboxEngine::run` | `.../cicd/src/engine.rs` | `runs_steps_in_order`; `stops_after_first_failure_when_configured`; `step_timeout_reports_timeout`; `cancel_tears_down_backend` |
| Matrix Expansion | function | `Matrix::expand` | `.../cicd/src/matrix.rs` | `expands_cartesian_legs`; `unavailable_leg_is_skipped_not_passed`; `leg_names_are_unique` |
| CI Verdict | function | `CiReport::verdict` | `.../cicd/src/report.rs` | `all_pass_is_pass`; `any_fail_is_fail`; `skipped_does_not_count_as_pass`; `unknown_cache_status_is_preserved` |

**Risks & open questions.**
- UNVERIFIED: `dagger-sdk` 0.21.10 on windows-msvc with Docker Desktop; engine provisioning needs network and a container runtime; per-step cache-hit reporting through the SDK.
- VERIFIED coupling: `dagger-sdk` pulls reqwest 0.11 (a third reqwest major); hence the optional feature.
- Honest CI semantics: a green run in a warm cache is not evidence the code builds from scratch; reports carry cache status and engine, and Proof-of-Done records the conditions.
- Scope risk: P8 "could"; this crate must not block MVP. The sandbox-backed default engine makes `/ci` useful without Dagger.
- Clippy and similar linters can be pipeline steps with default settings; no repository lint configuration is introduced (D10).

**Chatlog sources.** msg 41 §1, §4, §5.2 (`DaggerCiEngine`, Dagger + BuildKit, persistent cache mounts for cargo registry and sccache, DAG step caching, local matrix CI, `/ci`, `/ci-run`, `$ci(dagger)`, Proof-of-Done integration) - adopted with corrections: Dagger needs an engine (a container runtime) and is not a library-only dependency; the SDK pins an old reqwest, so it is feature-gated; time savings are targets; the sample's `rust:1.85` image is replaced by the pinned toolchain (1.99.0) from `rust-toolchain.toml`; CUDA legs on this GPU are build-only. msg 1 §9 (verification loop, max 3 retries) - consumes `CiReport`.

---

## 13.4 Group G4 — Cognition & knowledge

Scope: 18 packages (System One x8, search x3, ext x3, ingest, nvidia-docs, browser, hacker). Labels follow the AGENTS.md kernel: VERIFIED = checked in this session or stated as verified in the PLAN core (§0–§12); LIKELY = consistent with context, not verified; UNVERIFIED/GUESSING = unchecked. Every number marked "target" is a budget to be measured, never a result. Nothing in this section has been run.

Cross-group design notes that apply to all G4 crates (stated once, referenced below):

- **S1 access path.** Consumers never import `polaris-system-one-*` directly across sibling layers. They call the sdk `DecisionEngine` trait with a `SchemaId`; schema data (questions, thresholds, fail mode) lives in `polaris-system-one-schemas` and reaches the L2 client through an sdk-level `SchemaCatalog` trait injected at the composition root. This is how `search-sift` (L3), `nvidia-docs` (L3) and `ext-skills` (L2) use S1 without a forbidden edge. (PROPOSED: needs `SchemaCatalog` in `polaris-sdk` and `workspace.toml` agreement.)
- **Untrusted text.** Everything S1 classifies (tool descriptions, fetched pages, documents, diffs, a11y text) is attacker-influenced. S1 output may *escalate* (block, ask, drop to lower trust) but never *downgrade* a deterministic DFA/AST block. Biscuit bounds injection damage; it does not prevent it.
- **No fabricated timings.** The chatlog's "<15 ms / 18 ms / 45 ms" figures are rejected. The vendor CPU figure for GLiNER2.5-Decide is about 167 ms on 48 cores (VERIFIED, plan item 6); on the Ryzen 7 2700 (8c/16t) it is UNMEASURED and expected to be slower. All latency targets below are placeholders for spike S0-4 to fill in.

---

### `polaris-system-one` — facade for the typed System-One cognitive coprocessor (Choice / Score / Noul)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one` | L3 | facade | — | P4 |

**Purpose.** Re-exports the seven System-One children and owns the crate-level documentation of the System One contract: the typed question/decision API, schema calibration, the fail-open/closed matrix, batching and caching, and the 16 GB memory budget. System One is TypeSafe's concept with primitives Choice, Score and Noul (VERIFIED, plan item 5). Polaris realizes it with `fastino/GLiNER2.5-Decide`, a GLiNER2 span checkpoint on DeBERTa-v3-large (24 layers, 1.95 GB fp32 safetensors, no ONNX shipped; VERIFIED, plan item 6) plus the PII companion `fastino/gliner2-privacy-filter-PII-multi` (published ONNX; VERIFIED).

**Re-export list.**
```rust
pub use polaris_system_one_schemas as schemas;     // L3 data: questions, thresholds, fail modes, calibration records
pub use polaris_system_one_router as router;       // L3: intent/effort/tools/files/sandbox routing
pub use polaris_system_one_verifier as verifier;   // L3: safety gate, preflight, postflight, diff arbiter
pub use polaris_system_one_watchdog as watchdog;   // L3: drift, loops, role boundary, fatigue
pub use polaris_system_one_client as client;       // L2: kernel-side batching + cache + timeouts (tonic over named pipe)
// polaris_system_one_engine and polaris_system_one_nano are deliberately NOT re-exported:
// they are linked only by apps/polaris-worker (ADR-008).
```
**Resolved at merge (VERIFIED):** the facade depends only on the five kernel-side children; `engine` and
`nano` are linked solely by `apps/polaris-worker`, so the kernel process never links ONNX Runtime (D3).
`workspace.toml` was updated accordingly and `cargo tree -p polaris-kernel` contains neither crate
(nor `polaris-ingest`, which is likewise reached only through the ingest worker).

**The System-One typed API (sdk types, documented here).** Question types map onto the GLiNER2 `classify_text` schema (Python `gliner2` library: `AutoExtractor`, schema dict, `multi_label`, `cls_threshold`, yes/no prompts, ordinal labels "0".."10", several tasks per call; exact Rust surface of `gliner2-rs` 0.9.6 is "API to confirm at implementation").
```rust
pub struct SchemaId(String);           // e.g. "safety.command", "effort.task", "sift.chunk"
pub struct SchemaVersion(u32);
pub struct QuestionId(String);
pub struct OptionLabel(String);
pub struct Prob(f32);                  // TryFrom<f32>: finite and within 0.0..=1.0, else SchemaError

pub enum Question {
    Choice(ChoiceQuestion),            // pick one/many of N labels            -> classify_text, labels, multi_label
    Score(ScoreQuestion),              // ordinal 0..=10 (or 1..=5 sub-range)  -> ordinal labels "0".."10"
    Noul(NoulQuestion),                // yes/no with probability              -> yes/no prompt (exact Noul semantics: LIKELY = boolean+probability; confirm against TypeSafe description)
}
pub struct ChoiceQuestion { pub id: QuestionId, pub prompt: String, pub options: Vec<OptionLabel>, pub multi_label: bool }
pub struct ScoreQuestion  { pub id: QuestionId, pub prompt: String, pub lo: u8, pub hi: u8 }
pub struct NoulQuestion   { pub id: QuestionId, pub statement: String }

pub struct DecisionRequest { pub schema: SchemaId, pub version: SchemaVersion, pub input: DecisionInput, pub consumer: Consumer }
pub struct DecisionInput { pub text: String, pub context: Vec<String> }   // bounded by schema max_chars; truncation is explicit, never silent

pub enum Answer {
    Choice { id: QuestionId, ranked: Vec<(OptionLabel, Prob)>, selected: Vec<OptionLabel> },
    Score  { id: QuestionId, distribution: Vec<Prob>, expected: f32, argmax: u8 },
    Noul   { id: QuestionId, yes: Prob },
}
pub struct Decision { pub schema: SchemaId, pub version: SchemaVersion, pub answers: Vec<Answer>, pub provenance: Provenance }
pub struct Provenance { pub engine: EngineKind, pub checkpoint: Blake3Hex, pub cache: CacheOutcome, pub latency_us: u64, pub truncated: bool }
pub enum Verdict { Accept, Reject, Abstain }          // produced by schemas::apply_threshold, NOT by the engine

pub trait DecisionEngine: Send + Sync {                 // sdk registry trait (prompt.md section 6.2), generic bound, no dyn
    fn decide(&self, req: DecisionRequest) -> impl Future<Output = Result<Decision, DecisionError>> + Send;
}
```
The engine returns raw probabilities only. Thresholds are applied by `schemas`, so a threshold can be recalibrated without re-running inference or invalidating the cache.

**Schema and threshold calibration process (normative).**
1. Each schema ships as TOML: questions, `max_chars`, thresholds, `fail` policy, and a `[calibration]` table (`status = "uncalibrated" | "calibrated"`, `fixture_set`, `fixture_blake3`, `n`, `precision_target`, `observed_precision`, `observed_recall`, `ece`, `checkpoint_blake3`, `date`).
2. MVP ships every schema `uncalibrated` with conservative defaults (high bar for Accept on safety-relevant schemas, Abstain band wide). The chatlog thresholds (0.70 escalate, 0.65 drift, 0.90 race-accept, 0.99 safety) are starting hypotheses only.
3. Labelled fixtures (JSONL, provenance and licence per file) are split dev/test by content hash. Hostile and injection fixtures are generated at test time, not committed (plan: AV/secret-scanner safety).
4. Run the Python `gliner2` reference and the Rust engine on the same fixtures. Gate: top-1 parity >= 99% (spike S0-4) before any Rust-side calibration is trusted; otherwise calibrate against the supervised Python sidecar.
5. Pick the threshold on the dev split to hit the schema's precision target (per consumer: safety = high-recall on Reject, sifter = high-recall on Keep). Fit probability calibration (temperature scaling or isotonic) only if ECE on dev exceeds the schema's bound; record ECE.
6. Freeze on the test split; write the `[calibration]` record; a CI check fails if a schema is `calibrated` but its `fixture_blake3` or `checkpoint_blake3` no longer matches.
7. Re-calibrate on: checkpoint change, ONNX re-export or precision change (fp32 to fp16/int8), fixture growth over 25%, or a regression found by `/nv-verify`-style audits.
8. Unlabelled drift guard (post-MVP): log (hashed) decision margins and alert when the Abstain rate shifts materially.

**Fail-open / fail-closed matrix.** Rows 1-6 are the PLAN core (§0–§12); rows 7-11 are PROPOSED by this plan part and need approval.

| # | Consumer | On S1 timeout / error | Off synchronous path? | Deterministic fallback |
|---|---|---|---|---|
| 1 | Safety gate (command + diff) | **Closed** | No | DFA/AST rules; unknown = approval prompt |
| 2 | PII masking | **Closed** | No | DFA + Shannon entropy only; never send unmasked |
| 3 | Triage (file ranking) | Open | Yes | lexical/BM25 + tree-sitter rank |
| 4 | Tool pruning | Open | Yes | lexical rank, bounded top-k |
| 5 | Watchdog | Open (`Unchecked`, logged) | Yes | exact-repeat + token-pressure checks |
| 6 | Sifter / distiller | Open | Yes | lexical filter + token-budget truncation |
| 7 | Effort router | Open | Yes | default `medium` (2), no spawn advice |
| 8 | Postflight tautology check | **Closed** (ProofOfDone reports UNVERIFIED, never PASS) | No | deterministic tautology detectors |
| 9 | Diff arbiter | **Closed** (escalate to supervisor, no auto-merge) | No | orthogonal-AST-only merge |
| 10 | Sandbox auto-routing | Open | Yes | conservative tier (most isolated available) |
| 11 | ConTree env matcher | Open | Yes | exact repo+commit match, else manual |

**Batching and caching.** The client coalesces concurrent questions inside a short window (window and max batch are S0-4 outputs, not constants) and sends multi-task requests so one forward pass answers several schemas on the same text. Cache key = `xxh3-128(schema_id | schema_version | checkpoint_blake3 | normalized_input)` in `quick_cache` (L1), with single-flight de-duplication of identical in-flight requests. xxh3 is non-cryptographic, so schemas flagged `integrity = "strict"` (safety, PII, postflight) store a blake3 of the full input beside the entry and treat a mismatch as a miss; this blocks hash-collision cache poisoning. Timeouts, errors and Abstain-by-truncation are never cached. Cache scope follows `$storage-level(global|isolated)`. The sdk-level concurrency pool for tensor work (chatlog "SystemOneTensorSemaphore 2-4") is a hypothesis; the real value comes from S0-4 load tests.

**Memory budget on 16 GB RAM (Ryzen 7 2700, GTX 1650 4 GB, plan item 10).** Figures are targets, weights are VERIFIED where stated.

| Resident item | Size | Status |
|---|---|---|
| Decide weights fp32 | 1.95 GB | VERIFIED (safetensors size); ONNX fp32 about the same, UNMEASURED |
| Decide fp16 / int8 | about 1.0 GB / 0.5 GB | GUESSING (arithmetic); accuracy must pass the parity gate |
| Activations + ORT arena (batch 8, 256 tokens) | unknown | UNMEASURED; cap via `max_batch`/`max_seq_len` |
| PII-NER model | unknown | UNVERIFIED (size of the published ONNX not checked) |
| Needle 3 | 8-29 MB | VERIFIED (.cact size range) |
| WSL2 VM cap | about 6 GB | plan P0 `.wslconfig` |

Proposed worker caps: `s1` worker resident <= 3.0 GB (target), `ingest` worker <= 1.5 GB, kernel + TUI <= 1.0 GB, leaving the rest for Windows, Docker Desktop and WSL2. The sum is tight on 16 GB; hence lazy load, idle-TTL eviction (default 10 min, target), pressure eviction when `memory_pressure()` crosses a threshold, and force-evict before heavy builds via `polaris-models` leases. Eviction order (PROPOSED): ingest OCR models, Needle, PII-NER (DFA fallback is safe), Decide last. The GTX 1650 shares 4 GB VRAM with the desktop and with CUDA demo tasks: the CUDA EP is opt-in and always evicted before a GPU build. Chatlog "~700 MB single instance" is rejected (does not match the 1.95 GB file).

**API sketch.** See the typed API above; facade adds only `pub use` lines.

**Dependencies.** Internal: the seven children above. External: none (`ext = []` in workspace.toml).

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: kernel-side re-export set (no `engine`/`nano`), matrix-vs-schemas consistency check.
- [ ] Keep the kernel-side-only dependency set (no `engine`/`nano` edges) and re-check `cargo tree -p polaris-kernel -i ort` is empty once real deps land.
- [ ] Add a doc-test or integration test asserting every `Consumer` variant has a row in the matrix and a `fail` policy in the builtin schemas.
- [ ] Publish the calibration procedure and the memory budget as crate-level `//!` docs and keep them in sync with `schemas` and `engine` after S0-4.

**Tests.**

| Category | Cases |
|---|---|
| unit / doc | doc-tests compile the typed API; matrix consistency test (11 rows vs `Consumer` enum) |
| contract | facade re-exports exactly the seven children (`check-layers` + `check-members`) |
| spike | S0-4 fills the latency and memory columns above |

**Risks & open questions.**
- VERIFIED: no ONNX ships for Decide; the memory and latency columns are blocked on the S0-4 export.
- VERIFIED (skeleton): the facade no longer depends on `engine`/`nano`; re-verify once real dependencies exist.
- UNVERIFIED: exact TypeSafe meaning of Noul; PII model size; `gliner2-rs` classification API details (S0-4).
- GUESSING: that zero-shot Decide is accurate enough for goal-drift and tautology questions; calibration may show it is not, in which case those consumers stay deterministic-only.

**Chatlog sources.** Msgs 1-3 (dual-engine, S1 duties, expanded S1 scope), 5 (verifier), 11 (security sieve, arbiter), 29 (single shared substrate, watchdog), 35 (S1+Needle+DFA token economy). Corrected: latencies, 340M/700 MB sizes, `todo!()`-style stubs, "confidence < 0.99 blocks" as an uncalibrated constant.

---

### `polaris-system-one-engine` — GLiNER2 inference on ONNX Runtime, linked only by the worker

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/engine` | L2 | lib | `polaris-system-one` | P4 |

**Purpose.** Loads the Decide checkpoint (and the PII-NER checkpoint) through `gliner2-rs` on ONNX Runtime with `load-dynamic`, runs batched multi-task classification/extraction, and returns raw probabilities. Because no ONNX ships for Decide, this crate also owns the export pipeline definition, the parity gate against the Python `gliner2` reference (spike S0-4) and the supervised Python-sidecar fallback. It never decides thresholds and never touches the network except to fetch weights via `hf-hub`.

**API sketch.**
```rust
pub struct CheckpointRef { pub repo: String, pub revision: String }            // e.g. fastino/GLiNER2.5-Decide @ pinned commit
pub enum Precision { Fp32, Fp16, Int8Dynamic }
pub enum ExecProvider { Cpu { intra_threads: u16 }, Cuda { device: u32, vram_cap_mib: u32 } }
pub struct ModelManifest { pub checkpoint: CheckpointRef, pub files: Vec<(String, Blake3Hex)>, pub precision: Precision, pub exporter: String, pub ort_api_min: u32 }
pub struct EngineConfig { pub manifest: ModelManifest, pub provider: ExecProvider, pub max_batch: u16, pub max_seq_len: u32, pub ort_dylib: PathBuf }

pub struct ClassifyJob { pub text: String, pub tasks: Vec<ClassifyTask> }     // several tasks per forward pass
pub struct ClassifyTask { pub id: QuestionId, pub labels: Vec<OptionLabel>, pub multi_label: bool }
pub struct ClassifyOutput { pub per_task: Vec<(QuestionId, Vec<(OptionLabel, Prob)>)>, pub truncated: bool }
pub struct EntityJob { pub text: String, pub labels: Vec<String> }
pub struct EntitySpan { pub label: String, pub start: usize, pub end: usize, pub prob: Prob }

pub struct GlinerEngine { /* ort session(s), tokenizer, manifest */ }
impl GlinerEngine {
    pub fn load(cfg: &EngineConfig) -> Result<Self, EngineError>;
    pub fn classify(&self, jobs: &[ClassifyJob]) -> Result<Vec<ClassifyOutput>, EngineError>;
    pub fn extract(&self, jobs: &[EntityJob]) -> Result<Vec<Vec<EntitySpan>>, EngineError>;
    pub fn resident_bytes(&self) -> ByteCount;
    pub fn unload(self) -> Result<ByteCount, EngineError>;
}
pub struct SidecarEngine { /* supervised python gliner2, JSON over stdio or pipe; transport to confirm */ }
pub enum EngineBackend { Native(GlinerEngine), PythonSidecar(SidecarEngine) }  // enum dispatch per composition-root ADR
pub struct ParityReport { pub n: usize, pub top1_agree: f64, pub max_abs_prob_diff: f32, pub passed: bool }
pub fn parity_check(native: &GlinerEngine, reference_jsonl: &Path, min_top1: f64) -> Result<ParityReport, EngineError>;
pub enum EngineError { OrtLoad(String), OrtVersionTooOld { found: String, need: String }, ModelFileMissing(PathBuf), Tokenize(String), Inference(String), ParityGate(ParityReport), SidecarDied, OutOfMemory }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`. External (workspace.toml): `gliner2-rs = 0.9.6`, `ort = =2.0.0-rc.13` (features std, ndarray, tracing, load-dynamic, api-23; pinned because oar-ocr-core requires it), `tokenizers = 0.23.2`, `hf-hub = 1.0.0`, `ndarray = 0.17.2`. Proposed additions: `thiserror = 2.0.21`, `serde = 1.0.229`, `serde_json = 1.0.154`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: model loading + ORT discovery, multi-task classify, entity extraction, parity gate, sidecar fallback.
- [ ] Spike S0-4 step 1: export Decide to ONNX fragments (encoder, heads) from the Python checkpoint; record exporter script, tool versions and output blake3 in `ModelManifest`.
- [ ] Spike S0-4 step 2: build the Python reference run (uv is missing on this machine; install it in P0) and produce `reference.jsonl` for the calibration fixtures.
- [ ] Implement ORT loading: `ORT_DYLIB_PATH`, DLL beside the exe (never System32), reject ORT < 1.23 with `OrtVersionTooOld`.
- [ ] Implement CPU path via `gliner2-rs` classification with probabilities; confirm the crate really accepts span checkpoints and our export (API to confirm at implementation).
- [ ] Implement `parity_check` and enforce top-1 >= 99% on the fixture set; below that, `EngineBackend` selects the sidecar.
- [ ] Measure fp32 / fp16 / int8-dynamic on CPU and CUDA EP (GTX 1650, sm_75): p50/p95 for batch 1/8/32 at 128/256/512 tokens; record in the manifest and in `PHASES_STATUS.md`.
- [ ] Implement the PII-NER path using the published ONNX of `gliner2-privacy-filter-PII-multi` (BLOCKED-ON-ACCESS: Hugging Face download reachability and licence check for both checkpoints; licences are UNVERIFIED).
- [ ] Implement `resident_bytes` accounting and `unload` that releases the ORT session and arena; test RSS returns near baseline.
- [ ] Implement the supervised Python sidecar (JSON frames; restart policy delegated to the kernel supervisor) as the fallback backend.
- [ ] Add batch-composition invariance checks (result for an item must not depend on its neighbours in the batch; padding mask correctness).
- [ ] Wire the worker-side gRPC adapter in `apps/polaris-worker` (not in this crate); engine exposes only plain Rust calls.

**Tests.**

| Category | Cases |
|---|---|
| unit | label-to-prompt formatting; truncation flag set when input exceeds `max_seq_len`; manifest hash mismatch -> `ModelFileMissing`; ORT version gate |
| proptest | per-task probabilities within [0,1] and (single-label) sum to about 1; batch-composition invariance within tolerance; unicode/empty/very long text never panics |
| golden-parity | native vs Python `gliner2` on the calibration fixtures: top-1 >= 99% (gate), `max_abs_prob_diff` recorded; separate run per precision/EP |
| spike | S0-4: export feasibility, parity, CPU vs CUDA EP latency and RSS |
| load | sustained batches (1/8/32) for 10 min: throughput, p99, RSS slope |
| stress | load/unload x 100 loop (RSS leak check); ORT DLL missing; OOM at large batch returns `OutOfMemory`, not abort |
| fuzz | tokenizer + text preprocessing with arbitrary bytes-as-UTF-8 and lone surrogate edge cases (WSL2 nightly) |
| security | prompt-injection corpus: long padding to push the real question out of the window (truncation must keep the instruction side), confusable unicode in labels |
| fixtures | `tests/fixtures/s1/{decide,pii}/*.jsonl` with provenance + licence; hostile inputs generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Engine model loading | function | `GlinerEngine::load` | `crates/polaris-system-one/crates/engine/src/load.rs` | `load_valid_manifest_returns_engine`, `load_missing_file_returns_model_file_missing`, `load_hash_mismatch_rejected`, `load_ort_below_1_23_rejected`, `load_dylib_beside_exe_preferred_over_path` |
| Multi-task classify | method | `GlinerEngine::classify` | `crates/polaris-system-one/crates/engine/src/classify.rs` | `classify_returns_probabilities_in_unit_range`, `classify_multi_task_single_pass_same_as_separate`, `classify_batch_order_preserved`, `classify_truncation_flagged`, `classify_empty_text_returns_error_not_panic` |
| Parity gate | function | `parity_check` | `crates/polaris-system-one/crates/engine/src/parity.rs` | `parity_passes_at_or_above_threshold`, `parity_fails_below_threshold_reports_counts`, `parity_ignores_ties_within_epsilon`, `parity_reference_file_malformed_is_error` |
| Backend selection | enum | `EngineBackend` | `crates/polaris-system-one/crates/engine/src/backend.rs` | `native_selected_when_parity_passed`, `sidecar_selected_when_parity_failed`, `sidecar_death_surfaces_sidecar_died`, `backend_switch_preserves_cache_key_checkpoint` |

**Risks & open questions.**
- VERIFIED: no ONNX for Decide; self-export is required; `gliner2-rs` needs ORT >= 1.23 and is the plan's Rust path.
- UNVERIFIED: whether the exported graph (DeBERTa-v3 disentangled attention) converts cleanly to ONNX and matches `gliner2-rs` input conventions. Covered by S0-4; fallback = supervised Python sidecar (needs `uv`, currently missing).
- VERIFIED: ort is pinned `=2.0.0-rc.13` workspace-wide; if `gliner2-rs` 0.9.6 needs a different rc the probe would have caught it (probe-verified per workspace.toml note), but runtime behaviour is UNTESTED.
- LIKELY: fp16/int8 will degrade accuracy on a DeBERTa-large span model; only the parity gate can say.
- GUESSING: CUDA EP on a 4 GB GTX 1650 can host Decide fp16 plus activations; needs S0-4 measurement and CUDA/cuDNN DLL availability.

**Chatlog sources.** Msg 1 (sections 3.1, 6, 8.1), 3 (batch triage), 29 (single instance multiplexed by schema), 21 (GLiNER as zero-shot NER for PII), 17 (tensor semaphore). Corrected: "<15-45 ms", "340M / ~700 MB", Candle path (not in the pinned stack), `todo!()` body.

---

### `polaris-system-one-client` — kernel-side S1 client: batching, cache, timeouts, worker IPC

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/client` | L2 | lib | `polaris-system-one` | P4 |

**Purpose.** Implements the sdk `DecisionEngine` against the `polaris-worker --mode s1` process over tonic on a Windows named pipe (uses `polaris-proto` and tonic, never `polaris-rpc`). Owns request coalescing, the xxh3 + `quick_cache` result cache, per-consumer deadlines and the deterministic fail-open/fail-closed outcome, so no caller hand-rolls timeout behaviour.

**API sketch.**
```rust
pub enum Consumer { SafetyGate, PiiMask, Triage, ToolPrune, Watchdog, Sifter, EffortRouter, Postflight, DiffArbiter, SandboxRoute, EnvMatch, NvidiaProfile, SkillSelect }
pub enum FailMode { Open, Closed }
pub struct ConsumerPolicy { pub consumer: Consumer, pub deadline: Duration, pub on_timeout: FailMode }
pub struct ClientConfig { pub batch_window: Duration, pub max_batch: u16, pub cache_capacity: usize, pub queue_bound: usize, pub policies: Vec<ConsumerPolicy> }

pub trait WorkerTransport: Send + Sync {
    fn decide_batch(&self, req: proto::DecideBatchRequest) -> impl Future<Output = Result<proto::DecideBatchReply, TransportError>> + Send;
    fn health(&self) -> impl Future<Output = Result<proto::WorkerHealth, TransportError>> + Send;
}
pub struct NamedPipeTransport { /* custom tonic connector, token auth metadata */ }
pub struct InProcessTransport<E> { /* tests: wraps an engine-like fake that is NOT a stub: real fixture-backed answers */ }

pub enum Outcome<T> { Answered(T), FailedOpen { reason: FailReason }, FailedClosed { reason: FailReason } }
pub struct S1Client<T: WorkerTransport, C: SchemaCatalog> { /* batcher, cache, transport, catalog */ }
impl<T: WorkerTransport, C: SchemaCatalog> DecisionEngine for S1Client<T, C> { /* decide */ }
impl<T: WorkerTransport, C: SchemaCatalog> S1Client<T, C> {
    pub fn new(cfg: ClientConfig, transport: T, catalog: C) -> Result<Self, ClientError>;
    pub async fn decide_with_policy(&self, req: DecisionRequest) -> Outcome<Decision>;
    pub fn cache_stats(&self) -> CacheStats;
    pub fn purge_cache(&self, scope: CacheScope);
}
pub enum ClientError { Config(String), Transport(TransportError), QueueFull, SchemaUnknown(SchemaId), WorkerUnhealthy }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`, `polaris-storage-cache`. External (workspace.toml): `tonic = 0.14.6`, `tokio = 1.54.0`, `quick_cache = 0.7.0`, `xxhash-rust = 0.8.19`. Proposed additions: `blake3 = 1.8.7` (strict-integrity keys), `thiserror = 2.0.21`, `tokio-util = 0.7.20` (cancellation), `tower = 0.5.3` (named-pipe connector, API to confirm).

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: request batcher, cache key + strict integrity, deadline/fail-mode resolution, named-pipe transport.
- [ ] Define `worker.proto` `DecideBatch` messages with `polaris-proto` owners; include `protocol_version` and `checkpoint_blake3` in replies.
- [ ] Implement the batcher: time-window + size coalescing, per-request response routing, bounded queue with `QueueFull` backpressure.
- [ ] Implement cache key construction and single-flight; strict-integrity blake3 check on hit for safety/PII/postflight schemas.
- [ ] Implement `Outcome` resolution so each `Consumer` maps to its `FailMode` exactly as the facade matrix states; one place, table-driven.
- [ ] Implement `NamedPipeTransport`: custom tonic connector, pre-created next pipe instance on the server side (worker), token auth metadata; document the DACL concern (default allows other local users).
- [ ] Add circuit-breaker cooperation: after N consecutive transport failures return `FailedOpen/Closed` immediately (no queueing) until the supervisor reports the worker healthy.
- [ ] Add cache stats + purge wired to `/cache stats|purge` via telemetry events.
- [ ] Add a non-stub in-process transport backed by recorded real engine answers (fixtures labelled synthetic until re-recorded) for kernel tests.
- [ ] Load-test coalescing under 50-subagent bursts (spike `polaris-spike-s1`); tune window and max batch from S0-4 numbers.

**Tests.**

| Category | Cases |
|---|---|
| unit | policy resolution table for every `Consumer`; timeouts not cached; schema version in key invalidates old entries |
| proptest | every request receives exactly its own answer under random interleavings; cache key stable under input normalization, different under any schema/version/checkpoint change |
| loom | batcher state machine (enqueue/flush/shutdown) modelled without tokio: no lost or duplicated request |
| contract | `worker.proto` request/reply against a fake worker over a real named pipe; protocol_version mismatch rejected |
| load | cached decisions per second and p99; mixed hit/miss at 1k, 10k concurrent callers; record with hdrhistogram |
| scalability | 1 -> 32 frames each submitting watchdog/triage questions |
| stress | worker killed mid-batch: all in-flight requests resolve to the policy outcome within deadline; worker crash loop; queue saturation |
| spike | `polaris-spike-s1` burst: 50 concurrent subagent questions; compare batching on/off |
| security | cache-poison attempt via crafted colliding inputs on a strict schema (must miss); pipe connection without token rejected |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| S1 request batching | struct | `Batcher` | `crates/polaris-system-one/crates/client/src/batch.rs` | `batcher_flushes_on_window_expiry`, `batcher_flushes_on_max_batch`, `batcher_routes_responses_to_callers`, `batcher_rejects_when_queue_full`, `batcher_cancelled_caller_does_not_stall_batch` |
| S1 result cache | struct | `DecisionCache` | `crates/polaris-system-one/crates/client/src/cache.rs` | `cache_hit_returns_same_decision`, `cache_miss_on_schema_version_bump`, `cache_never_stores_timeouts`, `cache_strict_schema_rejects_hash_collision`, `cache_single_flight_dedupes_concurrent_requests` |
| Consumer fail policy | function | `resolve_outcome` | `crates/polaris-system-one/crates/client/src/policy.rs` | `safety_gate_timeout_is_failed_closed`, `pii_timeout_is_failed_closed`, `triage_timeout_is_failed_open`, `watchdog_timeout_is_failed_open_unchecked`, `every_consumer_has_a_policy` |
| Named-pipe worker transport | async_function | `NamedPipeTransport::decide_batch` | `crates/polaris-system-one/crates/client/src/pipe.rs` | `pipe_roundtrip_returns_reply`, `pipe_wrong_token_rejected`, `pipe_protocol_version_mismatch_rejected`, `pipe_worker_gone_returns_transport_error` |

**Risks & open questions.**
- LIKELY: tonic 0.14 over Windows named pipes needs a custom connector and a server-side pre-created next instance (plan); exact tower/hyper-util wiring is "API to confirm at implementation".
- VERIFIED: failure matrix rows 1-6 are in the PLAN core (§0–§12); rows 7-11 are PROPOSED.
- UNVERIFIED: that batching helps at all on CPU for a DeBERTa-large encoder (padding waste can dominate); S0-4 decides whether `max_batch` is 1.
- GUESSING: xxh3-128 collision risk is negligible for benign inputs; the strict check exists for adversarial inputs.

**Chatlog sources.** Msgs 1 and 3 (S1 duties), 11 (batching/coalescing idea, applied here to S1; S2 batching belongs elsewhere), 17 (permit pool), 29 (one multiplexed S1 instance), 14-15 (xxh3 for fast compare, here used as cache key). Corrected: xxh3 used only as a non-cryptographic key with integrity check on strict schemas.

---

### `polaris-system-one-schemas` — decision schemas as versioned data with calibrated thresholds and fail modes

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/schemas` | L3 | lib | `polaris-system-one` | P4 |

**Purpose.** Holds every System-One schema as TOML data (questions, input limits, thresholds, fail policy, calibration record) and implements the sdk `SchemaCatalog` trait. It is the only place thresholds live; it applies them to raw `Decision` probabilities to produce a `Verdict`. It performs no inference.

**API sketch.**
```rust
pub enum SchemaKind { Intent, Effort, ToolRelevance, FileRelevance, ChunkRelevance, SnippetRelevance, SafetyCommand, DiffTampering, SecretSemantic, Tautology, Congruence, LoopSemantic, RoleBoundary, SandboxRoute, EnvMatch, Pii, SecurityArbitration, NvidiaStack, SkillRelevance, MergeCongruence }
pub struct Thresholds { pub accept: Prob, pub reject: Prob, pub abstain_band: (Prob, Prob) }   // validated: reject <= abstain.lo <= abstain.hi <= accept
pub enum CalibrationStatus { Uncalibrated, Calibrated }
pub struct CalibrationRecord { pub status: CalibrationStatus, pub fixture_set: String, pub fixture_blake3: Option<Blake3Hex>, pub n: u32, pub precision_target: Option<f32>, pub observed_precision: Option<f32>, pub observed_recall: Option<f32>, pub ece: Option<f32>, pub checkpoint_blake3: Option<Blake3Hex>, pub date: Rfc3339 }
pub struct FailPolicy { pub consumer: Consumer, pub on_timeout: FailMode, pub deadline_ms: u32, pub integrity: Integrity }   // Integrity::{Normal, Strict}
pub struct Schema { pub id: SchemaId, pub version: SchemaVersion, pub kind: SchemaKind, pub max_chars: u32, pub questions: Vec<Question>, pub thresholds: Thresholds, pub fail: FailPolicy, pub calibration: CalibrationRecord }
pub struct SchemaSet { /* indexed by SchemaId + version */ }
impl SchemaSet {
    pub fn load_builtin() -> Result<SchemaSet, SchemaError>;
    pub fn from_toml(src: &str) -> Result<SchemaSet, SchemaError>;
    pub fn get(&self, id: &SchemaId) -> Result<&Schema, SchemaError>;
    pub fn validate(&self) -> Result<ValidationReport, SchemaError>;
}
pub fn apply_threshold(schema: &Schema, decision: &Decision) -> Result<Vec<(QuestionId, Verdict)>, SchemaError>;
impl SchemaCatalog for SchemaSet { /* resolve questions, fail policy, verdicts for L2/L3 consumers */ }
pub enum SchemaError { Parse(String), DuplicateId(SchemaId), UnknownQuestion(QuestionId), ThresholdOrder, CalibrationStale { id: SchemaId }, ProbOutOfRange(f32) }
```
Builtin schema ids (PROPOSED): `route.intent`, `route.effort`, `route.tool`, `route.file`, `route.sandbox`, `route.env`, `sift.chunk`, `sift.snippet`, `safety.command`, `safety.diff_tamper`, `safety.secret_semantic`, `safety.tautology`, `watch.congruence`, `watch.loop`, `watch.role`, `sec.arbitrate`, `pii.ner`, `nv.stack`, `skill.relevance`, `merge.congruence`.

**Dependencies.** Internal: `polaris-sdk`. External (workspace.toml): `serde = 1.0.229`, `toml = 1.1.8`. Proposed additions: `thiserror = 2.0.21`, `blake3 = 1.8.7` (fixture/checkpoint hash fields are hex strings validated here; hashing itself is done by xtask).

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: schema TOML parsing, threshold validation, threshold application, calibration staleness check, builtin coverage.
- [ ] Define the TOML grammar and a `schemas/*.toml` directory embedded with `include_str!`; one file per schema id.
- [ ] Implement `Prob`-validated thresholds with ordering invariants and exhaustive error variants.
- [ ] Implement `apply_threshold` for Choice, Score (expected vs argmax rule per schema) and Noul.
- [ ] Ship all builtin schemas as `uncalibrated` with conservative defaults; safety-relevant ones set `integrity = "strict"` and `on_timeout = "closed"`.
- [ ] Implement `SchemaCatalog` and unit-test that every `Consumer` has exactly one default fail policy.
- [ ] Implement the staleness check (`calibrated` requires matching `fixture_blake3` / `checkpoint_blake3`); xtask supplies the hashes.
- [ ] Add the calibration runner as an `xtask` subcommand contract (inputs: fixtures + engine output JSONL; outputs: TOML patch) without embedding inference here.
- [ ] Build the first labelled fixture sets for effort (50+ tasks), safety.command (benign vs destructive), tool relevance (MCP tool lists), chunk relevance; record provenance and licence.
- [ ] Re-run calibration after S0-4 once a trustworthy engine exists (BLOCKED-ON-ACCESS: S0-4 export result, HF checkpoint access).

**Tests.**

| Category | Cases |
|---|---|
| unit | duplicate ids rejected; malformed TOML gives a typed error; every builtin loads and validates |
| proptest | threshold monotonicity: raising `accept` never increases the number of Accept verdicts; verdicts total over any valid probability vector |
| kani | `apply_threshold` on bounded vectors: no panic, no out-of-range index, no NaN reaching a comparison |
| golden | insta snapshot of the loaded builtin set (guards accidental threshold edits) |
| contract | every `Consumer` variant has a schema with a fail policy; safety/PII/postflight/arbiter are `closed` + `strict` |
| fixtures | labelled calibration sets (JSONL, provenance + licence); injection negatives generated at test time |
| security | schema TOML from a repo (`.polaris/`) cannot lower a built-in `closed` policy to `open` (override only tightens) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Schema loading | function | `SchemaSet::from_toml` | `crates/polaris-system-one/crates/schemas/src/load.rs` | `load_builtin_set_succeeds`, `load_duplicate_schema_id_rejected`, `load_unknown_question_ref_rejected`, `load_malformed_toml_returns_parse_error`, `load_max_chars_zero_rejected` |
| Threshold application | function | `apply_threshold` | `crates/polaris-system-one/crates/schemas/src/threshold.rs` | `noul_above_accept_is_accept`, `noul_in_band_is_abstain`, `choice_multi_label_selects_all_above_accept`, `score_uses_configured_rule`, `raising_accept_never_adds_accepts` |
| Calibration staleness | function | `SchemaSet::validate` | `crates/polaris-system-one/crates/schemas/src/validate.rs` | `calibrated_with_matching_hashes_ok`, `calibrated_with_stale_fixture_hash_rejected`, `uncalibrated_needs_no_hashes`, `threshold_order_violation_rejected` |
| Fail policy defaults | module | `policy` | `crates/polaris-system-one/crates/schemas/src/policy.rs` | `every_consumer_has_policy`, `safety_and_pii_are_closed`, `repo_override_cannot_loosen_closed`, `strict_integrity_set_for_safety_schemas` |

**Risks & open questions.**
- VERIFIED: GLiNER2 supports ordinal labels "0".."10" and yes/no prompts, so Score and Noul are expressible.
- UNVERIFIED: whether Decide's probabilities are calibrated at all; ECE may force isotonic mapping, which then belongs in this crate's data.
- LIKELY: two parallel threshold sources (engine `cls_threshold` vs ours) will confuse; engine always runs at threshold 0 and returns full distributions.
- UNVERIFIED: size and label quality of the fixture sets achievable before the deadline; MVP stays `uncalibrated` and says so in `/status`.

**Chatlog sources.** Msgs 1 and 3 (`DecisionSchema`, `Choice/Boolean/Score` enums, example questions such as `{"relevant_to_goal": Boolean, "information_density": Score}`), 5 (`{"is_tautological", "tests_actual_business_logic", "coverage_confidence"}`), 11 (arbiter and security questions), 29 (alignment question). Corrected: constants like 0.99/0.70/0.65 become uncalibrated defaults with a calibration record; `untagged` serde enum ambiguity replaced by explicit tagged answers.

---

### `polaris-system-one-router` — S1 routing: intent, effort 1-5, tool pruning, file triage, sandbox tier, env ranking

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/router` | L3 | lib | `polaris-system-one` | P4 |

**Purpose.** Turns user input, steps and candidate lists into routing advice using the S1 `DecisionEngine`: trivial-vs-S2, effort level, spawn advice, top-k tool pruning, top-k file triage, `$sandbox(auto)` tier choice and ConTree environment ranking. It returns an `Effort` (1-5); mapping effort to a Nemotron model (and `$subagent-models` overrides) is done by `polaris-models` at the composition root, since router and models are L3 siblings. Candidate generation (ripgrep, tree-sitter) is the caller's job; the router only scores.

**API sketch.**
```rust
pub struct RouteInput { pub text: String, pub root_goal: Option<String>, pub repo_facts: RepoFacts, pub directives: DirectiveSnapshot }
pub enum RoutePath { DirectTool { tool: ToolId }, S2, AskClarification }
pub struct SpawnAdvice { pub parallel_branches: bool, pub max_depth: u8 }
pub struct Route { pub path: RoutePath, pub effort: Effort, pub needs_vision: bool, pub spawn: SpawnAdvice, pub confidence: Prob, pub source: RouteSource }   // RouteSource::{S1, DeterministicFallback}

pub struct ToolCandidate { pub id: ToolId, pub name: String, pub summary: String }       // summary truncated; attacker-controlled
pub struct FileCandidate { pub path: PathBuf, pub signature_text: String, pub lexical_score: f32 }
pub struct Ranked<T> { pub item: T, pub score: Prob, pub reason: RankReason }
pub struct TopK(u16);
pub struct SandboxFacts { pub runs_server: bool, pub gui_or_browser: bool, pub untrusted_code: bool, pub bench_batch: bool, pub nested_kvm_ok: bool }
pub enum SandboxTier { E0, E1, E2, E3, E4, E1w }

pub struct Router<D: DecisionEngine, C: SchemaCatalog> { /* engine, catalog, deterministic fallbacks */ }
impl<D: DecisionEngine, C: SchemaCatalog> Router<D, C> {
    pub fn new(engine: D, catalog: C) -> Self;
    pub async fn route(&self, input: &RouteInput) -> Result<Route, RouterError>;
    pub async fn prune_tools(&self, step: &StepText, tools: &[ToolCandidate], k: TopK) -> Result<Vec<Ranked<ToolId>>, RouterError>;
    pub async fn rank_files(&self, query: &str, files: &[FileCandidate], k: TopK) -> Result<Vec<Ranked<PathBuf>>, RouterError>;
    pub async fn route_sandbox(&self, facts: &SandboxFacts, directive: SandboxDirective) -> Result<SandboxTier, RouterError>;
    pub async fn rank_env_images(&self, manifest: &RepoManifestFacts, images: &[EnvImageCandidate]) -> Result<Vec<Ranked<ImageId>>, RouterError>;
}
pub enum RouterError { Engine(DecisionError), Schema(SchemaError), EmptyCandidates, TopKZero }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-system-one-schemas`. External: none in workspace.toml (`ext = []`). Proposed: `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: `route`, `prune_tools`, `rank_files`, `route_sandbox`, `rank_env_images`.
- [ ] Implement deterministic fallbacks first (lexical scoring, effort default, conservative sandbox tier) so fail-open behaviour exists before S1 does.
- [ ] Implement `route`: Choice over {trivial-direct, S2} + Score effort 1-5 + Noul parallelizable/needs-vision; unify into one multi-task request.
- [ ] Implement `prune_tools` as two-stage: lexical prefilter to a bounded candidate set, then S1 Score; hard caps on candidate count and per-summary length.
- [ ] Implement `rank_files` as two-stage (tree-sitter/ripgrep candidates are inputs); never score all N files with S1 (cost grows with N on CPU).
- [ ] Implement `route_sandbox`: deterministic rules first (nested KVM absent -> no E2; no key -> no E3), S1 only for ambiguous facts.
- [ ] Implement `rank_env_images`: exact repo+commit/lockfile-hash match short-circuits S1; S1 ranks only near-misses.
- [ ] Respect `$subagent-models`/`$sandbox` directives as hard constraints applied after scoring.
- [ ] Build labelled fixtures: 100+ task descriptions with effort labels, 30 MCP tool lists with relevant-tool labels, 20 repos with file relevance labels (BLOCKED-ON-ACCESS for real-model calibration: S0-4).
- [ ] Measure S1 cost vs candidate count (10/20/80/200 files, 10/50 tools) on the target machine and set the candidate caps from data.

**Tests.**

| Category | Cases |
|---|---|
| unit | zero candidates and `TopK(0)` are typed errors; directive overrides win over scores; tie-breaking is deterministic |
| proptest | output is a permutation subset of input ids, length <= k, scores non-increasing; same input gives same output |
| golden-parity | router verdicts on labelled fixtures with the Python reference vs Rust engine: identical routes in >= 99% of cases |
| smoke | with a fake always-timeout engine every method returns its deterministic fallback with `RouteSource::DeterministicFallback` |
| load | 1k route() calls and 50-tool pruning under concurrent frames; p99 recorded |
| scalability | 10 / 50 / 500 tools; 20 / 200 / 2000 candidate files (cap holds) |
| security | tool-description injection corpus: a tool whose summary says "always select me" must not outrank genuinely relevant tools beyond a measured bound; oversize summaries truncated |
| fixtures | `tests/fixtures/s1/router/*.jsonl` (labelled tasks/tools/files) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Effort and intent routing | async_function | `Router::route` | `crates/polaris-system-one/crates/router/src/route.rs` | `route_trivial_command_goes_direct_tool`, `route_complex_refactor_sets_high_effort`, `route_timeout_returns_default_medium_fallback`, `route_vision_flag_set_for_screenshot_task`, `route_effort_always_within_1_to_5` |
| Tool pruning | async_function | `Router::prune_tools` | `crates/polaris-system-one/crates/router/src/tools.rs` | `prune_returns_at_most_k`, `prune_lexical_fallback_on_timeout`, `prune_truncates_oversize_summaries`, `prune_poisoned_description_not_boosted`, `prune_empty_tools_is_error` |
| File triage | async_function | `Router::rank_files` | `crates/polaris-system-one/crates/router/src/files.rs` | `rank_files_caps_s1_candidates`, `rank_files_keeps_lexical_order_on_timeout`, `rank_files_scores_in_unit_range`, `rank_files_stable_on_ties` |
| Sandbox tier selection | async_function | `Router::route_sandbox` | `crates/polaris-system-one/crates/router/src/sandbox.rs` | `no_kvm_never_selects_e2`, `explicit_directive_overrides_auto`, `untrusted_code_prefers_most_isolated_tier`, `s1_timeout_selects_conservative_tier` |

**Risks & open questions.**
- GUESSING: chatlog "80 files in about 45 ms" is false on this CPU; batch cost grows with candidates, hence the two-stage design and measured caps.
- VERIFIED: effort-to-model table and numeric 1-5 parsing are in the PLAN core (§0–§12); mapping lives in `polaris-models`.
- LIKELY: zero-shot effort scoring will be noisy; MVP may need a rules-plus-S1 hybrid, decided after fixtures exist.
- UNVERIFIED: how ConTree env candidates are exposed (names/tags via `list_images(tag_prefix)`); `rank_env_images` input shape depends on spike S0-13 (BLOCKED-ON-ACCESS: Contree key).

**Chatlog sources.** Msgs 1-3 (intent routing, zero-token tool selection top-3, effort scoring 1-5, file triage, `$subagent-models` as complexity map), 11 (ConTree resolver), 25 (`$sandbox(auto)` rules). Corrected: "~18 ms/45 ms/15 ms" latency claims; ConTree resolver placed here as ranking only, catalog access stays in nebius sandboxes crate; `sled` catalog index replaced by the storage layer.

---

### `polaris-system-one-verifier` — S1 verification: safety gate, preflight, postflight, diff arbiter

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/verifier` | L3 | lib | `polaris-system-one` | P4 |

**Purpose.** Four checks, each deterministic-first with S1 only for ambiguity: (1) safety gate for shell commands, (2) preflight on a proposed diff (test tampering, secret leaks, sandbox-escape patterns), (3) postflight on a green run (tautological tests, diagnostics ranking), (4) arbiter for concurrent diffs to one file. Design rule: S1 may add a block or ask but can never override a DFA/AST block. Safety, PII-adjacent and postflight checks fail closed.

**API sketch.**
```rust
pub enum TamperKind { AssertionRemoved, AssertionWeakened, IgnoreAttributeAdded, SkipMarkerAdded, TestDeleted, CiConfigEdited, FeatureGateDisablesTest }
pub enum Finding {
    TestTampering { kind: TamperKind, loc: SourceSpan },
    SecretLeak { kind: SecretKind, loc: SourceSpan, entropy_bits: f32 },
    DestructiveCommand { pattern: PatternId },
    SandboxEscapeAttempt { pattern: PatternId },
    CudaSafety { rule: RuleId, loc: SourceSpan },
    Tautology { kind: TautologyKind, test: TestId },
}
pub enum GateVerdict { Allow, AskUser { reason: String }, Block { findings: Vec<Finding> } }
pub struct PreflightReport { pub verdict: GateVerdict, pub findings: Vec<Finding>, pub s1_used: bool, pub degraded: Option<DegradedReason> }
pub struct PostflightReport { pub verdict: ProofVerdict, pub findings: Vec<Finding>, pub ranked_diagnostics: Vec<Ranked<DiagnosticId>> }  // ProofVerdict::{Pass, Fail, Unverified}
pub enum MergeOutcome { CompatibleAutoMerge { merged: AstMergeResult }, SemanticConflict { symbols: Vec<SymbolPath>, reason: String }, RequiresSupervisorAttention }

pub struct Verifier<D: DecisionEngine, C: SchemaCatalog, S: Scanner> { /* engine, catalog, DFA scanner, code-intel handle */ }
impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    pub async fn check_command(&self, cmd: &CommandFacts) -> Result<GateVerdict, VerifierError>;
    pub async fn preflight(&self, diff: &ParsedDiff) -> Result<PreflightReport, VerifierError>;
    pub async fn postflight(&self, run: &TestRunFacts, diff: &ParsedDiff) -> Result<PostflightReport, VerifierError>;
    pub async fn arbitrate(&self, base: &SourceFile, a: &ParsedDiff, b: &ParsedDiff) -> Result<MergeOutcome, VerifierError>;
}
pub enum VerifierError { Parse(String), Scanner(String), Engine(DecisionError), Schema(SchemaError), UnsupportedLanguage(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-system-one-schemas`, `polaris-security-automata`, `polaris-code-intel`. External: none in workspace.toml. Proposed: `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: command gate, preflight tamper detection, secret scan, postflight tautology, diff arbiter.
- [ ] Implement AST-based test-tampering detection via `code-intel` (removed/weakened asserts, `#[ignore]`, `pytest.mark.skip`, `unittest.skip`, `@Disabled`, deleted test fns); replace the chatlog's substring check (`"-    assert"`), which is trivially bypassed and noisy.
- [ ] Implement secret detection with DFA signatures + Shannon entropy from `security-automata`; findings never echo the secret value (redacted evidence).
- [ ] Implement the command gate: shell tokenization, destructive-pattern DFA (`rm -rf`, unquoted variable expansion into rm, `/proc/kcore`, `curl|sh`, hidden-file edits), S1 only for the residual "unclear intent" class; unknown -> `AskUser`.
- [ ] Implement deterministic tautology detectors (`assert!(true)`, `assert_eq!(x, x)`, no assertions, test never calls code under test via the changed symbols) before asking S1 `is_tautological`.
- [ ] Define the strong postflight rule: ProofOfDone integration needs mutation evidence (cargo-mutants, currently missing on this machine) or a falsifier run; S1 opinion alone yields `Unverified`, never `Pass`.
- [ ] Implement `arbitrate`: adjacency check from line/AST ranges, orthogonal -> `code-intel` AST merge, semantic congruence via S1, conflicting or S1 timeout -> `RequiresSupervisorAttention` with no disk write.
- [ ] Implement diagnostics ranking over deduplicated diagnostics supplied by RTK (the verifier does not parse compiler output itself).
- [ ] Build a malicious-diff corpus generator (tamper variants x 4 languages x obfuscations) run at test time; never commit the corpus.
- [ ] Calibrate thresholds for residual S1 questions after S0-4 (BLOCKED-ON-ACCESS: working engine).

**Tests.**

| Category | Cases |
|---|---|
| unit | each `TamperKind` detected in Rust/Python/TS/C++ diffs; benign refactors of tests not flagged; command gate on a 200-command labelled list |
| proptest | gate decisions invariant under whitespace/quoting-equivalent rewrites of the same command; a diff that only adds assertions never yields `TestTampering` |
| fuzz | diff parser and command tokenizer on arbitrary bytes (WSL2 nightly) |
| golden-parity | residual S1 questions: Python reference vs Rust engine agreement >= 99% on the fixture set |
| smoke | end-to-end: tampering diff blocked before sandbox; green run with `assert!(true)` test yields `Unverified`/`Fail` |
| stress | 10k-line diffs, 1k-file diffs: bounded time, no panic; S1 timeout storms degrade to deterministic path |
| security | obfuscated tampering corpus (assert moved into comment, `cfg(any())`, macro-wrapped skip); secret-in-diff corpus built at test time; evidence redaction check |
| fixtures | labelled diffs and commands with provenance; fake keys generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Preflight tamper detection | async_function | `Verifier::preflight` | `crates/polaris-system-one/crates/verifier/src/preflight.rs` | `preflight_flags_removed_assertion`, `preflight_flags_added_ignore_attribute`, `preflight_allows_assertion_replaced_by_stronger_one`, `preflight_s1_cannot_override_deterministic_block`, `preflight_redacts_secret_evidence` |
| Command safety gate | async_function | `Verifier::check_command` | `crates/polaris-system-one/crates/verifier/src/gate.rs` | `gate_blocks_rm_rf_root`, `gate_asks_on_unknown_command_when_s1_down`, `gate_allows_cargo_test`, `gate_blocks_curl_pipe_shell`, `gate_quoting_variants_same_verdict` |
| Postflight tautology | async_function | `Verifier::postflight` | `crates/polaris-system-one/crates/verifier/src/postflight.rs` | `postflight_detects_assert_true`, `postflight_detects_self_equality`, `postflight_s1_only_green_is_unverified`, `postflight_timeout_is_unverified_not_pass` |
| Concurrent diff arbiter | async_function | `Verifier::arbitrate` | `crates/polaris-system-one/crates/verifier/src/arbiter.rs` | `arbiter_merges_disjoint_functions`, `arbiter_conflict_on_same_symbol_escalates`, `arbiter_s1_timeout_requires_supervisor`, `arbiter_never_writes_disk` |

**Risks & open questions.**
- VERIFIED: fail-closed for safety gate; arbiter/postflight closed is PROPOSED.
- LIKELY: zero-shot Decide cannot reliably judge "does this test check business logic"; deterministic detectors plus mutation testing carry the weight, S1 only prioritizes.
- UNVERIFIED: `code-intel` AST merge quality across languages (owned by that crate); tree-sitter-cuda coverage for CUDA safety rules.
- GUESSING: false-positive rate of tamper rules on real refactors; to be measured on fixtures.

**Chatlog sources.** Msgs 1 and 3 (speculative diff and safety pre-flight, command blacklist, `<0.99` block), 4-5 (pre/post-flight verifier, compiler diagnostics sifter, `SafetyViolation` enum), 10-11 (diff arbiter, adjacency check, AST merge), 29 (role guard). Corrected: substring matching on diffs, hardcoded `confidence: 0.98`, "S1 guarantees correct code" claims; the arbiter's cycle-breaker (more than 3 edits) is owned by runtime leases, not here.

---

### `polaris-system-one-watchdog` — meta-cognitive watchdog over context frames

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/watchdog` | L3 | lib | `polaris-system-one` | P4 |

**Purpose.** Inspects each proposed step of an agent context frame before it reaches S2 and returns a `HealthVerdict`: goal drift, semantic loop, role-boundary violation, or context fatigue. Agents are context frames over one S2 client and one S1 engine, so the watchdog needs no per-agent model. Fails open on timeout (`Unchecked`, logged) because it is advisory; Biscuit capabilities remain the authoritative role boundary.

**API sketch.**
```rust
pub struct StepFingerprint(u128);                       // xxh3-128 of normalized step text
pub struct FrameSnapshot { pub frame: FrameId, pub role: AgentRole, pub root_goal: String, pub recent: Vec<StepRecord>, pub context_tokens: u32, pub context_budget: u32 }
pub struct StepRecord { pub fp: StepFingerprint, pub shingles: ShingleSet, pub tool: Option<ToolId>, pub outcome: StepOutcome }
pub struct ProposedStep { pub thought: String, pub tool_call: Option<ToolCallFacts> }
pub enum HealthVerdict {
    Nominal,
    GoalDrift { alignment: Prob, anchor: String },
    Loop { repeats: StepIdx, disproven: HypothesisId },
    RoleViolation { tool: ToolId, role: AgentRole },
    ContextFatigue { pressure: f32, repetition: f32 },
    Unchecked { reason: UncheckedReason },
}
pub struct LoopWindow { /* ring buffer of StepRecord */ }
impl LoopWindow { pub fn push(&mut self, rec: StepRecord); pub fn detect(&self, next: &StepRecord, cfg: &LoopConfig) -> Option<LoopMatch>; }
pub struct RoleTable { /* role -> allowed tool ids + path scopes, data-driven */ }
pub struct Watchdog<D: DecisionEngine, C: SchemaCatalog> { /* engine, catalog, role table, config */ }
impl<D: DecisionEngine, C: SchemaCatalog> Watchdog<D, C> {
    pub async fn inspect(&self, snap: &FrameSnapshot, step: &ProposedStep) -> Result<HealthVerdict, WatchdogError>;
}
pub enum WatchdogError { Engine(DecisionError), Schema(SchemaError), EmptyGoal }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-system-one-schemas`. External (workspace.toml): `xxhash-rust = 0.8.19`. Proposed: `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: loop detection, role-table check, goal-drift question, fatigue measure, `inspect` ordering.
- [ ] Implement deterministic checks first and in this order: role table (a table lookup), exact-repeat fingerprint window, near-duplicate via normalized token shingles (Jaccard), token-pressure ratio.
- [ ] Implement the goal-drift Noul question ("does this step advance the root goal") via S1 with a calibrated band; drift in the Abstain band does nothing.
- [ ] Replace the chatlog's "semantic vectors" with shingle-based similarity now; add Needle-3 embeddings as an optional refinement after S0-6 (GLiNER2 is not an embedder).
- [ ] Replace "reasoning entropy" (needs logprobs, not exposed) with measurable repetition rate and token pressure; record both in the verdict.
- [ ] Implement disproven-hypothesis marking: a rejected patch variant fingerprint is added to the frame's negative-constraint list and returned in `Loop`.
- [ ] Implement the fail-open path: S1 timeout yields `Unchecked` while deterministic verdicts (loop, role, pressure) still apply.
- [ ] Make the 5-10 step thresholds (window size, shingle size, similarity cutoff, pressure ratio) config data in `schemas`, not constants.
- [ ] Build trajectory fixtures: recorded or hand-built looping and non-looping step sequences (synthetic, labelled as such).
- [ ] Measure the per-step overhead (deterministic path vs with S1) on the target machine.

**Tests.**

| Category | Cases |
|---|---|
| unit | role table denies out-of-role tool; window of N identical steps -> `Loop`; pressure above ratio -> `ContextFatigue` |
| proptest | loop detector never flags strictly distinct steps (distinct fingerprints and low shingle overlap); permuting unrelated frames does not change a frame's verdict |
| golden | labelled trajectories: loop precision/recall recorded (deterministic path needs no model) |
| smoke | S1 down: deterministic verdicts still produced, drift reported as `Unchecked` |
| load | 1k frames x 15 steps/s of fingerprint checks: bounded time per call |
| scalability | window sizes 5/50/500; 1 -> 32 frames |
| security | injected "ignore your role" text in tool output must not change the role table; oversize thoughts bounded before hashing |
| fixtures | `tests/fixtures/s1/watchdog/*.jsonl` trajectories (synthetic, labelled) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Loop window | struct | `LoopWindow` | `crates/polaris-system-one/crates/watchdog/src/loops.rs` | `exact_repeat_detected`, `near_duplicate_detected_above_similarity`, `distinct_steps_not_flagged`, `window_evicts_oldest`, `disproven_hypothesis_recorded` |
| Role boundary check | function | `RoleTable::check` | `crates/polaris-system-one/crates/watchdog/src/role.rs` | `test_engineer_cannot_edit_core_paths`, `researcher_read_only_tools`, `unknown_role_denies_by_default`, `table_loaded_from_data_not_substrings` |
| Context fatigue | function | `measure_fatigue` | `crates/polaris-system-one/crates/watchdog/src/fatigue.rs` | `pressure_ratio_computed_without_overflow`, `repetition_rate_high_for_repeated_ngrams`, `fatigue_requires_both_signals_when_configured` |
| Frame inspection | async_function | `Watchdog::inspect` | `crates/polaris-system-one/crates/watchdog/src/inspect.rs` | `inspect_runs_deterministic_checks_before_s1`, `inspect_s1_timeout_returns_unchecked_not_error`, `inspect_drift_in_abstain_band_is_nominal`, `inspect_loop_verdict_beats_drift` |

**Risks & open questions.**
- VERIFIED: watchdog fails open on timeout (PLAN core).
- LIKELY: the "5 microsecond" role check is true only for a table lookup; a semantic role judgement is not microseconds.
- UNVERIFIED: zero-shot S1 drift detection quality; may be too noisy to act on, in which case the verdict stays informational (shown, not enforced).
- GUESSING: shingle similarity catches the common "same error, same patch" loop; it will miss paraphrased loops (Needle embeddings later).

**Chatlog sources.** Msgs 28-29 (agents as context frames, S1 as continuous watchdog, four checks, `AgentHealthVerdict`, xxh3 sliding window), 11 (cycle breaker idea, owned by runtime leases). Corrected: substring role check (`contains("refactor core networking")`), "semantic vectors" from S1, entropy without logprobs, `<15 ms` claim.


---

### `polaris-system-one-nano` — Tier-0 Cactus Needle 3 bridge (tool-arg filling, local embeddings, optional chaos corpus)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-system-one/crates/nano` | L2 | lib | `polaris-system-one` | P8 |

**Purpose.** Supervises the Cactus Needle 3 engine (C++ `--serve`, or the Python package `cactus-needle`) inside `polaris-worker --mode nano` and exposes JSON-argument filling and local embeddings behind sdk traits. It is gated by spike S0-6 (Windows or WSL2 viability). Needle 3 facts (VERIFIED, plan item 7): 121M parameters, 8-29 MB `.cact`, Apache-2.0, tool calls and embeddings, C++ engine with `--serve`, Python package, no Rust bindings, Windows unconfirmed. The chatlog's architecture details and "about 66 ms / 28 MB RAM" are vendor-style claims, UNVERIFIED here.

**API sketch.**
```rust
pub enum NeedleTransport { CppServe { exe: PathBuf, port: u16 }, PythonPackage { python: PathBuf }, Wsl2 { distro: String, exe: String } }
pub struct NeedleConfig { pub model: PathBuf, pub transport: NeedleTransport, pub startup_timeout: Duration, pub request_timeout: Duration }
pub struct EmbeddingModelId(String);                                    // stored with every vector table; never mix models in one table
pub struct ToolArgsRequest { pub tool: ToolSchemaRef, pub text: String }
pub struct ToolArgs { pub json: serde_json::Value, pub schema_valid: bool }
pub struct NeedleBridge { /* supervised child + client */ }
impl NeedleBridge {
    pub async fn start(cfg: NeedleConfig) -> Result<Self, NanoError>;
    pub async fn fill_args(&self, req: &ToolArgsRequest) -> Result<ToolArgs, NanoError>;   // output validated against the tool JSON Schema
    pub async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, NanoError>;
    pub fn embedding_model(&self) -> &EmbeddingModelId;
    pub fn embedding_dim(&self) -> usize;
    pub async fn chaos_inputs(&self, shape: &TypeShape, n: u16, seed: Seed) -> Result<Vec<serde_json::Value>, NanoError>;   // optional enricher
    pub async fn shutdown(self) -> Result<(), NanoError>;
}
impl EmbeddingProvider for NeedleBridge { /* sdk trait */ }
pub enum NanoError { EngineMissing(PathBuf), StartupTimeout, ProtocolError(String), SchemaInvalid, Dimension { expected: usize, got: usize }, Unsupported(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`. External (workspace.toml): `tokio = 1.54.0`, `serde_json = 1.0.154`. Proposed additions: `reqwest = 0.13.5` (if `--serve` is HTTP; protocol UNVERIFIED), `process-wrap = 10.0.1` (kill-tree for the engine child), `jsonschema = 0.59.0` (argument validation), `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: engine supervision, argument filling with schema validation, embedding provider, embedding-model/dimension guard.
- [ ] Run spike S0-6: build or obtain the Needle C++ engine on Windows; if impossible, run it in WSL2 (needs Ubuntu 24.04 installed in P0); if both fail, defer T0 and keep this crate documented-only.
- [ ] Capture the real `--serve` protocol (HTTP vs stdio, request shapes) in a spike note; until then the transport layer is "API to confirm at implementation".
- [ ] Implement start/health/shutdown with kill-tree and startup timeout; the supervisor restarts the worker on crash.
- [ ] Implement `fill_args` with mandatory JSON Schema validation of the output; invalid output is an error the caller can route to S2, never executed.
- [ ] Implement `embed` and record `EmbeddingModelId` + dimension; refuse to write into a vector table created with another model (D8: Qwen3-Embedding-8B now, Needle later requires a separate table or a full re-embed).
- [ ] Measure embedding quality on a small code-search benchmark vs Qwen3-Embedding-8B before enabling `/search-local` on Needle (BLOCKED-ON-ACCESS for the Qwen baseline: Token Factory key).
- [ ] Evaluate `chaos_inputs` against a deterministic boundary generator (in `polaris-hacker`); keep only if it adds measurable new failures.
- [ ] Add `$needle(on|off)` gating and memory accounting to the worker eviction order.

**Tests.**

| Category | Cases |
|---|---|
| unit | dimension guard; invalid JSON from engine -> `ProtocolError`; schema-invalid args flagged |
| proptest | `fill_args` output either validates against the schema or returns an error, never an unvalidated value |
| spike | S0-6: Windows vs WSL2 run, RSS, latency p50/p95 for 10-token and 100-token prompts, embedding throughput |
| smoke | engine start -> fill one known tool call -> shutdown, kill-tree leaves no child |
| stress | engine crash loop; request flood against `--serve`; startup timeout |
| security | prompt-injection text in `ToolArgsRequest.text` cannot change which tool is called (tool fixed by caller); path-traversal strings in args pass through validation, not execution |
| fixtures | tool schemas + paired natural-language requests with expected args (synthetic, labelled) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Needle argument filling | async_function | `NeedleBridge::fill_args` | `crates/polaris-system-one/crates/nano/src/args.rs` | `fill_args_valid_output_passes_schema`, `fill_args_invalid_output_marked_invalid`, `fill_args_timeout_returns_error`, `fill_args_never_returns_unvalidated_value` |
| Needle embeddings | async_function | `NeedleBridge::embed` | `crates/polaris-system-one/crates/nano/src/embed.rs` | `embed_returns_fixed_dimension`, `embed_dimension_change_rejected`, `embed_empty_input_returns_empty`, `embed_model_id_recorded` |
| Engine supervision | struct | `NeedleBridge` | `crates/polaris-system-one/crates/nano/src/engine_proc.rs` | `start_missing_engine_fails_typed`, `shutdown_kills_child_tree`, `startup_timeout_reported`, `restart_after_crash_recovers` |

**Risks & open questions.**
- UNVERIFIED: Needle 3 on Windows (S0-6); fallback = WSL2, then defer T0 (P8 is a could-have).
- UNVERIFIED: embedding dimension and quality; vectors from different models are not comparable, so the vector store keys tables by model id.
- LIKELY: a function-calling model is a poor fuzzer; deterministic boundary values come first (`polaris-hacker`).
- UNVERIFIED: Needle's license is Apache-2.0 per plan; confirm the `.cact` weights' license separately.

**Chatlog sources.** Msgs 32-33 (Needle 3 tiers, local semantic search, `$needle`, `/search-local`), 34-35 (Needle as chaos generator). Corrected: "grammar-constrained" and "66 ms" as vendor claims; chaos generation demoted to optional; Needle embeddings cannot silently replace Qwen3-Embedding-8B in an existing table.

---

### `polaris-search` — facade for web search providers and the S1 relevance sifter

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-search` | L3 | facade | — | P5 |

**Purpose.** Re-exports the two search children. Providers (L2) fetch; sift (L3) races sources and distills context. Per D7 web search uses Tavily and SearXNG, both fully integrated.

**Re-export list.**
```rust
pub use polaris_search_providers as providers;   // L2: Tavily, SearXNG, safe fetch-and-extract
pub use polaris_search_sift as sift;             // L3: multi-source race + S1 sifting
```

**API sketch.** `pub use` lines only.

**Dependencies.** Internal: `polaris-search-providers`, `polaris-search-sift`. External: none.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: facade surface check.
- [ ] Re-export children and document the data flow (query -> providers -> sift -> pristine context) in `//!`.
- [ ] Add a doc-test that builds a `SiftRace` over a fake `KnowledgeSource` (fixture-backed, not a stub of production code).
- [ ] Document the `/search-local` split: local semantic search is `vector` + Needle/Qwen embeddings, not this crate.

**Tests.** doc-tests and the `check-layers`/`check-members` contract only.

**Risks & open questions.**
- VERIFIED: only two children; the nvidia-docs source and the `$rag` source are adapters wired at the composition root, not dependencies of this facade.

**Chatlog sources.** Msgs 3 (web distill), 17 (speculative race), 2.

---

### `polaris-search-providers` — Tavily and SearXNG providers plus a hardened fetch-and-extract

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-search/crates/providers` | L2 | lib | `polaris-search` | P5 |

**Purpose.** Implements the sdk `WebSearch` trait for Tavily (API key) and SearXNG (self-hosted in WSL2, JSON API), and a `Fetcher` that downloads pages under rate limits and converts HTML to Markdown. Both providers are first-class (D7). The fetcher is the main attack surface: it fetches attacker-chosen URLs on behalf of a model, so SSRF, size and content-type controls are part of the contract.

**API sketch.**
```rust
pub struct SearchQuery { pub text: QueryText, pub max_results: u8, pub freshness: Option<Freshness>, pub domains: DomainFilter }
pub struct SearchHit { pub url: Url, pub title: String, pub snippet: String, pub published: Option<Rfc3339>, pub provider_score: Option<f32>, pub provider: ProviderId }
pub struct TavilyConfig { pub base_url: Url, pub api_key: SecretHandle, pub timeout: Duration }          // key supplied by composition root (secrets crate)
pub struct SearxngConfig { pub base_url: Url, pub engines: Vec<String>, pub timeout: Duration }          // localhost allowlisted for this one base_url only
pub struct Tavily { /* reqwest client */ }
pub struct Searxng { /* reqwest client */ }
pub enum SearchProvider { Tavily(Tavily), Searxng(Searxng) }                                             // enum dispatch
impl WebSearch for SearchProvider { /* sdk trait */ }

pub struct FetchPolicy { pub max_bytes: u64, pub max_redirects: u8, pub timeout: Duration, pub allow_private_ranges: bool, pub allowed_content_types: Vec<String>, pub user_agent: String }
pub struct FetchedPage { pub final_url: Url, pub status: u16, pub content_type: String, pub markdown: String, pub truncated: bool, pub trust: Trust }   // Trust::Untrusted
pub struct Fetcher { /* client, policy, ratelimit handle */ }
impl Fetcher { pub async fn fetch(&self, url: &Url) -> Result<FetchedPage, SearchError>; }
pub enum SearchError { Http(String), Status(u16), RateLimited { retry_after: Option<Duration> }, Blocked(BlockReason), TooLarge, UnsupportedContent(String), Decode(String), ProviderConfig(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-ratelimit`. External (workspace.toml): `reqwest = 0.13.5` (rustls, http2, json, stream, charset), `serde_json = 1.0.154`, `htmd = 0.5.6`, `url = 2.5.8`. Proposed additions: `serde = 1.0.229`, `tokio = 1.54.0`, `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: Tavily request/response mapping, SearXNG mapping, SSRF guard, fetch limits, HTML-to-Markdown extraction.
- [ ] Define the provider-agnostic `SearchQuery/SearchHit` mapping and error taxonomy; keep wire structs private per provider.
- [ ] Implement SearXNG first (keyless): docker compose file for WSL2 (localhost-only bind, JSON output enabled in `settings.yml`, since JSON is off by default; LIKELY, confirm), health check, engine allow-list.
- [ ] Implement the SSRF guard: reject non-http(s), private/loopback/link-local/metadata ranges after DNS resolution, re-validate on every redirect, allowlist only the configured SearXNG base URL. Resolver pinning API in reqwest 0.13 is "API to confirm at implementation".
- [ ] Implement fetch limits: byte cap with streaming abort, decompression cap, redirect cap, content-type allowlist, charset via `charset` feature.
- [ ] Implement HTML to Markdown with `htmd`; strip scripts/hidden elements and mark all output `Trust::Untrusted`.
- [ ] Implement Tavily client (BLOCKED-ON-ACCESS: Tavily API key; endpoint, auth header and response schema are UNVERIFIED in this session, check the official docs at implementation time).
- [ ] Route every outbound query through the sdk `Redactor` before egress (code context leaks to third-party search); fail closed if the redactor errors.
- [ ] Integrate `polaris-ratelimit`: per-provider limiter, Retry-After handling; no provider quota numbers assumed.
- [ ] Add wiremock-based contract tests with fixtures labelled synthetic until re-recorded live.

**Tests.**

| Category | Cases |
|---|---|
| unit | URL normalization; provider response mapping; error mapping for 429/5xx |
| proptest | SSRF guard: any IP literal in private ranges (v4/v6, mapped addresses, decimal/hex forms) is rejected |
| fuzz | HTML-to-Markdown on arbitrary bytes and malformed charsets (WSL2 nightly) |
| contract | wiremock for Tavily and SearXNG (synthetic fixtures, labelled); SearXNG live smoke when the container is up |
| smoke | SearXNG container up -> query -> hits (SKIPPED: no container); Tavily SKIPPED: no key |
| load | 50 concurrent fetches under the semaphore; memory stays bounded by `max_bytes` |
| stress | slow-loris server, redirect loops, gzip bomb, endless chunked body |
| security | SSRF corpus (127.0.0.1, 169.254.169.254, DNS rebinding via rebinding fixture server, redirect to internal), prompt-injection pages (hidden text instructions) must arrive tagged `Untrusted` and be neutralized by fencing downstream; query-egress PII corpus |
| fixtures | recorded search JSON (synthetic), HTML pages with nav/footer noise |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| SSRF guard | function | `validate_target` | `crates/polaris-search/crates/providers/src/ssrf.rs` | `rejects_loopback_v4_v6`, `rejects_metadata_ip`, `rejects_redirect_to_private`, `rejects_ipv4_mapped_ipv6_private`, `allows_configured_searxng_base_only` |
| Bounded fetch | async_function | `Fetcher::fetch` | `crates/polaris-search/crates/providers/src/fetch.rs` | `fetch_aborts_over_max_bytes`, `fetch_rejects_disallowed_content_type`, `fetch_respects_redirect_cap`, `fetch_marks_output_untrusted`, `fetch_retry_after_surfaces_rate_limited` |
| SearXNG provider | struct | `Searxng` | `crates/polaris-search/crates/providers/src/searxng.rs` | `searxng_maps_results_to_hits`, `searxng_json_disabled_returns_provider_config_error`, `searxng_down_returns_http_error`, `searxng_query_is_redacted_before_send` |
| Tavily provider | struct | `Tavily` | `crates/polaris-search/crates/providers/src/tavily.rs` | `tavily_maps_results_to_hits`, `tavily_401_returns_provider_config_error`, `tavily_429_returns_rate_limited`, `tavily_key_never_logged` |

**Risks & open questions.**
- VERIFIED: D7 requires both providers; no Tavily key exists yet (D9), so the Tavily path is BLOCKED-ON-ACCESS and reports SKIPPED.
- LIKELY: SearXNG needs `json` enabled under `search.formats`; instance may rate-limit or get blocked by upstream engines.
- UNVERIFIED: Tavily endpoint/auth/response schema and quota (confirm from official docs at implementation).
- LIKELY: SearXNG results quality/availability varies; fetch is the dependable path for `$links`.
- Open: robots.txt policy for `$links`/crawl (default: honor, document).

**Chatlog sources.** Msgs 3 (query normalization, SEO pruning, 2-3 URLs to scrape), 17 (WebResearch semaphore, 429/IP-ban risk). Corrected: scraping "via OpenShell" is replaced by a hardened in-process fetcher (pages are data, not code); semaphore sizes are config hypotheses.

---

### `polaris-search-sift` — speculative multi-source race and S1 relevance sifting

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-search/crates/sift` | L3 | lib | `polaris-search` | P5 |

**Purpose.** Queries NVIDIA docs, `$rag` and `$websearch` concurrently under named semaphores, validates hits with S1, aborts losers, and "sifts" retrieved chunks into pristine, provenance-tagged context. The race is a latency hedge, not a quality oracle: the policy can wait for the best of several hits within a deadline. Sources are adapters (`KnowledgeSource`) implemented at the composition root, so this crate has no edge to `nvidia-docs` or `storage-vector`.

**API sketch.**
```rust
pub struct SourceId(String);
pub struct Chunk { pub source: SourceId, pub locator: Locator, pub text: String, pub score_hint: Option<f32>, pub trust: Trust }   // Locator: url | doc+page+line | symbol
pub trait KnowledgeSource: Send + Sync {
    fn id(&self) -> SourceId;
    fn search(&self, q: &SiftQuery) -> impl Future<Output = Result<Vec<Chunk>, SourceError>> + Send;
}
pub struct SiftQuery { pub text: String, pub goal: String, pub max_chunks: u16 }
pub enum RaceMode { FirstAccepted, BestWithinDeadline }
pub struct RacePolicy { pub mode: RaceMode, pub deadline: Duration, pub accept: Prob, pub max_validations: u8 }             // accept default comes from schemas (uncalibrated)
pub struct SemaphoreSet { pub docs: Arc<Semaphore>, pub rag: Arc<Semaphore>, pub web: Arc<Semaphore> }
pub enum RaceOutcome { Accepted { hit: ValidatedHit, aborted: Vec<SourceId> }, NoneSufficient { best: Option<ValidatedHit> }, FailedOpen { reason: String, fallback: Vec<Chunk> } }
pub struct DropStats { pub dropped: u32, pub est_tokens_saved: u32, pub dropped_refs: Vec<ContentRef> }                        // recoverable via polaris_expand
pub struct Distilled { pub kept: Vec<KeptChunk>, pub dropped: DropStats }
pub struct KeptChunk { pub text: String, pub locator: Locator, pub relevance: Prob, pub density: Prob, pub trust: Trust }
pub struct Sifter<D: DecisionEngine, C: SchemaCatalog> { /* engine, catalog */ }
impl<D: DecisionEngine, C: SchemaCatalog> Sifter<D, C> {
    pub async fn race<S: KnowledgeSource>(&self, q: &SiftQuery, sources: &[S], sems: &SemaphoreSet, policy: &RacePolicy) -> Result<RaceOutcome, SiftError>;
    pub async fn sift(&self, goal: &str, chunks: Vec<Chunk>, budget: TokenBudget) -> Result<Distilled, SiftError>;
}
pub enum SiftError { Source(SourceError), Engine(DecisionError), Schema(SchemaError), Cancelled }
```
Race implementation notes: `tokio::task::JoinSet`, `abort_all()` after acceptance; permits are RAII-released. Abort cancels at `.await` points and does not stop `spawn_blocking` work.

**Dependencies.** Internal: `polaris-sdk`, `polaris-search-providers`. External (workspace.toml): `tokio = 1.54.0`. Proposed additions: `tokio-util = 0.7.20` (cancellation), `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: race, validation, sift with token budget, drop manifest, fail-open fallback.
- [ ] Define `KnowledgeSource` and the semaphore set; name pools `docs`, `rag`, `web` (sizes are config, chatlog 4-8/8-12/16 are hypotheses).
- [ ] Implement `race` with both modes; default `BestWithinDeadline` for research, `FirstAccepted` only for latency-critical lookups.
- [ ] Implement S1 validation batching (all candidate hits of a source in one multi-task request) and a cap `max_validations` to bound S1 load.
- [ ] Implement `sift`: Noul `relevant_to_goal` + Score `information_density`; keep order stable; enforce the token budget by dropping lowest relevance first.
- [ ] Persist dropped chunks to CAS and return `dropped_refs` so S2 can recover them (`polaris_expand`); sifting must be lossless-recoverable.
- [ ] Implement fail-open: on S1 timeout return lexically ranked chunks truncated to the token budget, flagged `fallback`.
- [ ] Tag provenance (URL / doc page / line) and `Trust` on every kept chunk; never merge chunks across sources without provenance.
- [ ] Build fixtures: 30 queries with labelled relevant chunks across docs-style, RAG-style and web-style noise (synthetic, labelled).
- [ ] Measure token savings (approximate token estimate, labelled approximate) and recall of relevant chunks on the fixtures; record both.

**Tests.**

| Category | Cases |
|---|---|
| unit | deadline expiry returns `NoneSufficient`; all-sources-fail returns `FailedOpen`; budget enforcement drops lowest relevance first |
| proptest | sift output is a subsequence of input order; token estimate never exceeds budget; permits released after any outcome |
| loom | race coordinator state (accept vs late arrival) modelled without tokio: no double-accept, no leaked permit count |
| golden-parity | chunk relevance verdicts S1 Rust vs Python reference >= 99% agreement on fixtures |
| smoke | three fixture sources, one fast-irrelevant, one slow-relevant: `BestWithinDeadline` picks the relevant one |
| load | 50 concurrent races; semaphore queueing bounded; p99 recorded |
| scalability | 10 / 100 / 1000 chunks per sift |
| stress | cancellation during S1 validation; source that never completes; source that panics (task isolation via JoinSet) |
| security | injection in chunk text claiming relevance cannot raise a chunk above the measured bound; sifter is not an injection defense and kept chunks stay `Untrusted`; PII in chunks flagged downstream |
| fixtures | `tests/fixtures/search/*.jsonl` |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Speculative source race | async_function | `Sifter::race` | `crates/polaris-search/crates/sift/src/race.rs` | `race_accepts_first_valid_and_aborts_rest`, `race_best_within_deadline_waits_for_better_hit`, `race_all_sources_fail_returns_failed_open`, `race_releases_permits_on_abort`, `race_honors_max_validations` |
| Chunk sifting | async_function | `Sifter::sift` | `crates/polaris-search/crates/sift/src/sift.rs` | `sift_drops_low_relevance_chunks`, `sift_respects_token_budget`, `sift_keeps_provenance`, `sift_timeout_falls_back_to_lexical`, `sift_preserves_input_order` |
| Drop manifest | struct | `DropStats` | `crates/polaris-search/crates/sift/src/dropped.rs` | `dropped_chunks_recoverable_by_ref`, `drop_stats_count_matches_dropped`, `dropped_refs_unique` |

**Risks & open questions.**
- LIKELY: "first S1-approved wins" is order-biased (the local index always answers first); hence `BestWithinDeadline` default for research.
- LIKELY: `abort_all()` frees futures at await points only; blocking calls inside sources are not interrupted.
- UNVERIFIED: S1 can score 50-200 chunks per query within a useful time on this CPU; the lexical pre-cut keeps the S1 set small, caps from S0-4.
- GUESSING: >80% noise removal (chatlog) depends on corpus; measure on fixtures.

**Chatlog sources.** Msgs 3 (RAG/Web sifter, `ContextDistiller`, `FilteredContext`, `saved_tokens_est`), 17 (speculative race, semaphores, `abort_all`), 2. Corrected: `is_informative = true` stub, first-hit semantics, `todo!()` fetchers, "pristine context" must stay recoverable.


---

### `polaris-ext` — facade for MCP client and skills loader

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ext` | L2 | facade | — | P7 |

**Purpose.** Re-exports the two extension mechanisms: `$mcp` servers (tools from outside) and `$skills` (procedural knowledge from files). Both feed the tool and context registries; both are untrusted input channels.

**Re-export list.**
```rust
pub use polaris_ext_mcp as mcp;        // rmcp client: stdio child + streamable HTTP, tool registry merge
pub use polaris_ext_skills as skills;  // SKILL.md loader, trust levels, on-demand selection
```

**API sketch.** `pub use` lines only.

**Dependencies.** Internal: `polaris-ext-mcp`, `polaris-ext-skills`. External: none.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: facade surface check.
- [ ] Re-export children; document the shared trust model (global vs repo-provided extensions; nothing from a repo is trusted until approved).
- [ ] Add the `//!` threat note: MCP descriptions and skill bodies are prompt-injection vectors; capabilities (Biscuit) bound damage, they do not prevent it.

**Tests.** `check-layers`/`check-members` contract and doc-tests only.

**Risks & open questions.**
- LIKELY: users will attach MCP servers from untrusted repos; default must be "ask before connect".

**Chatlog sources.** Msgs 1 (`$mcp`, `$skills`, MCP client crate), 3 (MCP indexing for S1 whitelist).

---

### `polaris-ext-mcp` — MCP client and tool-registry merge for `$mcp` servers

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ext/crates/mcp` | L2 | lib | `polaris-ext` | P7 |

**Purpose.** Connects configured MCP servers over stdio (child process) or streamable HTTP using `rmcp`, lists and invokes tools, and normalizes them into the Polaris tool registry under namespaced ids. It also exports a bounded, sanitized tool summary for S1 pruning. Stdio MCP servers are arbitrary code and must run in a sandbox backend, not directly on the Windows host.

**API sketch.**
```rust
pub struct ServerName(String);
pub struct NamespacedToolId { pub server: ServerName, pub tool: String }          // "server::tool"; collisions impossible
pub enum McpTransport {
    Stdio { command: Vec<String>, cwd: Option<PathBuf>, env: Vec<(String, SecretRef)> },
    StreamableHttp { url: Url, auth: Option<SecretRef> },
}
pub enum TrustLevel { UserGlobal, RepoApproved, RepoUnapproved }                  // RepoUnapproved never auto-connects
pub struct McpServerSpec { pub name: ServerName, pub transport: McpTransport, pub trust: TrustLevel, pub startup_timeout: Duration, pub call_timeout: Duration, pub tool_allowlist: Option<Vec<String>>, pub max_result_bytes: u64 }
pub struct ToolSpec { pub id: NamespacedToolId, pub description: String, pub input_schema: serde_json::Value, pub prune_summary: String }   // prune_summary: truncated + stripped
pub struct ToolOutcome { pub content: Vec<ToolContent>, pub is_error: bool, pub truncated: bool }
pub struct McpHub { /* servers, rmcp clients */ }
impl McpHub {
    pub fn new(launcher: impl ProcessLauncher) -> Self;                           // sdk PlatformOps-backed launcher or sandbox exec
    pub async fn connect(&mut self, spec: McpServerSpec) -> Result<(), McpError>;
    pub fn tools(&self) -> Vec<ToolSpec>;
    pub async fn call(&self, id: &NamespacedToolId, args: serde_json::Value, cancel: &CancelToken) -> Result<ToolOutcome, McpError>;
    pub async fn refresh(&mut self, server: &ServerName) -> Result<(), McpError>;   // tools/list_changed
    pub async fn disconnect(&mut self, server: &ServerName) -> Result<(), McpError>;
}
pub enum McpError { Spawn(String), Handshake(String), VersionMismatch, ToolNotFound(NamespacedToolId), SchemaInvalid(String), Timeout, ResultTooLarge, Denied(TrustLevel), Transport(String) }
```
`rmcp = 3.5.1` types and method names are "API to confirm at implementation".

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`. External (workspace.toml): `rmcp = 3.5.1` (client, macros, transport-child-process, transport-streamable-http-client-reqwest, reqwest-tls-no-provider), `tokio = 1.54.0`, `schemars = 1.2.3`. Proposed additions: `serde = 1.0.229`, `serde_json = 1.0.154`, `url = 2.5.8`, `jsonschema = 0.59.0`, `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: connect + handshake, namespacing, schema normalization, result bounding, trust gating, prune-summary sanitizer.
- [ ] Define `ProcessLauncher` so stdio servers start via `PlatformOps` (Job Object kill-tree) or inside a sandbox, never a bare `Command` that bypasses the reaper; `rmcp`'s child-process transport must accept a pre-built command (confirm).
- [ ] Implement connect/list/call/refresh/disconnect with per-call timeout, cancellation and `max_result_bytes`; oversized results go to CAS and return a `§ref`.
- [ ] Implement schema normalization: validate each tool `inputSchema` with `jsonschema`, reject invalid ones, record dialect, drop unsupported keywords for the S2 tool-calling format (Token Factory tool-schema limits UNVERIFIED, S0-5).
- [ ] Implement namespacing and collision handling; keep display names stable for the TUI.
- [ ] Implement trust gating: repo-provided specs need explicit approval; approval stored with a hash of the spec (changed spec re-prompts).
- [ ] Implement `prune_summary`: first N chars, control characters and markdown/HTML stripped, instruction-like prefixes neutralized; this is what S1 sees, never the full description.
- [ ] Wire the output to the tool registry in `polaris-tools-builtin` through an sdk trait at the composition root (no sibling edge).
- [ ] Verify against a real reference server (BLOCKED-ON-ACCESS only for the Contree MCP, which has its own crate; a keyless local test server such as an echo MCP server is fine).
- [ ] Add reconnect/backoff and a supervisor hook so a crashed server marks its tools unavailable instead of failing the turn.

**Tests.**

| Category | Cases |
|---|---|
| unit | namespacing; allowlist filter; result truncation; trust matrix (RepoUnapproved denied) |
| proptest | any JSON schema input either normalizes to a valid schema or returns `SchemaInvalid`, never panics; prune summary length bounded and free of control chars |
| fuzz | JSON-RPC frame decoding and schema normalizer on arbitrary JSON (WSL2 nightly) |
| contract | handshake and tools/list/call against a local fixture MCP server (stdio and streamable HTTP) |
| smoke | connect fixture server, call tool, disconnect, no leaked child |
| load | 20 servers x 50 tools; list latency; 100 concurrent calls |
| stress | server crashes mid-call, hangs on handshake, floods output |
| security | tool-poisoning corpus (descriptions with injected instructions, names that shadow built-ins, ANSI/zero-width characters); stdio server env cannot read host secrets; repo-config attempt to auto-run command is blocked |
| fixtures | fixture MCP servers (generated), schema zoo |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| MCP connect and handshake | async_function | `McpHub::connect` | `crates/polaris-ext/crates/mcp/src/hub.rs` | `connect_stdio_fixture_lists_tools`, `connect_http_fixture_lists_tools`, `connect_handshake_timeout_errors`, `connect_repo_unapproved_denied`, `connect_spawn_goes_through_launcher` |
| Tool call bounding | async_function | `McpHub::call` | `crates/polaris-ext/crates/mcp/src/call.rs` | `call_returns_content`, `call_truncates_oversize_result`, `call_cancel_aborts_request`, `call_timeout_returns_timeout_error`, `call_unknown_tool_errors` |
| Schema normalization | function | `normalize_schema` | `crates/polaris-ext/crates/mcp/src/schema.rs` | `valid_schema_passes`, `invalid_schema_rejected`, `unsupported_keywords_dropped_and_recorded`, `ref_cycles_rejected` |
| Prune summary sanitizer | function | `prune_summary` | `crates/polaris-ext/crates/mcp/src/summary.rs` | `summary_truncates_to_limit`, `summary_strips_control_and_zero_width`, `summary_neutralizes_instruction_prefix`, `summary_deterministic` |

**Risks & open questions.**
- VERIFIED: `rmcp` is pinned 3.5.1 with the features above; the Contree sidecar (separate crate) also uses it, so API drift affects both.
- LIKELY: stdio servers are the common distribution form (npx/uvx); they need Node/Python in the sandbox image, which the Polaris sandbox image may lack.
- UNVERIFIED: how Token Factory handles very large `tools` arrays and strict schemas (S0-5).
- Open: MCP spec revision negotiated by rmcp 3.5.1; confirm at implementation.

**Chatlog sources.** Msgs 1 (`$mcp`, `polaris-mcp` stdio/SSE), 3 (index all MCP tools in S1, dynamic whitelist, top-3 of 50), 25 (none). Corrected: raw description fed to S1 (injection), unmanaged child processes, "SSE transport" naming (streamable HTTP in the pinned rmcp).

---

### `polaris-ext-skills` — `$skills` loader with trust levels and on-demand selection

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ext/crates/skills` | L2 | lib | `polaris-ext` | P7 |

**Purpose.** Discovers SKILL.md-format procedural skills in `~/.polaris/skills` (user) and `.polaris/skills` (repo), parses front matter, and mounts selected bodies into a frame's context on demand. Only name and description are indexed; bodies are loaded when selected (progressive disclosure keeps the prefix stable and small). Selection can use S1 through a generic `DecisionEngine`, or a deterministic lexical selector when S1 is down.

**API sketch.**
```rust
pub struct SkillName(String);
pub enum SkillSource { UserGlobal, RepoApproved, RepoUnapproved }
pub struct SkillMeta { pub name: SkillName, pub description: String, pub source: SkillSource, pub path: PathBuf, pub body_blake3: Blake3Hex, pub requires_tools: Vec<String> }
pub struct Skill { pub meta: SkillMeta, pub body: String }
pub struct SkillRegistry { /* indexed metas, lazy bodies */ }
impl SkillRegistry {
    pub fn discover(roots: &[SkillRoot]) -> Result<Self, SkillError>;
    pub fn list(&self) -> &[SkillMeta];
    pub fn load(&self, name: &SkillName) -> Result<Skill, SkillError>;
    pub async fn select<D: DecisionEngine, C: SchemaCatalog>(&self, task: &str, engine: &D, catalog: &C, k: TopK) -> Result<Vec<Ranked<SkillName>>, SkillError>;
    pub fn select_lexical(&self, task: &str, k: TopK) -> Vec<Ranked<SkillName>>;
    pub fn approve_repo_skill(&mut self, name: &SkillName, approval: &Approval) -> Result<(), SkillError>;
}
pub fn parse_skill(src: &str) -> Result<(FrontMatter, String), SkillError>;
pub enum SkillError { Io(String), FrontMatter(String), TooLarge, NameCollision(SkillName), NotApproved(SkillName), Unknown(SkillName) }
```

**Dependencies.** Internal: `polaris-sdk`. External (workspace.toml): `pulldown-cmark = 0.13.4`, `serde = 1.0.229`. Proposed additions: `serde-saphyr = 2.0.0` (YAML front matter; already pinned in the workspace; alternative is TOML front matter), `blake3 = 1.8.7`, `thiserror = 2.0.21`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: front-matter parsing, discovery precedence, trust gating, selection (S1 and lexical), body loading limits.
- [ ] Decide front-matter format (YAML as in the SKILL.md convention vs TOML) and record the decision; add the YAML dependency to `workspace.toml` if chosen.
- [ ] Implement discovery with precedence repo-approved over user-global on name collision only when approved; otherwise user-global wins.
- [ ] Implement size limits (front matter, body) and path confinement (skill resource paths cannot escape their directory).
- [ ] Implement trust gating: repo skills are `RepoUnapproved` until approved; approval stored with the body hash, a changed body re-prompts.
- [ ] Implement `select_lexical` (BM25-like over name+description) as the deterministic fallback.
- [ ] Implement S1 `select` through the sdk trait with schema id `skill.relevance`; only descriptions are sent.
- [ ] Ensure a skill cannot widen capabilities: it contributes text and suggested tools only; tool access still goes through the frame's Biscuit token.
- [ ] Add `requires_tools` checking against the available tool registry and surface missing tools in `/status`.
- [ ] Build a small bundled skill set (e.g. git-bisect, database-migrate, from the chatlog examples) as fixtures with provenance.

**Tests.**

| Category | Cases |
|---|---|
| unit | parse valid/invalid front matter; precedence rules; approval hash mismatch re-prompts |
| proptest | `parse_skill` never panics and body round-trips byte-exactly; lexical selection returns <= k, deterministic |
| fuzz | front-matter parser on arbitrary bytes (WSL2 nightly) |
| smoke | discover fixture dirs, select by task, load body |
| scalability | 10 / 200 / 2000 skills: index time and memory |
| security | skill body with hidden instructions, path traversal in resource refs, oversize body, symlink escape; unapproved repo skill never injected |
| fixtures | skill directories (generated for hostile cases) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Skill parsing | function | `parse_skill` | `crates/polaris-ext/crates/skills/src/parse.rs` | `parse_valid_skill_returns_meta_and_body`, `parse_missing_name_errors`, `parse_oversize_body_rejected`, `parse_body_preserved_exactly` |
| Skill discovery | function | `SkillRegistry::discover` | `crates/polaris-ext/crates/skills/src/discover.rs` | `discover_finds_user_and_repo_skills`, `discover_name_collision_prefers_user_unless_approved`, `discover_skips_symlink_escape`, `discover_empty_roots_ok` |
| Skill trust gating | method | `SkillRegistry::approve_repo_skill` | `crates/polaris-ext/crates/skills/src/trust.rs` | `unapproved_repo_skill_not_loadable`, `approval_binds_to_body_hash`, `changed_body_requires_reapproval` |
| Skill selection | async_function | `SkillRegistry::select` | `crates/polaris-ext/crates/skills/src/select.rs` | `select_returns_top_k_by_score`, `select_s1_timeout_uses_lexical`, `select_excludes_unapproved`, `select_sends_description_only` |

**Risks & open questions.**
- LIKELY: the SKILL.md convention is YAML front matter; we do not have a YAML crate in this crate's workspace.toml list (see TODO).
- LIKELY: a repo that ships `.polaris/skills` is an attacker-controlled prompt channel; approval flow is mandatory.
- UNVERIFIED: how many skills S1 can usefully rank per task; lexical selection may be the right default.

**Chatlog sources.** Msgs 1 (`$skills(git-bisect, database-migrate)`), 3 (S1-selected relevant items). Corrected: "S1 selects skills" kept but deterministic-first and trust-gated.

---

### `polaris-ingest` — document ingest (docx, xlsx/csv, pdf, pptx, html, md, images) with an OCR pair

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ingest` | L2 | lib | — | P5 |

**Purpose.** Implements the sdk `DocParser` per format and normalizes documents to Markdown blocks with provenance (page, line, sheet/cell). Scanned PDF pages and images go to an OCR pair: `ocrs` (pure Rust on `rten`) and `oar-ocr` (PaddleOCR via ONNX Runtime). It runs only in `polaris-worker --mode ingest`, so a parser crash or abort cannot take down the kernel. Output is untrusted data and goes through PII masking before any cloud call.

**API sketch.**
```rust
pub enum DocKind { Docx, Xlsx, Csv, Pdf, Pptx, Html, Markdown, Image, Text }
pub struct IngestLimits { pub max_bytes: u64, pub max_pages: u32, pub max_uncompressed_bytes: u64, pub max_zip_entries: u32, pub max_image_pixels: u64, pub wall_timeout: Duration }
pub enum OcrPolicy { Never, ScansOnly, Always }
pub struct IngestRequest { pub path: PathBuf, pub kind_hint: Option<DocKind>, pub limits: IngestLimits, pub ocr: OcrPolicy }
pub struct Loc { pub page: Option<u32>, pub line: Option<u32>, pub sheet: Option<String>, pub cells: Option<String> }
pub enum Block { Heading { level: u8, text: String, loc: Loc }, Paragraph { text: String, loc: Loc }, Table { headers: Vec<String>, rows: Vec<Vec<String>>, loc: Loc }, Code { lang: Option<String>, text: String, loc: Loc }, ImageText { ocr: OcrResult, loc: Loc } }
pub struct Document { pub id: ContentId, pub kind: DocKind, pub blocks: Vec<Block>, pub warnings: Vec<IngestWarning>, pub trust: Trust }
pub enum DocumentContent { Text(String), StructuredTable { headers: Vec<String>, rows: Vec<Vec<String>> }, ScannedImageNeedsOcr(Vec<u8>) }
pub struct OcrLine { pub text: String, pub bbox: [f32; 4], pub confidence: f32 }
pub struct OcrResult { pub lines: Vec<OcrLine>, pub engine: OcrEngineId }
pub enum OcrEngine { Ocrs(OcrsEngine), Oar(OarEngine) }                         // enum dispatch
pub fn detect_kind(path: &Path, head: &[u8]) -> Result<DocKind, IngestError>;   // magic bytes via infer, not extension alone
pub fn ingest(req: &IngestRequest, ocr: &OcrEngine) -> Result<Document, IngestError>;
pub fn to_markdown(doc: &Document) -> String;
pub enum IngestError { Io(String), UnknownKind, LimitExceeded(LimitKind), Parse { kind: DocKind, detail: String }, Ocr(String), Timeout, Encoding(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-log`. External (workspace.toml): `docx-rs = 0.4.22`, `calamine = 0.37.0`, `pdf-extract = 0.12.1`, `pdfium-render = 0.9.4`, `ocrs = 0.13.1`, `rten = 0.26.0` (aligned with ocrs 0.13.1; 0.27 would duplicate the runtime), `oar-ocr = 0.10.0` (default-features off, `simd`; requires `ort =2.0.0-rc.13`), `image = 0.25.10`, `htmd = 0.5.6`, `pulldown-cmark = 0.13.4`, `scraper = 0.27.0`, `encoding_rs = 0.8.42`, `infer = 0.22.0`. Proposed additions: `thiserror = 2.0.21`, a zip reader with entry/size limits for OOXML pre-validation (crate and API to confirm), `ort` is transitive via oar-ocr.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: kind detection, docx to Markdown, xlsx/csv to tables, pdf text vs scan detection, OCR routing, limit enforcement, Markdown rendering.
- [ ] Implement `detect_kind` by magic bytes with `infer` and cross-check extension; mismatches produce a warning and trust the bytes.
- [ ] Implement pre-parse container checks for OOXML/ZIP (entry count, total and per-entry uncompressed size, compression ratio, path traversal in names) before handing bytes to `docx-rs`/`calamine`.
- [ ] Implement docx via `docx-rs` (paragraphs, headings, tables, lists); xlsx/csv via `calamine` (sheet names, cell types, merged cells; cached values for formulas) and CSV with `encoding_rs`; pptx and html (`scraper` + `htmd`), md (`pulldown-cmark`).
- [ ] Implement PDF: text layer via `pdf-extract`; per-page scan detection (low text-per-page or no text layer) routes that page to render (`pdfium-render`) then OCR. The pdfium shared library must be supplied (source and licence to confirm); document the DLL-beside-exe rule.
- [ ] Implement the OCR pair: `ocrs` first (pure Rust, needs `.rten` model files, source and licence to confirm), `oar-ocr` for higher quality/code text; both models are downloaded assets (BLOCKED-ON-ACCESS: model download source and licence verification).
- [ ] Define the engine selection rule (language, confidence, fallback to the other engine when mean confidence is below a calibrated bound) and measure on a scan fixture set.
- [ ] Implement `to_markdown` with table escaping (pipes, newlines) and provenance comments.
- [ ] Wire the poison-document handling: document blake3 id is reported to the supervisor so repeated crashes route to the DLQ and the user sees which page was skipped.
- [ ] Add memory accounting for OCR models into worker eviction (ingest models are first to evict).
- [ ] Build fixtures: small public-domain or generated docx/xlsx/pdf/png with provenance; hostile files generated at test time.

**Tests.**

| Category | Cases |
|---|---|
| unit | kind detection; markdown table escaping; heading levels; xlsx merged cells; CSV encodings/BOM |
| proptest | `to_markdown` round-trips table cells containing `|`, newlines, backticks without breaking structure; `Loc` ordering monotone within a document |
| fuzz | every parser (docx/xlsx/pdf/pptx/html/md/image) on mutated seeds under limits, in WSL2 nightly; timeout and RSS cap enforced |
| golden | insta snapshots of normalized Markdown for fixture docs |
| smoke | ingest each fixture kind in the ingest worker; scanned PDF page OCR'd with both engines |
| load | 100 MB PDF, 50k-row xlsx: wall time, RSS peak (targets to measure) |
| scalability | 1 / 10 / 100 concurrent ingests across worker restarts |
| stress | poison document crashes the worker repeatedly: circuit breaker + DLQ, other documents still ingest; OOM cap |
| security | zip bomb, billion-laughs-style XML, PDF decompression bomb, path-traversal entry names, extension-spoofed polyglots, huge-dimension PNG; document-embedded prompt injection arrives tagged `Untrusted` |
| fixtures | `tests/fixtures/ingest/` with provenance and licence; hostile set generated at test time |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Document kind detection | function | `detect_kind` | `crates/polaris-ingest/src/detect.rs` | `detect_docx_by_magic_not_extension`, `detect_mismatched_extension_warns`, `detect_unknown_bytes_errors`, `detect_empty_file_errors` |
| Container limits | function | `check_container` | `crates/polaris-ingest/src/limits.rs` | `zip_bomb_rejected`, `too_many_entries_rejected`, `path_traversal_entry_rejected`, `normal_docx_accepted` |
| Spreadsheet to tables | function | `parse_xlsx` | `crates/polaris-ingest/src/xlsx.rs` | `xlsx_sheets_become_tables`, `xlsx_merged_cells_expanded`, `xlsx_formula_uses_cached_value`, `xlsx_empty_sheet_yields_no_table` |
| PDF scan routing | function | `route_pdf_pages` | `crates/polaris-ingest/src/pdf.rs` | `text_page_uses_text_layer`, `scan_page_routed_to_ocr`, `mixed_pdf_routes_per_page`, `encrypted_pdf_returns_typed_error` |
| OCR engine selection | function | `OcrEngine::recognize` | `crates/polaris-ingest/src/ocr.rs` | `ocrs_result_has_lines_and_bboxes`, `low_confidence_falls_back_to_other_engine`, `oversize_image_rejected`, `ocr_deterministic_for_same_input` |

**Risks & open questions.**
- VERIFIED: `lopdf` direct and Tesseract C-FFI are dropped (chatlog); `pdf-extract` wraps lopdf; OCR pair is `ocrs` + `oar-ocr`.
- VERIFIED: `ort` is pinned `=2.0.0-rc.13` because `oar-ocr-core` requires it; `rten 0.26.0` matches `ocrs 0.13.1`.
- UNVERIFIED: that the ingest worker and the S1 worker can both load ORT via `load-dynamic` without DLL conflicts (separate processes, same DLL).
- UNVERIFIED: pdfium binary distribution and licence; model asset licences for ocrs and PaddleOCR.
- LIKELY: `catch_unwind` cannot contain segfaults in native code; process isolation plus DLQ is the real containment, with Tokio `JoinError::Panic` for task panics.
- GUESSING: OCR quality on code screenshots; measure before promising `@img` for code.

**Chatlog sources.** Msgs 20-21 (`@docx`, `@pdf`, `@xlsx`, `@img`, `@scans`, native parsers, dual OCR, PII after ingest, DLQ/circuit breaker), 25 (screenshots as `@img` input). Corrected: lopdf, Tesseract, `AssertUnwindSafe` as protection from segfaults, `todo!()` parsers, "OCR actor in-process".


---

### `polaris-nvidia-docs` — NVIDIA knowledge engine: docs ingestion, index, `$nvidia(auto)` profiles, `/nv-profile`, `/nv-verify`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-nvidia-docs` | L3 | lib | — | P5 |

**Purpose.** Makes the agent competent on the NVIDIA stack (CUDA, PTX, TensorRT-LLM, Triton, NCCL, OpenShell, NemoClaw, Nemotron). It ingests official docs incrementally into a lexical (SQLite FTS5) plus vector (LanceDB) index, detects the stack and the target GPU for `$nvidia` / `$nvidia(auto)`, injects short cited doc slices, and ships advisor rule packs behind `/nv-profile` (advice) and `/nv-verify` (hard checks). The keyless part (sync + lexical search) is P5 and does not need API keys; hybrid vector search needs embeddings (Token Factory key).

**API sketch.**
```rust
pub enum NvidiaTopic { CudaLowLevel, Ptx, TensorRtLlm, Triton, Nccl, OpenShell, NemoClaw, Nemotron, HardwareArch }
pub enum SourceKind { LlmsTxt, MarkdownSuffix, DocsMcp { endpoint: Url }, HtmlScrape }
pub struct DocSource { pub id: SourceId, pub root: Url, pub kind: SourceKind, pub topic: NvidiaTopic, pub licence_note: String }
pub struct DocChunk { pub id: ChunkId, pub source: SourceId, pub url: Url, pub anchor: Option<String>, pub heading_path: Vec<String>, pub text: String, pub signatures: Vec<ApiSignature>, pub hw_scope: HardwareScope, pub etag: Option<String>, pub fetched: Rfc3339 }
pub struct SyncBudget { pub max_requests: u32, pub max_bytes: u64, pub concurrency: u8 }
pub struct SyncReport { pub fetched: u32, pub not_modified: u32, pub failed: Vec<(Url, String)>, pub chunks_added: u32, pub chunks_updated: u32 }

pub struct ComputeCapability { pub major: u8, pub minor: u8 }
pub enum GpuArch { Turing, Ampere, Ada, Hopper, Blackwell, Unknown }
pub struct GpuTarget { pub cc: ComputeCapability, pub arch: GpuArch, pub name: String }
pub enum NvidiaStack { CudaNative, Cutlass, TensorRtLlm, Triton, OpenShell, NemoClaw, RustCuda }
pub enum ProfileMode { Auto, Manual(Vec<ProfileFlag>), Off }
pub struct WorkspaceProbe { pub files: Vec<ProbeFile> }                       // built by the caller from ignore-aware walk; heads only
pub trait GpuProbe: Send + Sync { fn local(&self) -> impl Future<Output = Result<Option<GpuTarget>, ProbeError>> + Send; }
pub struct NvidiaProfile { pub local: Option<GpuTarget>, pub deploy_target: GpuTarget, pub stacks: Vec<NvidiaStack>, pub active_rules: Vec<RuleId>, pub doc_slices: Vec<DocSliceRef>, pub detect_ms: u32 }

pub struct NvidiaKnowledge<V: VectorStore, R: RegistryIndex, G: GpuProbe> { /* stores, transport, probe */ }
impl<V: VectorStore, R: RegistryIndex, G: GpuProbe> NvidiaKnowledge<V, R, G> {
    pub async fn sync(&self, topics: &[NvidiaTopic], budget: SyncBudget) -> Result<SyncReport, DocsError>;
    pub async fn query(&self, topic: Option<NvidiaTopic>, q: &str, k: TopK) -> Result<Vec<DocHit>, DocsError>;
    pub async fn auto_profile<D: DecisionEngine, C: SchemaCatalog>(&self, probe: &WorkspaceProbe, prompt: &str, mode: ProfileMode, engine: &D, catalog: &C) -> Result<NvidiaProfile, DocsError>;
}
pub struct AdvisorFinding { pub rule: RuleId, pub severity: Severity, pub loc: SourceSpan, pub message: String, pub doc: Option<DocSliceRef>, pub basis: Basis }   // Basis::{Heuristic, Deterministic}
pub fn nv_profile(profile: &NvidiaProfile, source: &SourceFile) -> Result<Vec<AdvisorFinding>, DocsError>;   // advice: coalescing, bank conflicts, divergence, pinned memory, streams
pub fn nv_verify(profile: &NvidiaProfile, source: &SourceFile) -> Result<Vec<AdvisorFinding>, DocsError>;    // hard checks: cudaGetLastError after launch, grid/block limits, shared-mem limits
pub enum DocsError { Fetch(String), Parse(String), Index(String), Probe(ProbeError), UnknownTopic, Licence(String), Budget }
```
Sources (VERIFIED, plan item 9): `llms.txt` at `docs.nvidia.com`, `docs.nvidia.com/cuda`, `developer.nvidia.com`, `.../openshell`, `.../nemoclaw`; appending `.md` to a page URL yields Markdown; docs MCP server at `https://docs.nvidia.com/nemoclaw/_mcp/server`. MCP availability for the other sites is LIKELY only.

**Dependencies.** Internal: `polaris-sdk`, `polaris-storage-registry`, `polaris-storage-vector`, `polaris-code-intel`, `polaris-ratelimit`. External (workspace.toml): `reqwest = 0.13.5`, `htmd = 0.5.6`, `pulldown-cmark = 0.13.4`, `url = 2.5.8`. Proposed additions: `serde = 1.0.229`, `tokio = 1.54.0`, `thiserror = 2.0.21`, `blake3 = 1.8.7` (chunk ids; or via storage-cas). The docs-MCP path is injected through a small `DocsTransport` trait implemented at the composition root with `polaris-ext-mcp` (no direct dependency).

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: llms.txt parsing, incremental sync, Markdown chunking, stack detection, GPU target resolution, `nv_verify` launch checks, `nv_profile` rules.
- [ ] Implement the `llms.txt` index parser and a polite incremental fetcher (conditional GET with ETag/Last-Modified, `.md` suffix preferred, concurrency and request budget, robots/terms respected); this is keyless.
- [ ] Implement the Markdown chunker (heading-path aware, keeps code blocks whole, extracts API signatures such as C/C++/PTX declarations) with `pulldown-cmark`.
- [ ] Persist chunks: FTS5 lexical index via `storage-registry` (keyless), vectors via `storage-vector` when an embedding provider is available (BLOCKED-ON-ACCESS: Token Factory key for Qwen3-Embedding-8B; lexical-only mode ships first).
- [ ] Implement `query` as hybrid (BM25 + vector when present) with provenance; return cited slices, never uncited summaries.
- [ ] Implement deterministic stack detection (file extensions `.cu .cuh .ptx`, `__global__`, `<<<`, `#include <cuda_runtime.h>`, Cargo deps `cudarc`/`cust`, Python `tensorrt_llm`/`triton`/`nemo`/`torch.cuda`, `nvcr.io/nvidia/` images, OpenShell policy YAML) and cache by workspace-marker fingerprint.
- [ ] Implement `GpuProbe` contract; implementation lives at the composition root (`nvidia-smi --query-gpu=name,compute_cap` via `PlatformOps`; field availability depends on driver version, LIKELY, confirm). Local machine is GTX 1650, sm_75 (VERIFIED, plan item 10).
- [ ] Model `local` vs `deploy_target` separately: Hopper features (e.g. TMA, WGMMA, distributed shared memory) cannot run or be benchmarked on sm_75; profile must say "unverifiable locally" and the proof run needs a Nebius GPU VM (BLOCKED-ON-ACCESS: AI Cloud GPU quota).
- [ ] Implement S1 topic scoring for the prompt (Choice multi-label over `NvidiaTopic`, schema `nv.stack`) with a deterministic keyword fallback.
- [ ] Implement advisor rule packs as data + tree-sitter-cuda queries via `code-intel`: missing `cudaGetLastError` after launch, uncoalesced strided global access (affine index in `threadIdx`), shared-memory stride patterns, `__syncthreads` under thread-dependent branches, grid/block bound checks against per-arch limits taken from the cited CUDA docs tables. Every finding labels `Heuristic` vs `Deterministic`.
- [ ] Decide the offline seed: built by `xtask` as a release artifact (not in git); gate on a licence/terms review of NVIDIA docs redistribution (decision gate: if unclear, ship fetch-on-first-run only).
- [ ] Wire `$nvidia-docs(sync)`, `$nvidia(...)`, `/nv-profile`, `/nv-verify` to these APIs at the composition root (other groups own the directive grammar).

**Tests.**

| Category | Cases |
|---|---|
| unit | llms.txt parsing edge cases; chunker keeps code blocks intact; compute-capability to arch mapping; per-arch limits table lookups |
| proptest | chunker: concatenated chunk text covers the source (no loss), chunk size bounded; stack detection is order-independent |
| fuzz | llms.txt and Markdown chunker on arbitrary bytes (WSL2 nightly) |
| contract | wiremock NVIDIA-like site (synthetic llms.txt, `.md` pages, 304 responses); live fetch smoke SKIPPED unless network allowed |
| golden | insta snapshots of chunk output for fixture pages; advisor findings on a seeded-bug CUDA corpus (known bank-conflict, divergent sync, unchecked launch) with precision/recall recorded |
| smoke | sync one small topic, query it offline, produce a profile for a fixture CUDA repo |
| load | sync of ~10k pages with a request budget: wall time, bytes, RSS (targets to measure); query p99 over the index |
| scalability | 1k / 10k / 100k-file repos for stack detection (marker fingerprint cache) |
| stress | interrupted sync resumes; server returns 429/5xx storms; corrupted index rebuilds |
| security | injection in fetched docs (docs are untrusted); SSRF/redirect guard reused from `providers` contract; licence note recorded per source |
| fixtures | `tests/fixtures/nvidia/` (synthetic pages, sample `.cu`, Cargo/pyproject/Dockerfile markers) with provenance |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| llms.txt incremental sync | async_function | `NvidiaKnowledge::sync` | `crates/polaris-nvidia-docs/src/sync.rs` | `sync_fetches_listed_pages`, `sync_not_modified_skips_reindex`, `sync_respects_request_budget`, `sync_failed_page_reported_not_fatal`, `sync_prefers_md_suffix` |
| Markdown chunking | function | `chunk_markdown` | `crates/polaris-nvidia-docs/src/chunk.rs` | `chunk_keeps_code_block_whole`, `chunk_records_heading_path`, `chunk_extracts_signatures`, `chunk_size_bounded`, `chunk_covers_source_without_loss` |
| NVIDIA auto profile | async_function | `NvidiaKnowledge::auto_profile` | `crates/polaris-nvidia-docs/src/profile.rs` | `auto_detects_cuda_repo`, `auto_separates_local_sm75_from_target`, `manual_flags_override_detection`, `off_mode_returns_empty_profile`, `s1_timeout_uses_keyword_fallback` |
| CUDA verify checks | function | `nv_verify` | `crates/polaris-nvidia-docs/src/verify.rs` | `flags_launch_without_error_check`, `flags_block_size_over_limit`, `flags_sync_in_divergent_branch`, `does_not_flag_checked_launch`, `findings_carry_basis_label` |

**Risks & open questions.**
- VERIFIED: local GPU is GTX 1650 sm_75; docs sources and the docs MCP URL as listed; the chatlog's Hopper code cannot be validated locally.
- LIKELY: static detection of bank conflicts and coalescing is heuristic; real confirmation needs profiling on a GPU (e.g. Nsight Compute), outside this crate. Findings say so.
- UNVERIFIED: NVIDIA docs terms for redistribution of an offline seed; robots/terms for crawl rate; `nvidia-smi` `compute_cap` field on this driver.
- UNVERIFIED: an `H100 SXM5 / 1.8 TB/s` style output in the chatlog was invented; any bandwidth number shown to users must come from a real run or a cited table.
- Open: keep S1 only for topic scoring; do not use S1 to author rules or hardware limits.

**Chatlog sources.** Msgs 4-7 (docs module, `polaris-nvidia-docs`, `$nvidia(auto)`, `$nvidia-docs(sync)`, `/nv-profile`, `/nv-verify`, hardware resolver, advisor packs, topic enum, offline seed, `NvidiaAutoProfile`), 17 (NvidiaDocsSemaphore), 21 (docs in research). Corrected: `sled`/RocksDB/DuckDB storage (replaced by FTS5 + LanceDB), scraper-per-site modules (use llms.txt and `.md`), hardcoded `sm_90a` default, `todo!()` bodies, `__syncthreads` rule stated absolutely.

---

### `polaris-browser` — host-side browser contract: actions, a11y compression, pHash visual diff

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-browser` | L2 | lib | — | P7 |

**Purpose.** Defines how the host drives the Chromium instance inside the sandbox via `polaris-guest` (the guest binary uses `chromiumoxide`; this crate holds the shared contract and host-side logic): browser actions, accessibility-tree compression into a small outline, console and network error capture, screenshot hashing and comparison. It supports `$browser`, `/screenshot`, `/browser goto|tree`, `/ui-diff` and the visual verification loop.

**API sketch.**
```rust
pub struct ElementRef(u32);                                   // valid only for the snapshot that produced it
pub struct SnapshotId(u64);
pub enum BrowserAction { Goto(Url), Click(ElementRef), Type { target: ElementRef, text: String, submit: bool }, Scroll { dy: i32 }, Press(KeyName), WaitFor(Condition), Screenshot { full_page: bool } }
pub struct RawAxNode { pub id: u32, pub role: String, pub name: String, pub value: Option<String>, pub states: Vec<String>, pub children: Vec<u32> }
pub struct RawAxTree { pub nodes: Vec<RawAxNode>, pub root: u32 }
pub struct OutlineBudget { pub max_tokens_approx: u32, pub keep_roles: Vec<String> }
pub struct A11yOutline { pub snapshot: SnapshotId, pub text: String, pub node_count: u32, pub truncated: bool, pub approx_tokens: u32, pub trust: Trust }
pub fn compress_a11y(tree: &RawAxTree, snapshot: SnapshotId, budget: &OutlineBudget) -> Result<A11yOutline, BrowserError>;

pub struct Screenshot { pub png: Bytes, pub width: u32, pub height: u32 }
pub struct PHash(u64);
pub struct DiffConfig { pub global_threshold: u32, pub grid: (u8, u8), pub tile_threshold: u32, pub blank_variance_max: f32 }
pub enum VisualVerdict { Same, Similar, Different, Blank }
pub struct VisualDiff { pub verdict: VisualVerdict, pub global_distance: u32, pub changed_tiles: Vec<TileRect> }
pub fn perceptual_hash(shot: &Screenshot) -> Result<PHash, BrowserError>;
pub fn visual_diff(a: &Screenshot, b: &Screenshot, cfg: &DiffConfig) -> Result<VisualDiff, BrowserError>;
pub fn is_blank(shot: &Screenshot, cfg: &DiffConfig) -> Result<bool, BrowserError>;

pub struct ConsoleEvent { pub level: ConsoleLevel, pub text: String, pub url: Option<Url> }
pub struct NetworkFailure { pub url: Url, pub status: Option<u16>, pub kind: FailureKind }
pub trait GuestChannel: Send + Sync { fn apply(&self, a: &BrowserAction) -> impl Future<Output = Result<ActionResult, BrowserError>> + Send; fn ax_tree(&self) -> impl Future<Output = Result<RawAxTree, BrowserError>> + Send; fn events(&self) -> impl Future<Output = Result<Vec<GuestEvent>, BrowserError>> + Send; }
pub struct BrowserSession<G: GuestChannel> { /* channel, current snapshot, policy */ }
impl<G: GuestChannel> BrowserSession<G> { pub async fn act(&mut self, a: BrowserAction) -> Result<StepReport, BrowserError>; pub async fn outline(&mut self, b: &OutlineBudget) -> Result<A11yOutline, BrowserError>; pub async fn verify_visual(&mut self, baseline: &Screenshot, cfg: &DiffConfig) -> Result<VisualReport, BrowserError>; }
pub enum BrowserError { StaleElement(ElementRef), Guest(String), Decode(String), Blocked(Url), Timeout, TooLarge }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`. External (workspace.toml): `image = 0.25.10` (png, jpeg, webp), `image_hasher = 3.1.1`. Proposed additions: `bytes = 1.12.1`, `url = 2.5.8`, `thiserror = 2.0.21`, `serde = 1.0.229`. `chromiumoxide = 0.9.1` is used by `apps/polaris-guest`, not here.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: a11y compression, element-ref staleness, pHash, tile diff, blank detection, action policy.
- [ ] Define the `GuestService` browser messages with `polaris-proto` (raw AX tree, actions, events); the guest populates the tree via the CDP Accessibility domain (LIKELY `getFullAXTree`; confirm in `chromiumoxide` 0.9.1). The chatlog's `window.__polaris_a11y_dump` JS function does not exist and is dropped.
- [ ] Implement `compress_a11y`: priority order interactive > headings/landmarks > labelled text; stable `ElementRef`s per snapshot; deterministic output; mark truncation; strip control characters.
- [ ] Implement stale-ref protection: refs from an old `SnapshotId` fail with `StaleElement`.
- [ ] Implement `perceptual_hash` via `image_hasher` (pHash, not xxh3; xxh3 changes entirely on one pixel) and the global-distance verdict; add a tile grid of per-tile hashes to localize changes since one 64-bit hash cannot say where.
- [ ] Implement blank-page detection (low pixel variance / single dominant colour) plus console-error correlation for white-screen cases.
- [ ] Define deterministic rendering requirements for baselines (fixed viewport and scale, animations off, font set fixed in the sandbox image) and record them with each baseline.
- [ ] Implement the action policy: domain allowlist from the sandbox network policy, `evaluate_js` not exposed to S2 by default, download and file-chooser actions denied.
- [ ] Implement the visual verification loop: screenshot -> hash/diff -> if `Different`/`Blank`, bundle screenshot + console errors for S2, optionally the vision model `Nemotron-3.5-Super-VL` as arbiter for ambiguous diffs (BLOCKED-ON-ACCESS: Token Factory key).
- [ ] Measure outline size vs raw DOM/AX tree on 10 fixture pages and report the real ratio (the "80k to 600 tokens" claim is unverified).

**Tests.**

| Category | Cases |
|---|---|
| unit | stale ref error; outline determinism; truncation flag; blank detection on solid vs real screenshots |
| proptest | `compress_a11y` never exceeds `max_tokens_approx` (approximate counter), keeps all interactive nodes when budget allows, output independent of input node order; pHash distance symmetric and zero for identical images; small noise stays within `Similar` |
| fuzz | `compress_a11y` on arbitrary AX trees (cycles, huge depth, huge fan-out) and PNG decoding of arbitrary bytes under dimension limits (WSL2 nightly) |
| golden | insta snapshots of outlines for fixture trees; hash stability across platforms |
| smoke | guest-less: fixture AX tree + fixture PNGs end-to-end; with guest (Linux CI leg / WSL2): navigate a local static page, snapshot, click, re-snapshot |
| load | 100 snapshots/s of 5k-node trees; 4k x 4k image hash time |
| stress | adversarial tree (100k nodes, depth 10k), decompression-bomb PNG |
| security | page text/aria-labels containing instructions are delivered as `Untrusted` data; ANSI/zero-width stripped; navigation to disallowed host blocked; image dimension caps |
| fixtures | `tests/fixtures/browser/` (synthetic AX trees, small PNGs generated at test time) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| A11y outline compression | function | `compress_a11y` | `crates/polaris-browser/src/a11y.rs` | `keeps_interactive_nodes_within_budget`, `truncates_and_flags_over_budget`, `output_is_deterministic`, `refs_unique_within_snapshot`, `strips_control_characters` |
| Element ref staleness | method | `BrowserSession::act` | `crates/polaris-browser/src/session.rs` | `click_with_current_ref_succeeds`, `click_with_stale_ref_errors`, `goto_blocked_host_denied`, `evaluate_js_not_exposed_by_default` |
| Perceptual hash and diff | function | `visual_diff` | `crates/polaris-browser/src/visual.rs` | `identical_images_same`, `small_noise_is_similar`, `moved_element_is_different_with_tile`, `blank_screenshot_is_blank`, `oversize_image_rejected` |
| Visual verification loop | async_function | `BrowserSession::verify_visual` | `crates/polaris-browser/src/verify.rs` | `matching_baseline_passes`, `blank_with_console_error_reports_both`, `different_page_returns_bundle_for_s2`, `baseline_viewport_mismatch_is_error` |

**Risks & open questions.**
- VERIFIED: xxh3 is not perceptual; `image_hasher` provides pHash (plan item 12).
- LIKELY: pHash is sensitive to font/AA differences between environments; baselines must come from the same sandbox image.
- UNVERIFIED: tokens saved by the outline; chromiumoxide AX API surface; headless Chromium + Xvfb inside the OpenShell vm driver (S0-2, S0-12).
- GUESSING: pHash thresholds; calibrate on fixture pairs.

**Chatlog sources.** Msg 25 (interactive browser, token-squeezer, A11y tree, pHash visual loop, `/screenshot`, `/browser`, `/ui-diff`, Xvfb screenshots), 33 (hacker uses microVM), 29-35 context. Corrected: xxH3-as-perceptual, invented JS dump function, "Firecracker <10 ms" (replaced by OpenShell `vm` driver, D2), `with_head()` headless misuse, token-ratio claims.

---

### `polaris-hacker` — zero-token adversary: static rules, boundary inputs, sandboxed dynamic stress

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-hacker` | L3 | lib | — | P7 |

**Purpose.** Implements `/hack` and `/redteam`: it tries to break a patch without spending model tokens. Three stages: static AST/DFA detectors, deterministic boundary-input generation (Needle 3 optional), and dynamic stress runs inside a sandbox obtained through the sdk `SandboxBackend` trait. "Zero-token" is enforced by construction: the crate takes no `ChatModel` and has no dependency that can reach one; `tokens_used = 0` is derived from a measured counter, not asserted. Escalation to S2 happens in the caller, only for findings whose confidence is `Reproduced`.

**API sketch.**
```rust
pub enum AuditDepth { Static, StaticPlusBoundary, Full }
pub struct StressPlan { pub threads: u16, pub iterations: u32, pub sanitizer: Sanitizer, pub wall_timeout: Duration, pub mem_cap_mib: u32, pub pids_cap: u32 }   // Sanitizer::{None, Tsan, Asan, Miri, Loom}
pub struct AuditRequest { pub diff: ParsedDiff, pub depth: AuditDepth, pub plan: StressPlan, pub seed: Seed }
pub enum FindingClass { BlockingInAsync, QuadraticHotLoop, UnsafeMisuse, UncheckedPanicPath, PathTraversal, CudaBarrierDivergence, CudaBankConflict, CudaUncheckedError, DataRace, Deadlock, PanicReproduced, MemoryExhaustion }
pub enum Confidence { StaticHeuristic, StaticDeterministic, Observed, Reproduced }
pub struct Finding { pub id: FindingId, pub class: FindingClass, pub severity: Severity, pub cwe: Option<Cwe>, pub loc: SourceSpan, pub evidence: Evidence, pub confidence: Confidence, pub poc: Option<PocRef> }
pub struct DynamicRun { pub command: String, pub seed: Seed, pub exit: ExitStatus, pub sanitizer: Sanitizer, pub observed: Vec<Observation>, pub duration: Duration }
pub struct AuditReport { pub findings: Vec<Finding>, pub runs: Vec<DynamicRun>, pub tokens_used: TokenCount, pub duration: Duration, pub coverage_note: String }
pub struct BoundaryGen;
impl BoundaryGen { pub fn values_for(shape: &TypeShape, seed: Seed) -> impl Iterator<Item = BoundaryValue> + '_; }   // 0, -1, MIN, MAX, NaN, +-inf, empty, huge, deep nesting, path traversal, unicode edge cases
pub fn static_scan(diff: &ParsedDiff, rules: &RuleSet) -> Result<Vec<Finding>, HackerError>;
pub async fn audit<B: SandboxBackend>(req: &AuditRequest, backend: &B) -> Result<AuditReport, HackerError>;
pub enum HackerError { Parse(String), Sandbox(String), Unsupported(Language), Budget, ToolchainMissing(String) }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-code-intel`, `polaris-security-automata`. External (workspace.toml): `proptest = 1.12.0` (strategy-based input generation). Proposed additions: `thiserror = 2.0.21`, `serde = 1.0.229`. Overlap with `polaris-security-rules` (CWE rule packs, L3 sibling) is handled by an sdk `RuleEngine` trait injected at the composition root instead of a sibling edge.

**TODO.**
- [ ] Write JSON descriptors (prompt.md section 6.3) for: static async-blocking detector, boundary generator, dynamic stress runner, zero-token invariant, finding confidence rules.
- [ ] Implement static detectors over tree-sitter ASTs (DFA only as a prefilter): blocking calls inside `async fn` (`std::thread::sleep`, blocking fs/net), nested loops over the same collection in hot paths, `unsafe` blocks without a SAFETY note, `unwrap`/`expect`/indexing per prompt.md, `__syncthreads` under thread-dependent control flow, strided global access, launches without error checks (shared logic with `nvidia-docs` rules via the injected `RuleEngine`).
- [ ] Implement `BoundaryGen`: deterministic, seeded, covers integer/float extremes, NaN/inf, empty and huge strings/buffers, deeply nested and cyclic-looking JSON, path-traversal strings; Needle-generated inputs are an optional add-on measured against it.
- [ ] Implement the stress runner over `SandboxBackend`: apply patch, build with sanitizer, run N iterations with thread pressure, capture seeds and exit data, produce a reproducible command. Races are detected with ThreadSanitizer (nightly + `-Zsanitizer=thread` on Linux/WSL2), `loom` models where the target has them, Miri for UB; output-string matching for "data race" is not detection and is dropped.
- [ ] Classify results honestly: deadlock = "no progress past wall timeout" -> `Observed` with thread dump when available; a clean run yields "no failure observed in N runs under <sanitizer>", never "code resilient".
- [ ] Enforce sandbox limits (pids, memory, no host mounts, network denied unless the plan opens crates.io) so a generated fork bomb or memory hog is contained; document which OpenShell driver provides which limits (docker driver vs `vm` driver, nested KVM depends on S0-2).
- [ ] Implement the zero-token invariant: `tokens_used` comes from a `TokenMeter` the crate cannot increment; `check-layers` asserts no dependency path to any `ChatModel` implementation.
- [ ] Emit PoC artifacts (test file + command + seed) into CAS for `Reproduced` findings and return refs.
- [ ] Build a seeded-bug corpus (blocking sleep in async, racy counter, off-by-one parser, divergent sync) for precision/recall measurement; hostile samples generated at test time.
- [ ] Wire `/hack`, `/redteam` and the rpc `ExecuteZeroTokenAudit` (owned by other groups) to `audit`; the sandbox needs a toolchain image with nightly + TSan (BLOCKED-ON-ACCESS: WSL2 Ubuntu + OpenShell not yet installed; Linux x86_64 only).

**Tests.**

| Category | Cases |
|---|---|
| unit | each detector on positive/negative snippets; finding confidence rules; deterministic with seed |
| proptest | `BoundaryGen` always includes min/max/zero/NaN per numeric shape, is deterministic per seed, bounded in count; static scan never panics on arbitrary source text |
| fuzz | `static_scan` on arbitrary bytes / truncated files for Rust, C++, CUDA, Python (WSL2 nightly) |
| kani | bounded checks on boundary-value arithmetic (no overflow in size/count computations) |
| golden | seeded-bug corpus: detection precision/recall per class recorded (targets, not claims) |
| smoke | fixture crate with a known TSan-detectable race is reported as `DataRace` with `Observed` (SKIPPED if no Linux sandbox); clean crate yields no finding and an honest coverage note |
| load | scan a 10k-line diff; 100 parallel static scans |
| scalability | diffs of 1 / 100 / 1000 files; stress plan threads 4 / 32 / 128 |
| stress | fork bomb, memory hog, infinite loop, output flood inside the sandbox: contained, reported, host unaffected, sandbox torn down by the reaper |
| security | PoC execution cannot reach host FS or network; adversarial source (comments containing "SIGSEGV", "data race") cannot fake a finding; secrets in PoC output redacted |
| fixtures | seeded-bug fixture crates (generated), CUDA snippets (`tests/fixtures/hacker/`) |

**Test-first descriptors.**

| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Async blocking detector | function | `static_scan` | `crates/polaris-hacker/src/static_scan.rs` | `flags_thread_sleep_in_async_fn`, `ignores_thread_sleep_in_sync_fn`, `ignores_mention_in_comment_or_string`, `flags_blocking_fs_call_in_async`, `findings_have_spans` |
| Boundary input generation | struct | `BoundaryGen` | `crates/polaris-hacker/src/boundary.rs` | `ints_include_min_max_zero_minus_one`, `floats_include_nan_inf_neg_zero`, `strings_include_empty_huge_traversal`, `same_seed_same_values`, `count_is_bounded` |
| Dynamic stress runner | async_function | `audit` | `crates/polaris-hacker/src/dynamic.rs` | `tsan_race_reported_as_observed_with_seed`, `clean_run_reports_no_failure_observed_not_resilient`, `deadlock_timeout_reported_as_suspected`, `sandbox_error_is_typed_error`, `fork_bomb_contained` |
| Zero-token invariant | struct | `AuditReport` | `crates/polaris-hacker/src/report.rs` | `tokens_used_is_zero_from_meter`, `crate_has_no_chatmodel_dependency`, `reproduced_finding_includes_poc_ref`, `static_only_finding_not_marked_reproduced` |

**Risks & open questions.**
- VERIFIED: `catch_unwind` cannot catch segfaults; string-matching "data race" is not race detection; the chatlog's "PASSED, code resilient" text is overclaiming (plan item 12).
- LIKELY: TSan/Miri require nightly and Linux; the Windows host cannot run the dynamic stage, only the WSL2/sandbox can (nightly toolchain exists locally; WSL2 Ubuntu does not yet).
- UNVERIFIED: OpenShell `vm` driver availability (S0-2) and its resource limits; docker driver pids/memory limits must be applied through policy.
- UNVERIFIED: target projects' dependencies need network at build time; policy must open only what is needed.
- GUESSING: static CUDA bank-conflict/coalescing rules will have noticeable false positives; hence `StaticHeuristic` labelling and a measured precision target.
- Open: a dedicated spike for TSan inside the chosen sandbox image (id to assign in PHASES.md).

**Chatlog sources.** Msgs 32-35 (hacker agent, attack vectors A concurrency/chaos, B security boundary probing, C bottleneck hunter, zero-token engine, `/hack`, `/redteam`, Needle chaos inputs, tokens_used = 0, `ExecuteZeroTokenAudit`), 11 (CWE patterns shared with the security rules crate). Corrected: Firecracker -> OpenShell `vm`/docker sandbox (D2), `contains("data race")`, `SIGSEGV` string checks, `unwrap()` use in the engine, fixed 32-thread run presented as proof, `DFA::builder().build_many(...).unwrap()`.

---

### G4 notes for the merger (cross-crate decisions this part depends on)

- **sdk additions (PROPOSED):** `SchemaCatalog`, `Consumer`, `Trust`, `TopK`, `Ranked<T>`, `Question`/`Answer`/`Decision` family, `ProcessLauncher`/PlatformOps-based spawn, `RuleEngine`, `GpuProbe` may live in sdk or in `nvidia-docs`; not yet in workspace.toml `api` lists.
- **Facade features:** `polaris-system-one` must gate `engine` and `nano` behind features so the kernel never links ORT.
- **workspace.toml ext gaps (PROPOSED):** `thiserror` for all G4 libs; `serde`/`serde_json` where listed above; `blake3` for client (strict integrity), schemas (optional), skills; `jsonschema` for nano and ext-mcp; a YAML crate for skills (serde-saphyr already pinned); `bytes`, `url` for browser/mcp; a zip reader for ingest container checks; `tokio-util` for sift/client.
- **Layer edges avoided on purpose:** sift, nvidia-docs, skills reach S1 only through sdk traits; sift does not depend on nvidia-docs or storage-vector (adapters at the composition root); router does not depend on models (returns `Effort`).
- **Spikes referenced:** S0-4 (Decide ONNX + parity + latency), S0-5 (Token Factory tools/caching/reasoning), S0-6 (Needle 3 Windows/WSL2), S0-2 (nested KVM), S0-12 (guest build + handshake), S0-13 (Contree MCP), plus a proposed TSan-in-sandbox spike.

---

## 13.5 Group G5 — Interfaces, applications, tests & tooling

Conventions used in this group. Versions are copied from `@prime/tools/workspace.toml` (the single source of truth); where the PLAN core (§0–§12) text differs, the TOML value is used and the difference is flagged in the crate's risks. Labels: VERIFIED = checked in this session against a file or command output; LIKELY = consistent with context, not verified; UNVERIFIED = needs a spike or live access. No test named below has been run: every test row is a plan. Budgets are targets to measure, never facts. `BLOCKED-ON-ACCESS (...)` marks work that needs a key, a gateway or a VM that does not exist yet (D9). `polaris.v1` is the proto package; tonic's generated Rust service methods are snake_case (`attach_session`), while RPC names in `.proto` stay PascalCase.

Cross-cutting design source (VERIFIED, read this session): `dist/demo-design-preview-slop` supplies DESIGN ONLY (tokens, logo cell art, composer/completion/keymap behaviour, the `--dump WxH` idea, the web welcome page). No code is reused: the demo used unsafe Win32 input code and god files. Its own README states that inference, tools, history and metrics are simulated.

---

### `polaris-rpc` — tonic server/client for the kernel service: named pipe (token-authenticated) locally, TCP + mTLS remotely

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-rpc` | L5 | lib | none | P9 |

**Purpose.** Carries the sdk `KernelApi` over gRPC so that the TUI, the Slint desktop, the web server and a remote laptop can share one kernel (`polarisd`). It owns the transports (Windows named pipe, Unix domain socket on the Linux leg, TCP + mTLS), the per-connection authentication (Biscuit token on every transport) and the server/client adapters. It never depends on the kernel crate: it is generic over `KernelApi` and the composition root wires it. Through P1-P8 the CLI embeds the kernel in-process; this crate only becomes load-bearing in P9 (daemon, desktop, web).

**API sketch.**
```rust
// Transport selection is an enum (ADR: enum dispatch for runtime-selected variants).
pub enum Transport {
    /// Windows: \\.\pipe\polaris-<random>. Linux/macOS leg: Unix domain socket path.
    Local { endpoint: LocalEndpoint },
    /// Remote headless daemon. mTLS is mandatory off loopback.
    Tcp { addr: std::net::SocketAddr, tls: Option<TlsServerConfig> },
}
pub struct TlsServerConfig { /* cert chain, key (secrecy), client CA roots */ }
pub struct TlsClientConfig { /* roots, optional client identity */ }

pub async fn serve<K: KernelApi>(
    kernel: K,
    transport: Transport,
    auth: Authenticator,                       // wraps polaris-security-capability
    shutdown: impl std::future::Future<Output = ()> + Send,
) -> Result<(), RpcError>;

pub async fn connect(transport: ClientTransport, credential: Credential) -> Result<KernelClient, RpcError>;

impl KernelClient {
    pub async fn attach_session(&self, resume: Option<ResumeToken>) -> Result<SessionStream, RpcError>;
    pub async fn execute_command(&self, input: &str) -> Result<CommandOutcome, RpcError>;
    pub fn stream_events(&self, from_seq: u64) -> impl futures::Stream<Item = Result<WireEvent, RpcError>>;
    pub async fn attach_terminal(&self, req: TerminalSpec) -> Result<TerminalChannel, RpcError>;
    pub async fn execute_zero_token_audit(&self, req: AuditRequest) -> Result<AuditReport, RpcError>;
}

#[derive(Debug, thiserror::Error)]
pub enum RpcError {
    #[error("unauthenticated")] Unauthenticated,
    #[error("transport: {0}")] Transport(String),
    // ...
}
```

Corrected `polaris.v1` outline (the `.proto` files are owned by `polaris-proto`; this block is the contract this crate needs from them). It resolves the chatlog mismatch: the msg-35 diagram lists `StreamEvents / ExecuteCommand / AttachTerminal / TriggerZeroTokenHack`, while the msg-35 proto lists only `AttachSession / ExecuteZeroTokenAudit`. Resolution: all of them exist; `TriggerZeroTokenHack` is renamed `ExecuteZeroTokenAudit` (it matches the proto messages and `polaris-hacker`); `AttachSession` is the primary multiplexed bidi stream; `StreamEvents` is server-streaming with a resume cursor (the diagram called it bidirectional, which it need not be).
```proto
syntax = "proto3";
package polaris.v1;

service PolarisKernelService {
  // Primary bidi stream: commands in, typed events out (replaces the stringly-typed event_type + bytes payload).
  rpc AttachSession (stream SessionCommand) returns (stream SessionEvent);
  // Unary: one $directive / /command / prompt submission.
  rpc ExecuteCommand (ExecuteCommandRequest) returns (ExecuteCommandResponse);
  // Server-streaming event subscription for passive clients; resumable from a sequence number.
  rpc StreamEvents (StreamEventsRequest) returns (stream SessionEvent);
  // Full-duplex PTY (stdin/stdout/resize).
  rpc AttachTerminal (stream TerminalInput) returns (stream TerminalOutput);
  // Local adversarial audit; the response must carry tokens_spent == 0.
  rpc ExecuteZeroTokenAudit (AuditRequest) returns (AuditResponse);
}

service WorkerService {                 // polaris-worker, one process per mode
  rpc Health (HealthRequest) returns (HealthResponse);          // rss, loaded models, mode
  rpc Decide (DecideBatch) returns (DecideBatchResult);          // s1: Choice/Score/Noul batch
  rpc Ingest (IngestRequest) returns (stream IngestChunk);       // ingest: normalized Markdown chunks
  rpc Evict (EvictRequest) returns (EvictResponse);              // idle / pressure / force-before-build
  rpc Shutdown (ShutdownRequest) returns (ShutdownResponse);
}

service GuestService {                  // polaris-guest inside the sandbox
  rpc Handshake (HandshakeRequest) returns (HandshakeResponse);  // protocol_version first, else fail closed
  rpc Exec (ExecRequest) returns (stream ExecEvent);             // structured stdout/stderr/exit
  rpc ApplySync (stream SyncDelta) returns (SyncAck);            // blake3 manifest deltas
  rpc Screenshot (ScreenshotRequest) returns (ScreenshotResponse);
  rpc A11yTree (A11yRequest) returns (A11yResponse);
}

message SessionCommand { oneof body { string input = 1; Cancel cancel = 2; Ack ack = 3; Approval approval = 4; } }
message SessionEvent   { uint64 seq = 1; oneof body { /* typed events mirrored from sdk::Event */ } }
message StreamEventsRequest { uint64 from_seq = 1; string resume_token = 2; }
message AuditRequest  { string target_path = 1; string diff_content = 2; bool aggressive_fuzzing = 3; }
message AuditResponse { bool is_vulnerable = 1; repeated string detected_bottlenecks = 2;
                        optional string crash_trace = 3; uint64 execution_time_ms = 4;
                        uint32 tokens_spent = 5; /* invariant: always 0 */ }
message HandshakeRequest  { uint32 protocol_version = 1; string host_build = 2; }
message HandshakeResponse { uint32 protocol_version = 1; bool compatible = 2; string guest_build = 3; }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`, `polaris-security-capability`. External: `tonic = 0.14.6` (features transport, router, codegen, tls-aws-lc, tls-native-roots), `tokio = 1.54.0`, `rustls = 0.23.46` (aws_lc_rs), `tokio-rustls = 0.26.6`, `rcgen = 0.14.10` (test CA and self-signed certs), `hyper-util = 0.1.21`, `tower = 0.5.3`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: named-pipe connector/acceptor, auth interceptor, `AttachSession` handler, TLS config builder, event resume/replay.
- [ ] Replace the doc-only lib skeleton's planned-API docs with real modules once `polaris-proto` publishes the five-RPC `PolarisKernelService`; add a descriptor-set snapshot test so renames fail loudly.
- [ ] Implement the Windows named-pipe acceptor with `tokio::net::windows::named_pipe::ServerOptions` (API to confirm at implementation): `first_pipe_instance(true)` for the first instance, `reject_remote_clients(true)`, and create the NEXT server instance before handing the connected one to tonic.
- [ ] Implement the matching client connector (custom `tower` connector wrapped for hyper via `hyper-util`'s `TokioIo`; API to confirm) returning a tonic `Channel` over the pipe.
- [ ] Implement token authentication as a tonic interceptor on every transport (Biscuit in `authorization` metadata, verified through `polaris-security-capability`). Reject before any handler runs.
- [ ] Implement the Linux/macOS leg: UDS acceptor behind `cfg(unix)`; Windows never references `tokio::net::UnixStream`.
- [ ] Implement TCP + mTLS: server config requiring client certs, client config with pinned CA, `rcgen`-generated test PKI; install the aws-lc-rs provider; gate `cargo tree -i ring` empty.
- [ ] Implement `AttachSession`: bounded per-client channel; a slow client gets `RESOURCE_EXHAUSTED` and is detached (never unbounded buffering).
- [ ] Implement `StreamEvents { from_seq }`: replay from the `EventStore` trait, then live tail; `resume_token` rejoin after a dropped connection.
- [ ] Implement `ExecuteZeroTokenAudit` delegation and assert `tokens_spent == 0` at the adapter boundary.
- [ ] Add HTTP/2 keepalive, max message size and per-RPC deadline defaults; document them in the crate docs.
- [ ] BLOCKED-ON-ACCESS (Nebius AI Cloud VM + quota): remote-daemon smoke over real mTLS across the internet.

**Tests.**
| Category | Cases |
|---|---|
| unit | interceptor rejects missing / expired / out-of-scope token; `RpcError` to `tonic::Status` mapping is total; pipe-name generator never repeats |
| proptest | `ResumeToken` encode/decode round-trips; `from_seq` replay is gap-free and monotonic for any interleaving of appends |
| contract | prost descriptor-set snapshot (insta) for `polaris.v1`; `GuestService.Handshake` rejects a mismatched `protocol_version` |
| smoke | loopback server+client over a named pipe (Windows leg) and a UDS (Linux leg in WSL2): `execute_command("/status")` round-trips |
| security | a second local client without a token is refused; wrong-CA client refused; a token replayed with a different session fact refused; pipe uses `reject_remote_clients` |
| load | 1 kernel, N = 1 / 8 / 64 `StreamEvents` clients, event-rate sweep; p50/p99 delivery latency with hdrhistogram (targets to measure, e.g. p99 < 10 ms on loopback) |
| stress | connect/disconnect storm; half-open TCP; a slow consumer never makes server memory grow without bound |
| fuzz | WSL2 nightly: gRPC frame decode via prost on arbitrary bytes; metadata parser |
| spike | `polaris-spike-web` client floods exercise this crate in P9 |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Named Pipe Transport | module | `pipe` | `crates/polaris-rpc/src/pipe.rs` | `first_instance_owns_name`, `next_instance_created_before_handoff`, `remote_clients_rejected`, `second_server_same_name_fails`, `connect_roundtrips_one_message` |
| Token Authentication | function | `authenticate` | `crates/polaris-rpc/src/auth.rs` | `valid_token_passes`, `missing_token_unauthenticated`, `expired_token_rejected`, `attenuated_scope_denies_command`, `same_check_on_every_transport` |
| Attach Session Stream | async_function | `attach_session` | `crates/polaris-rpc/src/server.rs` | `commands_in_events_out`, `slow_client_detached`, `cancel_stops_stream`, `unauthenticated_never_reaches_kernel` |
| Event Resume | function | `replay_from` | `crates/polaris-rpc/src/resume.rs` | `replay_then_live_is_gapless`, `unknown_resume_token_rejected`, `from_seq_beyond_head_waits`, `duplicate_events_never_delivered` |

**Risks & open questions.**
- VERIFIED (plan + workspace.toml): the chatlog's "gRPC over UDS" cannot hold on Windows; tokio's `UnixStream` is unix-only. Named pipes need a custom connector and acceptor.
- VERIFIED (plan, workspace.toml responsibilities): the default named-pipe DACL admits other local users, so this crate authenticates every connection with a token. Setting a restrictive security descriptor needs Win32 calls and would use one of the declared unsafe exceptions (named-pipe security descriptor); decide at implementation. Until then: unguessable pipe name plus token, which bounds but does not remove the exposure (LIKELY).
- LIKELY: tonic 0.14 generated service traits use snake_case method names, so the chatlog's Rust method names `ExecuteZeroTokenAudit` / `AttachSession` are wrong. Confirm against generated code in P1.
- UNVERIFIED: whether OpenShell's libkrun `vm` driver exposes vsock to the guest. The chatlog's AF_VSOCK/Firecracker path is dropped (D2); the host-to-guest `GuestService` transport is decided by spike S0-12.
- UNVERIFIED: the chatlog's "a few milliseconds" remote latency and "TTFB < 2 ms" are targets, not facts.
- Open: `StreamEvents` behaviour when a client falls behind the retained log window (force a full `ViewState` snapshot?).
- Declared v2: "cellular-resilient" resume across 5G/Wi-Fi switches (msg 37 s2.4). v1 only ships the `from_seq` / `resume_token` mechanism.

**Chatlog / demo sources.** msg 35 (service outline, proto, server sketch, transports UDS / mTLS / vsock, Remote Headless Daemon / cloud bursting, bidi streaming); msg 37 (session resume tokens; WireGuard mesh = v2); msg 1 (IPC between frontends and kernel); msg 13 (Biscuit on commands). Corrected: `todo!()` and the hard-coded `execution_time_ms: 38` in the msg-35 server are rejected (prompt.md §6.1); `unwrap` removed; UDS becomes named pipe on Windows; diagram/proto naming reconciled above.

### `polaris-ui` — facade re-exporting the four UI children; documents design tokens and UX rules

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ui` | L6 | facade | none (parent of model, cli, gui, web) | P1 |

**Purpose.** A pure facade: it re-exports `polaris-ui-model`, `-cli`, `-gui` and `-web` and hosts the crate-level documentation of the design contract (tokens, UX rules, keymap pointer). It contains no logic. Children never depend on it (checked by `check-layers`); shared types live in `polaris-ui-model`. Apps depend on the specific child they need (`polaris-cli` on `polaris-ui-cli`, and so on), not on this facade, so the TUI binary does not link Slint or Leptos.

**API sketch.**
```rust
//! crate docs: design tokens table, UX rules, link to docs in each child.
pub use polaris_ui_model as model;
pub use polaris_ui_cli as cli;
pub use polaris_ui_gui as gui;
pub use polaris_ui_web as web;
// No functions, no types, no features (see risk on optional children).
```

**Dependencies.** Internal: `polaris-ui-model`, `polaris-ui-cli`, `polaris-ui-gui`, `polaris-ui-web`. External: none (workspace.toml lists `ext = []`).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: facade re-export surface, no-logic guard.
- [ ] Keep the skeleton's `//!` docs in sync with `workspace.toml` responsibilities; add the token table (copied from `polaris-ui-model::tokens` docs, not duplicated as code).
- [ ] Add a cross-crate test in `polaris-tests` asserting `polaris_ui::model::tokens::ACCENT` is the same item as `polaris_ui_model::tokens::ACCENT`.
- [ ] Add an `xtask check-layers` assertion that no child lists `polaris-ui` in its dependencies (already a rule; verify it covers this parent).
- [ ] Decide and record whether the facade stays unconditional (links Slint + Leptos for any consumer) or gets cargo features (`cli`, `gui`, `web`) with optional deps. That needs a `workspace.toml` edit first; do not edit Cargo.toml by hand.
- [ ] Verify `cargo doc -p polaris-ui --no-deps` renders all four children.

**Tests.**
| Category | Cases |
|---|---|
| unit | none beyond compile: the crate has no logic by design |
| contract | re-export identity test (above); `cargo public-api`-style listing diff is optional and only if the tool is installed (UNVERIFIED availability) |
| smoke | `cargo check -p polaris-ui` on Windows; Linux leg in WSL2 |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Facade Re-export Surface | module | `polaris_ui` | `crates/polaris-ui/src/lib.rs` | `model_reexport_resolves`, `cli_reexport_resolves`, `gui_reexport_resolves`, `web_reexport_resolves`, `facade_defines_no_items` |

**Risks & open questions.**
- VERIFIED (workspace.toml): the facade depends on all four children unconditionally, so a consumer of the facade compiles Slint and Leptos. Mitigated by apps depending on children directly. Open: features vs. unconditional.
- VERIFIED (plan): web build uses cargo-leptos with ssr/hydrate features and is excluded from `default-members`; a facade that depends on `polaris-ui-web` must not break `cargo check --workspace` on the default feature set. Check at P9.
- LIKELY: the facade is the right home for the token documentation because it is the only crate all UIs share as a parent; the machine-readable tokens live in `polaris-ui-model`.

**Chatlog / demo sources.** msg 0 and msg 1 (`polaris-ui` with `crates/{cli,gui,web}` as the workspace shape); the plan's rule "parent is a pure facade". Corrected: msg 1 describes `ui/crates/web` as a "WebSockets / JSON-RPC bridge"; in this plan the web child holds Leptos components and the bridge is `polaris-web` + `polaris-rpc`. A `model` child is added (shared reducers).

### `polaris-ui-model` — shared view models, event reducers, composer/completion logic and design tokens for every frontend

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ui/crates/model` | L6 | lib | `polaris-ui` | P1 |

**Purpose.** One frontend-agnostic state machine: `reduce(&mut ViewState, &Event)` turns kernel events into the state the TUI, Slint GUI and Leptos web UI render. It also owns the pure logic that the demo implemented inside its TUI but that every frontend needs: composer buffer with paste chips and prompt history, `$` / `/` / `@` completion ranking, the keymap as data (actions), the command/directive metadata table, and the design tokens and logo cell art. It does no I/O and no rendering, so it can be unit-tested, proptested and Miri-run. The command grammar itself belongs to `polaris-kernel-directives`; this crate holds only presentation metadata (labels, help, colour role).

**API sketch.**
```rust
pub struct ViewState {
    pub conversation: Vec<Entry>, pub tools: Vec<ToolActivity>, pub goal: Option<GoalLine>,
    pub hud: TokenWeather, pub sessions: Vec<SessionRow>, pub mode: Mode,      // Auto | Plan
    pub model: ModelId, pub effort: Effort,                                     // Low..Max (parser also accepts 1-5)
    pub settings: SessionSettings, pub status: StatusSnapshot, pub dag: Option<DagView>,
}
pub fn reduce(state: &mut ViewState, event: &Event) -> Result<(), ReduceError>;  // seq gaps are an error, not a panic

pub struct Composer { /* text, cursor, selection, undo, attachments, history cursor */ }
impl Composer {
    pub fn insert(&mut self, text: &str) -> Result<(), ComposerError>;           // 1 MiB cap; >5 lines or >500 chars => chip
    pub fn newline(&mut self);
    pub fn history_prev(&mut self) -> bool;  pub fn history_next(&mut self) -> bool; // only at first/last visual row (caller supplies row)
    pub fn submit(&mut self) -> Result<Submission, ComposerError>;               // trims trailing blank rows, expands chips
}
pub enum Attachment { Text { chars: usize, payload: String }, Image { index: u32, rgba_len: usize } }
pub const MAX_TEXT_BYTES: usize = 1 << 20;  pub const MAX_IMAGE_RGBA: usize = 64 << 20;  pub const MAX_ATTACHMENTS_TOTAL: usize = 128 << 20;

pub enum CompletionKind { Command, Directive, Mention }
pub trait PathIndex { fn query(&self, q: &str, limit: usize) -> Vec<PathHit>; }   // injected; async index lives in the app
pub fn complete(input: &str, cursor: usize, paths: &impl PathIndex) -> Vec<Suggestion>; // files capped at 100

pub enum Action { Submit, Newline, ToggleMode, EffortUp, EffortDown, InterruptOrExit, ToggleTools, OpenStatus, OpenHelp, /* ... */ }
pub fn keymap() -> &'static [KeyBinding];            // data table; frontends map native keys to KeyChord

pub mod tokens { /* colour roles, see table */ }
pub mod logo   { pub const WELCOME: LogoGrid; pub const COMPACT: LogoGrid; pub const EMBLEM: LogoGrid; }
```

Design tokens extracted from the demo (VERIFIED, read from `docs/ui-design.md`, `docs/color-policy.md`, `docs/branding/hybrid-palette.json`, `webui/styles.css`). Every colour is a role; frontends choose truecolor, ANSI or inherit.

| Role | Hex | Source |
|---|---|---|
| canvas | `#000000` | ui-design.md, palette.json |
| accent (NVIDIA-inspired, not an official token) | `#76B900` | ui-design.md |
| muted metadata | `#858585` | ui-design.md (web `--p-dim`; web `--p-muted` is `#969C94`) |
| auto mode / pink | `#F5C2E7` | ui-design.md |
| plan mode, tool titles, `@file` mentions | `#F9E2AF` (hover `#FFFDE6`) | ui-design.md, composer.md |
| command focus / hover | `#B6E35B` / `#DBF5B0` (web token-command `#94CD40`) | composer.md, styles.css |
| dollar settings, goal line | `#89B4FA` (hover `#B4D2FF`) | prompt-settings.md |
| error title/body, regular weight | `#AC4242` (ANSI fallback Red) | ui-design.md |
| warning amber | `#E5B454` (web `--p-warning`; TUI value not stated in docs) | styles.css |
| sent-prompt surface | `#373737` | ui-design.md |
| "To latest" | bg `#262626`, hover `#373737` | ui-design.md |
| empty-editor placeholder | `#525252`, text "What's on your mind for Polaris?" | prompt-settings.md |
| chart series | green, cyan, gold | ui-design.md |
| selection | 30% source-colour tint blended over the current background (white on black = `#464646`; gold `#4B4435`; green `#233800`; blue `#29364B`; on `#373737` white = `#6C6C6C`); ANSI/terminal: neutral index 237; no-color: bold | composer.md |
| logo | star `#24D046`, traces `#76B900`, mint highlight `#96D6AC`, upper/left frame `#B8B8B8`, lower/right + pins `#6B6B6B`, package surface `#1C1C1C` | header-logo.md |
| web surfaces | surface `#141414`, raised `#1C1C1C`, text `#E8E8E8`, green-light `#C0E58C`, line `#FFFFFF12`, radius 14px, speed 160ms | styles.css |
| fonts (web) | Space Grotesk (display), Inter (body), JetBrains Mono (mono); OFL-licensed files bundled in the demo | styles.css, assets/fonts |

Logo cell art (`·` = ordinary space; mint highlight at local (7,2) welcome / (5,0) compact; package occupies x=2..12,y=1..5 welcome and x=1..9,y=1..3 compact; ASCII fallback maps frame to `+ - |` and star to `*`). Welcome 15×7:
```
····│··│··│····
··┌─┴──┴──┴─┐··
··│····╷····│··
──┤··╶─✦─╴··├──
··│····╵····│··
··└─┬──┬──┬─┘··
····│··│··│····
```
Compact 11×5:
```
···│·│·│···
·┌─┴─┴─┴─┐·
─┤··─✦─··├─
·└─┬─┬─┬─┘·
···│·│·│···
```
Emblem fallback (below seven body rows): 11×1, a centered `─✦─` with four ordinary spaces on each side.

**Dependencies.** Internal: `polaris-sdk`. External: `serde = 1.0.229`. Likely additions to propose in `workspace.toml` (not yet there): `unicode-width = 0.2.2` and `unicode-segmentation = 1.13.3` for grapheme-correct cursor and wrap logic (both already pinned in the TOML for the TUI crate); decide whether text-width logic lives here or in `polaris-ui-cli`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: `reduce`, `Composer` (insert/paste chips/submit/history), `complete`, keymap table, token module, logo grids.
- [ ] Define `Mode`, `Effort` (parser accepts low/medium/high/xhigh/max and 1-5), `Mode` toggle, and per-model in-memory effort preference.
- [ ] Implement `reduce` for conversation, tool activity (collapsed gold title; global expand flag plus per-entry override), goal line (active time excludes pauses), session list and status.
- [ ] Implement `Composer`: CRLF/CR to LF, 1 MiB cap, chip for >5 logical lines or >500 chars labelled `[Pasted Content M chars]`, `[Image N]` chips, 64 MiB image and 128 MiB session caps, undo/redo, submit trims trailing blank rows but keeps interior blank paragraphs.
- [ ] Implement prompt history: Up/Down only at the first/last visual row, unsent draft restored after the newest entry; history is session-local.
- [ ] Implement completion: `/` and `$` at a token boundary (not inside URLs or paths), `@` file mentions with quoted form for spaces, ranking basename/path prefix, then substring, then fuzzy, deterministic lexicographic ties, max 100.
- [ ] Implement token classification for colouring (complete recognised commands green, incomplete/unknown neutral, `$` known names blue, mentions gold, bare domains underlined).
- [ ] Encode the keymap table (list in `polaris-cli`) and a test that the footer/help strings are generated from it, so they cannot drift.
- [ ] Add the token-weather view model (context load, cache hit %, burn rate, S1/S2 latency); keep-warm indicator hidden unless the capability flag from S0-5 is set.
- [ ] Add the command/directive metadata table and a drift test in `polaris-tests` against `polaris-kernel-directives` (same names, same arity).
- [ ] Add Quick Chips data (`$websearch`, `$nvidia`, `/hack`, `/undo`, Screenshots) marked v2/mobile; compiled but unused until the Slint/PWA mobile targets.
- [ ] Provide web-token export (`tokens::css_variables() -> String`) so `polaris-ui-web` generates its `:root` block from the same source.

**Tests.**
| Category | Cases |
|---|---|
| unit | reducer: each event type; seq gap returns `ReduceError`; effort clamp at endpoints; goal timer pauses |
| proptest | composer: any sequence of insert/newline/delete keeps cursor on a char boundary; submit(expand(chips)) preserves payload; reducer is deterministic for any event permutation that respects seq; completion never returns more than 100 files; history navigation never loses the draft |
| insta snapshot | serialized `ViewState` after canonical event scripts (welcome, conversation, error, goal active/paused) |
| fuzz | WSL2 nightly: completion tokenizer on arbitrary UTF-8 including combining marks and surrogate-looking sequences |
| contract | metadata table vs `polaris-kernel-directives` grammar (in `polaris-tests`); token module vs the extracted table |
| loom | none: the crate is single-threaded and lock-free by design |
| fixtures | event scripts under `tests/fixtures/ui/*.json` (synthetic, labelled synthetic) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| View Reducer | function | `reduce` | `crates/polaris-ui/crates/model/src/reduce.rs` | `message_events_append_conversation`, `seq_gap_returns_error`, `tool_title_collapsed_until_toggle`, `goal_time_excludes_pauses`, `unknown_event_ignored_not_panicking` |
| Composer Buffer | struct | `Composer` | `crates/polaris-ui/crates/model/src/composer.rs` | `long_paste_becomes_chip`, `submit_trims_trailing_blank_rows`, `history_restores_unsent_draft`, `text_over_1mib_rejected`, `session_attachment_cap_enforced`, `crlf_normalized_to_lf` |
| Completion Ranking | function | `complete` | `crates/polaris-ui/crates/model/src/completion.rs` | `slash_inside_path_is_not_a_token`, `prefix_beats_substring_beats_fuzzy`, `quoted_mention_for_spaces`, `file_results_capped_at_100`, `ties_sorted_lexicographically` |
| Keymap Table | module | `keymap` | `crates/polaris-ui/crates/model/src/keymap.rs` | `every_action_has_a_binding_or_is_marked_unbound`, `no_duplicate_chords_in_one_context`, `footer_text_generated_from_table` |

**Risks & open questions.**
- VERIFIED (demo docs): selection-tint, placeholder and colour values above are taken from docs; their rendering was never visually re-checked by us.
- VERIFIED (README/ui-design): demo statistics were labelled DEMO DATA with a `D` toggle. Polaris shows real telemetry or an explicit empty state; the `D` demo toggle is dropped.
- Open: demo syntax `$subagents-model=[m(low-xhigh)]` and `=[a,b]` lists versus the plan's `$subagent-models(m(e1-e2),...)` and bare `$name` = on. Parser ownership is `polaris-kernel-directives`; propose accepting both spellings there, and keep colouring logic here independent of that choice.
- Open: does the model crate need `polaris-kernel-directives` (layer rule allows L1 under L6) to avoid duplicating command names? Current decision is a metadata table plus a drift test.
- LIKELY: grapheme/width handling belongs near rendering; cursor maths in the composer must still use the same width library as the renderer or selection drifts (the demo hit this with wrapped rows).
- Zero-alloc is not required here (UI cold path).

**Chatlog / demo sources.** msg 1 (`$` directives and `/` commands table; `ui` as shared state models); msg 13 (all frontends subscribe to one broadcast of events: reducers consume it); msg 23 (`/context-map`, `/context-expand`, `/parallel-dag` views); msg 31 (token weather HUD, blast radius alert, reflect rules); msg 37 (Quick Chips, v2 only); demo `docs/ui-design.md`, `composer.md`, `prompt-settings.md`, `color-policy.md`, `header-logo.md`, `branding/hybrid-palette.json`, `webui/styles.css`. Corrected: HUD numbers in msg 31 (94.2% KV hit, $0.04/turn) are examples, not measurements; cache-hit display depends on S0-5.

### `polaris-ui-cli` — ratatui widgets and screens: header/logo, conversation, composer, completion, bottom sheets, HUD, stats; Windows input handling

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ui/crates/cli` | L6 | lib | `polaris-ui` | P1 |

**Purpose.** Renders `polaris-ui-model` state into a terminal and turns terminal events into model `Action`s. It rebuilds the demo UX (design only): two-row top margin, microchip header, borderless composer with a plain `>` pointer, opaque full-width completion menus, bottom-sheet popups that keep the conversation and draft underneath, a one-row footer, tool activity with collapsed gold titles, numbered code gutters, full-row diffs, selection with copy-on-release, and a deterministic `--dump WxH` text render for snapshot tests. It contains no I/O loop of its own (the loop lives in `polaris-cli`), so every screen can be rendered into a `ratatui` buffer in tests.

**API sketch.**
```rust
pub struct Theme { pub mode: ThemeMode, pub depth: ColorDepth, pub ascii: bool, pub reduce_motion: bool }
pub enum ThemeMode { Dark, Terminal }                       // --theme dark|terminal
pub enum ColorDepth { Truecolor, Ansi, None }               // resolved from --color / --color-depth / NO_COLOR / TERM=dumb
pub fn resolve_theme(opts: &ThemeOptions, env: &EnvSnapshot, is_dump: bool) -> Result<Theme, ThemeError>;

pub fn render(frame: &mut ratatui::Frame<'_>, view: &ViewState, ui: &UiState, theme: &Theme);   // pure draw
pub fn dump(view: &ViewState, ui: &UiState, theme: &Theme, w: u16, h: u16) -> String;           // plain text, no escapes
pub fn dump_styles(view: &ViewState, ui: &UiState, theme: &Theme, w: u16, h: u16) -> StyledGrid; // cell fg/bg/modifiers for colour tests

pub fn decode_input(ev: crossterm::event::Event, state: &InputState) -> Result<Option<UiInput>, InputError>;
pub struct Layout { /* header rows, history, status line, composer, blank row, footer */ }
pub fn layout(area: ratatui::layout::Rect, view: &ViewState, ui: &UiState) -> Layout;            // shared by header, history and selection
pub struct SemanticTextMap { /* source text <-> wrapped rows, same wrap code as the renderer */ }
pub enum Screen { Welcome, Conversation, Model, Provider, Status, Usage, Stats, Diff, Help, Sessions, ContextMap, ParallelDag, Hud }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-ui-model`. External: `ratatui = 0.30.2`, `crossterm = 0.29.0` (feature event-stream), `ratatui-textarea = 0.9.3`, `syntect = 5.3.0` (default-fancy), `two-face = 0.5.2`, `ansi-to-tui = 8.0.1`, `arboard = 3.7.0`, `unicode-width = 0.2.2`, `unicode-segmentation = 1.13.3`. All crate APIs are to be confirmed at implementation; none was exercised in this session.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: theme/colour resolution, header layout + logo renderer, conversation wrapping + semantic text map, input decoding, completion menu, diff/code renderer.
- [ ] Implement colour policy: `--color auto|always|never`, `--color-depth auto|truecolor|ansi`, `--theme dark|terminal`, `NO_COLOR` respected only in auto, `TERM=dumb`, dumps always plain; invalid values rejected with a typed error.
- [ ] Implement the shared layout: header (welcome 15x7 / compact 11x5 / emblem 11x1 by body rows), history, goal line right-aligned above the composer, composer, exactly one blank row, one-row footer.
- [ ] Implement the logo renderer from `polaris-ui-model::logo` with per-cell colour roles and ASCII mapping; test every glyph is one cell wide by the same width library as the renderer.
- [ ] Implement conversation rendering: no role headings, sent prompts on `#373737`, assistant text neutral, `To latest` control, Worked-for summary line, Warning/Error labels visible without colour, URL underline.
- [ ] Implement code blocks (numbered muted gutter, lexical colours via syntect/two-face for Rust, Python, JS/TS, JSON, TOML, shell) and diffs (full-row green/red, +/- markers, old/new line numbers).
- [ ] Implement the composer widget over `ratatui-textarea` (API to confirm) or a model-backed custom widget; plain `>` on the first visible editor row even when scrolled; placeholder is outside the source map.
- [ ] Implement completion menus: opaque full width, one blank top margin row, clear covered cells and mouse targets, wheel navigation clamps, hover differs from keyboard focus.
- [ ] Implement cursor presentation: hide before writing changed cells, position, then show; popups keep the cursor hidden.
- [ ] Implement selection: drag then copy on release (arboard), double-click word within 450 ms, drag by words; copy semantic source text, not cells.
- [ ] Implement bottom sheets for `/model` (effort row, provider control), `/provider` form, `/status`, `/usage`, `/stats` (calendar heatmap, tokens chart), `/diff`, `/help`, `/sessions`, `/context-map`, `/parallel-dag`, token-weather HUD, blast-radius approval prompt.
- [ ] Implement input decoding for Windows: bracketed paste is text (never Enter), release events ignored, reverse-tab normalised; Shift+Enter relies on spike S0-10 outcome. No `unsafe` and no Win32 console reader (the demo's `ReadConsoleInputW` reader is NOT ported).
- [ ] Add minimum-size screen: below 60x20 ask for a larger terminal while preserving draft and navigation.

**Tests.**
| Category | Cases |
|---|---|
| unit | theme resolution matrix (color x depth x NO_COLOR x TERM=dumb x dump); `layout()` invariants (one blank row, no overlap at 60x20, 80x24, 120x40); logo cell widths |
| proptest | wrapped-row count equals semantic-map row count for arbitrary Unicode text (the demo's drift bug class); selection copy returns a substring of the source |
| insta snapshot (--dump WxH) | scenarios welcome, conversation, error/warning, models, provider, status, usage, stats, diff, help, sessions, context-map, parallel-dag; sizes 60x20, 80x24, 120x40; ASCII variant; plus `dump_styles` JSON snapshots for colour roles (black canvas, `#1C1C1C` package, `#24D046` star, `#373737` prompt, `#F9E2AF` tool title, `#F5C2E7` auto) |
| fuzz | WSL2 nightly: input decoder on arbitrary bytes; completion renderer on arbitrary labels (width overflow) |
| smoke | ConPTY transport tests live in `polaris-tests` (spawned binary through `portable-pty`): CSI-u Shift+Enter, bracketed paste, Shift+Tab, Esc/Return in popups, Ctrl+C twice |
| security | terminal-escape injection: model output containing ESC/OSC sequences is rendered inert (stripped or escaped) in conversation and tool bodies |
| load/stress | render 10k-entry history at 120x40: frame time measured (target to measure, e.g. < 16 ms) with virtualised rows |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Colour Policy | function | `resolve_theme` | `crates/polaris-ui/crates/cli/src/theme.rs` | `auto_respects_no_color`, `always_overrides_no_color`, `never_disables_all_color`, `invalid_depth_rejected`, `dump_is_plain_regardless_of_depth`, `terminal_theme_inherits_colors` |
| Shared Layout | function | `layout` | `crates/polaris-ui/crates/cli/src/layout.rs` | `one_blank_row_between_composer_and_footer`, `header_switches_to_compact_when_draft_unsent`, `emblem_below_seven_body_rows`, `no_overlap_at_60x20` |
| Semantic Text Map | struct | `SemanticTextMap` | `crates/polaris-ui/crates/cli/src/text_map.rs` | `row_count_matches_renderer`, `selection_crosses_message_boundaries`, `wide_and_combining_chars_keep_alignment`, `chip_copies_expanded_payload` |
| Input Decoding | function | `decode_input` | `crates/polaris-ui/crates/cli/src/input.rs` | `paste_never_submits`, `csi_u_shift_enter_is_newline`, `legacy_reverse_tab_is_shift_tab`, `release_events_ignored`, `unknown_sequence_is_error_not_panic` |

**Risks & open questions.**
- UNVERIFIED (spike S0-10): crossterm 0.29 on Windows may drop the ESC of a CSI-u Shift+Enter, or ConPTY may consume it. The demo's docs report exactly this with an older crossterm and fixed it with unsafe `ReadConsoleInputW`; that is the DEMO's evidence, not ours. Fallback order from the plan: Alt+Enter / Ctrl+J, then a declared unsafe exception only if those fail.
- UNVERIFIED: `ratatui-textarea` 0.9.3 feature set versus the demo's custom editor behaviour (undo, visual-row history, chips). May need a model-backed widget instead.
- VERIFIED (demo docs): physical Alacritty key emission, live resize and font rendering were never automated in the demo; ours will not be either. Manual checklist item for P10.
- LIKELY: `ratatui`'s test backend is enough for `--dump`; confirm its exact name and API on 0.30.2.
- UNVERIFIED: syntect `default-fancy` compile cost and Windows binary size; measure in P1.
- Open: the demo set minimum size 60x20; keep as a target and test it.

**Chatlog / demo sources.** msg 1 (Ratatui TUI with syntax highlight, diff view, sub-task tree), msg 13/17/23 (`/concurrency-stats`, `/parallel-dag`, `/context-map` boxed panels), msg 31 (blast-radius alert with [Approve]/[Abort], token-weather HUD), msg 35 (`/hack` report panel); demo `docs/ui-design.md`, `composer.md`, `keyboard-verification.md`, `color-policy.md`, `header-logo.md`, README option list (`--scenario`, `--dump WxH`, `--ascii`, `--reduce-motion`, `--key-trace`). Corrected: the demo's `--key-trace` privacy rule (no draft text in traces) is kept; the "preview/simulated" banner is removed because Polaris is real.

### `polaris-ui-gui` — Slint components and controllers bound to the shared view state (desktop now, mobile v2)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ui/crates/gui` | L6 | lib | `polaris-ui` | P9 |

**Purpose.** The Slint view layer for `polaris-desktop`: `.slint` components (chat, sessions, diff viewer, HUD, latency graph, sub-agent tree) plus Rust controllers that copy `ViewState` into Slint models and forward UI callbacks as `Action`s. Generated Slint code is isolated in this crate by `slint-build`. It reuses the web welcome-page tokens (`#76B900`, black canvas) through `polaris-ui-model::tokens`, so desktop, web and TUI look like one product. Mobile (Android Rust-only, iOS) is declared v2 and not built now.

**API sketch.**
```rust
pub fn mount(window: &MainWindow, state: std::sync::Arc<arc_swap::ArcSwap<ViewState>>)    // arc-swap not in this crate's ext list: API to confirm
    -> Result<GuiHandle, GuiError>;
pub struct GuiHandle;                                   // owns the callback wiring and the model adapters
impl GuiHandle {
    pub fn apply(&self, view: &ViewState) -> Result<(), GuiError>;                         // called on the Slint event loop
    pub fn on_action(&self, f: impl Fn(Action) + 'static);                                 // UI -> model actions
}
pub enum GuiError { #[error("slint platform: {0}")] Platform(String), /* ... */ }
// .slint: component ChatView, SessionList, DiffViewer, HudBar, DagTree, ComposerBar (+ Tokens global generated from ui-model)
```
Note: `arc-swap` is pinned in `workspace.toml` but not in this crate's planned `ext`; the simplest wiring is `slint::invoke_from_event_loop` pushing owned snapshots, which needs no extra crate (decide at implementation).

**Dependencies.** Internal: `polaris-sdk`, `polaris-ui-model`. External: `slint = 1.18.1` (features std, backend-winit, renderer-femtovg, renderer-software, accessibility, compat-1-2), `slint-build = 1.18.1`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: ViewState to Slint model adapter, action callback bridge, token global generation, diff viewer model.
- [ ] Record the Slint licence decision (GPLv3 / royalty-free / commercial) in the licence ADR before any binary is published (plan: decided with cargo-deny in P0).
- [ ] Add `build.rs` with `slint-build`; keep generated code in `OUT_DIR`, never committed.
- [ ] Generate a Slint `Tokens` global from `polaris-ui-model::tokens` so colours are not hand-copied.
- [ ] Implement `ChatView` (streaming text, tool activity collapse, code with line numbers, diff rows) with virtualised list.
- [ ] Implement `DiffViewer` with per-hunk approve/reject (maps to `Approval` commands); AST-chunk granularity only if the kernel emits it.
- [ ] Implement `HudBar` (token weather) and a latency graph S1 vs S2 fed by telemetry snapshots; empty-state when no data (no fake series).
- [ ] Implement `DagTree` for `/parallel-dag` and the sub-agent tree.
- [ ] Keyboard parity: same `Action` table as the TUI where the platform allows; accessibility feature enabled.
- [ ] Software renderer fallback test (headless) so CI can render without a GPU.
- [ ] v2 (declared, not built): Android/iOS targets and touch-first diff review (swipe approve/reject) and Quick Chips.

**Tests.**
| Category | Cases |
|---|---|
| unit | model adapter maps N conversation entries to N Slint rows; action bridge maps button to `Action` |
| smoke | headless software-renderer window creates, `mount` succeeds, one event renders (UNVERIFIED which Slint testing backend API: confirm at implementation) |
| insta snapshot | software-renderer screenshot of Chat/Diff/HUD at fixed sizes compared by hash or perceptual diff (`image_hasher` is in the workspace); tolerance to be measured, never loosened to pass |
| fixtures | reuse `tests/fixtures/ui/*.json` event scripts from `polaris-ui-model` |
| scalability | 10k-row conversation scroll frame time (target to measure) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Slint Model Adapter | function | `apply` | `crates/polaris-ui/crates/gui/src/adapter.rs` | `conversation_rows_match_view`, `empty_hud_shows_no_data_state`, `updates_are_applied_on_event_loop_only`, `tool_expansion_flag_propagates` |
| Action Bridge | function | `on_action` | `crates/polaris-ui/crates/gui/src/actions.rs` | `submit_button_emits_submit`, `approve_hunk_emits_approval`, `keyboard_shortcut_matches_keymap_table` |
| Design Tokens Global | module | `tokens` | `crates/polaris-ui/crates/gui/src/tokens.rs` | `accent_equals_model_accent`, `canvas_is_pure_black`, `all_roles_present` |

**Risks & open questions.**
- VERIFIED (plan): Slint is GPLv3 / royalty-free / commercial; licence decision is a P0 deliverable. Not decided yet.
- UNVERIFIED: femtovg on a GTX 1650 box and in headless CI; software renderer is the fallback. Binary size and compile time (Slint is heavy) are unmeasured.
- UNVERIFIED: Slint 1.18.1 mobile status (Android Rust-only was stated in workspace.toml; not verified here).
- LIKELY: P9 is late; the GUI must not block the MVP (end of P4). Keep it out of default workspace checks if compile time hurts.
- Open: which Slint testing/screenshot API is available for snapshot tests (to confirm).

**Chatlog / demo sources.** msg 0 (Gui: Slint), msg 1 (Slint desktop, light and fast), msg 3 (latency graphs S1/S2 and sub-agent tree in desktop), msg 1 action plan (Slint GUI with themes and an interactive diff viewer), msg 23 (`/parallel-dag` in GUI), msg 37 (Slint mobile v2, swipe review, Quick Chips; the chatlog said Tauri v2 Mobile, which D6 replaced with Slint). Corrected: Tauri dropped (D6).

### `polaris-ui-web` — Leptos components (SSR + hydrate), the approved welcome page, PWA manifest and service worker assets

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `crates/polaris-ui/crates/web` | L6 | lib | `polaris-ui` | P9 |

**Purpose.** The component library compiled twice by cargo-leptos: with the `ssr` feature into `polaris-web` (server) and with `hydrate` into wasm (browser). It rebuilds the approved welcome-page design (VERIFIED: only the welcome page of the demo web UI is approved) as Leptos components, plus streaming chat, sessions, research, usage and settings views. Server-only code (anything that touches the kernel) lives behind server functions or in `polaris-web`, not here. The only hand-written JavaScript in the product is the service worker; all wasm glue is generated.

**API sketch.**
```rust
#[component] pub fn App() -> impl IntoView;                       // router + meta + shell
#[component] pub fn Shell(children: Children) -> impl IntoView;   // 68px icon rail + topbar, mobile disclosure below 768px
#[component] pub fn WelcomePage() -> impl IntoView;               // atmosphere + h1 + composer pill
#[component] pub fn Composer(on_submit: Callback<Submission>) -> impl IntoView;
#[component] pub fn ModelPopover() -> impl IntoView;              // model / mode (plan, auto) / effort low..max radios
#[component] pub fn ChatView() -> impl IntoView;                  // consumes an SSE token stream
#[component] pub fn SessionsView() -> impl IntoView;
#[component] pub fn ContextMapView() -> impl IntoView;            // /context-map
#[component] pub fn ParallelDagView() -> impl IntoView;           // /parallel-dag
pub fn stylesheet() -> String;                                    // :root tokens generated from polaris-ui-model::tokens::css_variables()
pub const SERVICE_WORKER_JS: &str;                                // the single hand-written JS asset (include_str!)
pub const MANIFEST_JSON: &str;                                    // PWA manifest
```
(Leptos 0.8 component, `Suspense` and streaming-mode APIs are to be confirmed at implementation; names above are the intended shape.)

Web design contract extracted from the demo (VERIFIED from `webui/index.html`, `styles.css`, `README.md`):
- Pure black canvas and shell; fonts Space Grotesk (headings, weight 400-500, letter-spacing -.035em), Inter (body 14px/1.6), JetBrains Mono (labels); accent `#76B900`, light accent `#C0E58C`; gold `#F9E2AF`, blue `#89B4FA`, pink `#F5C2E7`, error `#AC4242`, line `#FFFFFF12`; radius 14px; transition 160 ms.
- Layout: 68px sticky icon rail (new chat, Chat, History, Models, Usage, States, Settings) with hover/focus tooltips, 56px topbar with a Context disclosure (workspace files, current goal, session settings); below 768px a native-disclosure mobile menu; below 360px tighter padding.
- Welcome: single heading "What's on your mind for Polaris?", a compact glass composer (about 60px tall when empty, grows to a bounded scroll), textarea placeholder "Ask Polaris anything...", attach button, a model pill opening a 300px opaque-black popover (model radios, mode plan/auto, effort low/medium/high/xhigh/max, "Model settings" link), send button.
- Atmosphere (welcome only): gray-green mesh, drifting nebula and two aurora ribbons (transform/opacity only, 31-48 s loops), 2,400 one-pixel stars in three seeded SVG layers twinkling over 13/19/23 s, a 128px grain PNG at low opacity, feathered dark backing behind the heading. `prefers-reduced-motion` freezes animation; `forced-colors` removes decoration; `prefers-reduced-transparency` removes light and grain.
- Accessibility: skip link, `sr-only` labels, visible 2px focus outline in light green, every nav link has an accessible name.
- The demo shipped these pages as static HTML+CSS with CSP `default-src 'none'`. Polaris hydrates with wasm, so the CSP must allow its own script and wasm (exact directives to determine at implementation, e.g. `'wasm-unsafe-eval'`; UNVERIFIED).

**Dependencies.** Internal: `polaris-sdk`, `polaris-ui-model`. External: `leptos = 0.8.22`, `leptos_router = 0.8.17`, `leptos_meta = 0.8.7` (pinned: leptos_axum 0.8.10 requires >= 0.8.6), `wasm-bindgen = 0.2.129`. Assets bundled locally (fonts, aurora/stars SVG, grain PNG, icons) with their OFL licence files; no CDN.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: token CSS generation, WelcomePage, Composer submit flow, SSE chat stream consumer, service-worker cache policy, manifest.
- [ ] Set up the cargo-leptos dual-feature layout (`ssr`, `hydrate`); keep `polaris-ui-web` out of `default-members` once real deps land; verify `cargo check --workspace` still passes on defaults.
- [ ] Port the CSS: generate `:root` tokens from `polaris-ui-model`; copy the demo's welcome atmosphere assets with their licences (fonts are OFL; star/aurora SVGs were generated by the demo author: confirm provenance and licence before copying).
- [ ] Implement `Shell`, `WelcomePage`, `Composer`, `ModelPopover` as accessible components (native radios, `details` fallback).
- [ ] Implement `ChatView` over SSE (token stream) with `Suspense` boundaries for heavy blocks (diff, test output, AST tree) so the shell paints first; verify out-of-order streaming in a browser.
- [ ] Implement route-level chunks (code splitting per route: `/research`, `/sessions`) if the toolchain supports it. The chatlog claims per-route wasm chunks; UNVERIFIED for Leptos 0.8 / cargo-leptos (spike `polaris-spike-web` reports binary sizes).
- [ ] Implement PWA: manifest, offline app-shell caching in the service worker, versioned cache names, update flow; never cache `/api` or event streams.
- [ ] Implement `/usage` (heatmap, token series using real telemetry or an explicit empty state) and `/settings` (session settings list).
- [ ] Implement `/context-map` and `/parallel-dag` views from `ViewState`.
- [ ] Mobile layout parity (breakpoints at 767 px and 359 px); Quick Chips and swipe review are v2.
- [ ] BLOCKED-ON-ACCESS (real browser matrix): Safari/Firefox/Chrome checks require installed browsers; Chrome path via chromiumoxide in the guest is P7.

**Tests.**
| Category | Cases |
|---|---|
| unit | `css_variables()` contains every token; manifest parses as JSON with required fields; service worker never caches non-GET or `/events` |
| insta snapshot | SSR HTML of `WelcomePage` (server render of the component tree, no browser) at fixed state; token CSS snapshot |
| contract | rendered HTML contains the `h1` text, skip link, labelled controls (static a11y checks written as string assertions, not a full audit) |
| smoke | `cargo leptos build` (ssr + wasm) succeeds in WSL2 or Windows (UNVERIFIED tool availability: cargo-leptos not installed on the machine) |
| spike | `polaris-spike-web`: SSE fan-out and bundle size/TTFB measurement |
| fixtures | event scripts shared with `polaris-ui-model` |
| security | CSP header test in `polaris-web`; no inline script; escaped model output (XSS test strings in assistant text) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Design Token CSS | function | `stylesheet` | `crates/polaris-ui/crates/web/src/style.rs` | `contains_all_roles`, `canvas_is_black`, `fonts_declared_with_swap`, `reduced_motion_block_present` |
| Welcome Page | module | `WelcomePage` | `crates/polaris-ui/crates/web/src/welcome.rs` | `renders_exact_heading`, `composer_is_labelled`, `model_popover_lists_five_efforts`, `atmosphere_is_aria_hidden` |
| Service Worker Policy | module | `sw` | `crates/polaris-ui/crates/web/src/sw.rs` | `shell_precached`, `api_and_streams_bypassed`, `old_caches_evicted_on_activate`, `non_get_never_cached` |
| Chat Stream Consumer | async_function | `consume_tokens` | `crates/polaris-ui/crates/web/src/chat.rs` | `appends_deltas_in_order`, `reconnect_resumes_from_seq`, `malformed_frame_is_error_not_panic` |

**Risks & open questions.**
- VERIFIED (chatlog msg 33 vs reality): "TTFB < 2 ms" and "shell in < 1.5 ms" are targets, not facts. Report measured numbers from the spike with conditions.
- UNVERIFIED: per-route wasm chunk splitting in the Leptos 0.8 toolchain; the claim is from the chatlog.
- UNVERIFIED: cargo-leptos is not installed (plan machine facts); `wasm32-unknown-unknown` target availability.
- VERIFIED (plan): leptos_axum 0.8.10 requires `leptos_meta` >= 0.8.6, pinned at 0.8.7; web libraries must not pull a second reqwest/axum major.
- LIKELY: the hydrated app needs `'wasm-unsafe-eval'` or equivalent in CSP; the demo's strict CSP will not carry over unchanged.
- Open: authentication for the browser (token cookie vs `Authorization`) is shared with `polaris-web`; browser clients cannot use named pipes, only HTTPS to `polaris-web`.
- Open: demo star SVG/aurora asset licence and provenance (OFL files are present only for the fonts).

**Chatlog / demo sources.** msg 32 (SSR with async page loading), msg 33 (Axum + Leptos SSR, immediate shell, out-of-order Suspense streaming, progressive hydration of interactive islands, SSE + WebSocket dual channel, chunk splitting, single binary), msg 37 (PWA, Quick Chips, swipe review: v2), msg 1 (web child as bridge, corrected); demo `webui/index.html`, `styles.css`, `README.md`, `assets/*`. Corrected: Tauri replaced by Leptos PWA (D6); "no Node.js" kept; static-HTML CSP not reusable as is.

### `polaris-cli` — terminal client `polaris`: ratatui TUI (default), headless `polaris run`, `doctor`, `--dump WxH`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-cli` | L7 | app | none | P1 |

**Purpose.** The first interface and the one the MVP demo runs on. It parses arguments (clap), resolves the layered config, starts an in-process kernel by default (or attaches to `polarisd` with `--connect` once P9 lands), and runs the TUI event loop built from `polaris-ui-cli`. It also offers `polaris run <task>` (headless, line-oriented or JSON-lines output for the scripted demo and CI), `polaris doctor` (environment and sandbox checks, keyed checks report `SKIPPED: no key`), and the deterministic `--dump WxH` snapshot mode. At startup it installs the rustls aws-lc-rs provider and the mimalloc global allocator. It is the only binary that wires terminal state; all rendering decisions live in `polaris-ui-cli` and `polaris-ui-model`.

**API sketch (CLI surface).**
```text
polaris [OPTIONS] [PROMPT]               # TUI (default); PROMPT pre-fills the composer
polaris run [OPTIONS] <TASK>             # headless; streams events; exit code reflects ProofOfDone
polaris doctor [--json]                  # WSL2, OpenShell, models, keys (SKIPPED: no key), disk, versions
polaris --dump WxH [--scenario NAME]     # deterministic plain-text screen, no TTY needed
polaris --connect <ENDPOINT>             # attach to polarisd (P9): pipe name, uds path or https://host:port (mTLS)

Options: --theme dark|terminal   --color auto|always|never   --color-depth auto|truecolor|ansi
         --ascii   --reduce-motion   --account-email EMAIL (display label only)
         --key-trace PATH (new file; no draft text)   --config PATH   -h/--help   -V/--version
```
```rust
// src/main.rs (replaces the doc-only lib skeleton)
fn main() -> miette::Result<()>;                         // no unwrap/expect; every error becomes a miette report
async fn run_tui(cfg: PolarisConfig, opts: UiOptions) -> Result<ExitCode, CliError>;
async fn run_headless(cfg: PolarisConfig, task: String, fmt: OutputFormat) -> Result<ExitCode, CliError>;
fn dump(opts: &UiOptions, w: u16, h: u16, scenario: Option<&str>) -> Result<String, CliError>;
pub enum CliError { /* thiserror: Config, Terminal, Kernel, Rpc, UsageError */ }
```
`--scenario` is allowed with `--dump` (and tests) only, and loads event scripts from fixtures; there is no simulated "preview mode" in the product.

**Full keymap (design from the demo, VERIFIED in README/composer.md; implemented by `polaris-ui-model::keymap` + `polaris-ui-cli::input`).**

| Key | Action |
|---|---|
| Enter / Return in composer | Submit |
| Shift+Enter | Insert newline (depends on S0-10) |
| Alt+Enter, Ctrl+Enter, Ctrl+J | Additional newline bindings (Ctrl+Enter depends on terminal; UNVERIFIED) |
| Up / Down at first / last visual composer row | Older / newer prompt history; Down past newest restores unsent draft |
| Shift+Tab | Toggle auto / plan mode |
| Shift+Up / Shift+Down | Raise / lower active model effort (low, medium, high, xhigh, max) immediately |
| Esc during work | Interrupt after dismissing any popup, menu, preview or selection; draft preserved |
| Esc in any popup | Back to previous view, no selection applied |
| Enter in info popups | Close status, usage, stats, diff, transcript, help |
| Enter in selection menus | Confirm (model menu applies model and closes) |
| `?` with empty draft | Open shortcuts |
| PageUp / PageDown | Scroll conversation or popup |
| Mouse wheel over `/`, `@`, `$` menu | Move one suggestion per notch (clamped); does not scroll conversation |
| F2 | Open status |
| Ctrl+O | Expand / collapse all tool bodies (click a title toggles one) |
| Ctrl+C | Interrupt work, close popup, clear draft; press twice on an empty draft to exit |
| Tab / Shift+Tab in provider form | Next / previous field |
| Ctrl+S in provider form | Save settings |
| P / C in /model | Choose / configure provider |
| Left / Right in /model | Adjust effort |
| Tab or Left / Right in /stats | Switch Overview / Models |
| R in /stats | Cycle All time / 7 days / 30 days |
| Ctrl+V, Shift+Insert | App-owned paste (image first, else text) |
| E / C in text-attachment preview | Expand into draft / copy |
| Mouse drag + release | Select and copy (semantic source text) |
| Double-click, then drag | Select word, extend by whole words |
| Click tool title / `To latest` | Toggle that tool entry / jump to bottom |

Dropped from the demo: `D` (demo-data toggle), because Polaris shows real data or an empty state. Undo/redo keys in the composer are required by the model (undo state exists) but their chords are not stated in the demo docs; assign at implementation (UNVERIFIED).

**TUI layout regions (top to bottom; one shared layout calculation).**
1. Top margin: 2 blank rows.
2. Header: microchip logo (welcome 15x7, compact 11x5, emblem 11x1 fallback), starting 2 rows down and 2 columns in, with 4 metadata rows to its right after 3 columns: bold `Polaris` + muted version, optional account label, model/provider, workspace (truncate at grapheme boundaries). Welcome shows with an empty conversation and enough height; submission or work switches to compact; an unsent draft also compacts when fewer than 11 body rows remain.
3. Conversation history (fills the centre): no role headings, sent prompts on `#373737`, tool activity as gold titles + Ctrl+O hint, code with numbered gutters, diffs with full-row surfaces, `Worked for <elapsed> · HH:MM`, Warning/Error labels.
4. `To latest` control (centred, only when scrolled up and no overlay is open).
5. Working line: `• Working (6m 34s · esc to interrupt)` with real elapsed time (ASCII: `*` and `.`).
6. Goal line: right-aligned, blue `#89B4FA`, `goal active (time)` / `goal paused (time)`.
7. Composer: no border, plain `>` pointer on the first visible row; empty placeholder "What's on your mind for Polaris?" in `#525252`.
8. Exactly one blank row.
9. Footer (one row): shortcuts left; `auto mode on` (`#F5C2E7`) / `plan mode on` (`#F9E2AF`), model, effort right.
10. Overlays: completion menus (opaque, full width, one blank top row), bottom-sheet popups (`/model /status /usage /stats /diff /help /sessions /providers /context-map /parallel-dag`), attachment preview, approval (blast-radius), toasts (2 s `Copied N words`).
11. Token-weather HUD placement is OPEN (candidates: right side of the footer, or a `/status` section); the demo has no HUD.

**Dependencies.** Internal: `polaris-kernel`, `polaris-ui-cli`, `polaris-rpc`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `clap = 4.6.7` (derive), `crossterm = 0.29.0`, `ratatui = 0.30.2`, `miette = 7.6.0` (fancy), `mimalloc = 0.1.52`, `rustls = 0.23.46`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: argument parsing, startup sequence, TUI event loop, headless runner, doctor report, dump mode.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris` (keep a thin `lib.rs` for testable logic if useful).
- [ ] Implement clap definitions and validation (invalid `--color-depth`/`--theme` rejected with actionable messages; `--dump` parses `WxH`).
- [ ] Startup: install aws-lc-rs as the rustls default provider, set mimalloc, init `polaris-log`, load config layers, build the kernel via the composition root.
- [ ] TUI loop: alternate screen, bracketed paste, mouse capture, panic-safe terminal restore (with `panic = "unwind"`, restore in a guard `Drop`; no `unsafe`), resize handling, event channel from the kernel broadcast.
- [ ] Implement `polaris --dump WxH` end to end and wire it to insta snapshots (first deliverable of P1 and a smoke test).
- [ ] Implement `polaris run`: JSON-lines of events, exit codes (proposal: 0 verified done, 1 task failed, 2 usage error, 3 ProofOfDone blocked completion, 4 environment/doctor failure; decide in the descriptor).
- [ ] Implement `polaris doctor`: collect checks from platform/openshell crates; keyed checks print `SKIPPED: no key`.
- [ ] Windows terminal handling per spike S0-10: ConPTY bytes test via `portable-pty`, then pick Shift+Enter path or fallback chords.
- [ ] `--connect` (P9): construct a `KernelClient` from `polaris-rpc` behind the same `KernelApi` trait so the TUI code path is identical.
- [ ] BLOCKED-ON-ACCESS (Token Factory key): live `polaris run` against Nemotron; until then run against the local Ollama/llama.cpp endpoint (P3 plan).
- [ ] BLOCKED-ON-ACCESS (WSL2 Ubuntu + OpenShell): `doctor` sandbox checks and sandboxed demo run.

**Tests.**
| Category | Cases |
|---|---|
| unit | clap parsing table (valid/invalid); `WxH` parser; exit-code mapping |
| insta snapshot (--dump WxH) | welcome / conversation / popups at 60x20, 80x24, 120x40, plain and `--ascii`; binary-level via `assert_cmd` so the real CLI path is covered |
| smoke | `polaris --version`, `--help`; `polaris doctor` exits with a report; `polaris --dump 80x24` stable across two runs; ConPTY session starts, accepts a typed draft, Ctrl+C twice exits cleanly (Windows) |
| contract | headless JSON-lines schema validated with `jsonschema` against the event schema from `polaris-sdk` |
| security | secrets never printed: key-trace file contains no draft text; `doctor` output redacts key values |
| stress | repeated resize storm in ConPTY; terminal always restored after panic injection test (unwind) |
| load | headless run streaming 100k events: constant memory (target to measure) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| CLI Argument Parsing | function | `parse_args` | `apps/polaris-cli/src/args.rs` | `defaults_start_tui`, `dump_requires_valid_wxh`, `invalid_color_depth_rejected`, `scenario_only_with_dump`, `run_requires_task` |
| Startup Sequence | async_function | `bootstrap` | `apps/polaris-cli/src/bootstrap.rs` | `installs_aws_lc_provider_once`, `config_error_is_reported_not_panicking`, `terminal_restored_on_error` |
| Headless Runner | async_function | `run_headless` | `apps/polaris-cli/src/headless.rs` | `emits_jsonl_events`, `exit_3_when_proof_of_done_blocks`, `cancel_on_ctrl_c_cleans_sandbox` |
| Doctor Report | function | `doctor` | `apps/polaris-cli/src/doctor.rs` | `keyed_checks_skipped_without_key`, `json_output_is_valid`, `redacts_secrets` |

**Risks & open questions.**
- UNVERIFIED (S0-10): crossterm 0.29 handling of Shift+Enter under ConPTY on Windows 11 (build 26100 here). Covered by spike S0-10 and the `portable-pty` transport test.
- VERIFIED (plan D3): `panic = "unwind"`; the TUI must restore the terminal on unwind via a guard. A process abort or kill cannot be handled (the demo documents the same limit).
- LIKELY: embedding the kernel in-process makes the TUI sensitive to kernel panics; the supervisor handles task panics, but FFI crashes go to `polaris-worker`.
- UNVERIFIED: physical terminal behaviour (Alacritty, Windows Terminal) is a manual check; the demo made the same disclaimer.
- Open: HUD placement; undo/redo chords; whether `polaris` with no TTY should print a static dump like the demo did (leaning yes, consistent with `--dump`).

**Chatlog / demo sources.** msg 0 (cli = ratatui; `$` / `/` session settings), msg 1 (TUI with syntax highlighting, diff view, sub-task tree; `/quit` or `/exit`), msg 31 (blast-radius prompt, HUD), msg 35 (`/hack` panel); demo README (keymap, options, `--dump WIDTHxHEIGHT`), `docs/keyboard-verification.md` (ConPTY test method: spawn real exe, write bytes), `docs/ui-design.md`. Corrected: the demo's unsafe Win32 console reader is not reused; `--preview` simulated mode dropped.

### `polaris-desktop` — native Slint desktop client `polaris-desktop`

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-desktop` | L7 | app | none | P9 |

**Purpose.** Hosts the `polaris-ui-gui` Slint window and bridges it to the kernel event stream. It runs an embedded kernel by default or connects to a running `polarisd` over a named pipe (Windows) / UDS (Linux leg) or mTLS TCP. It owns the Tokio runtime and the Slint event loop and keeps them apart: kernel work runs on Tokio threads, UI updates are marshalled onto the Slint thread. Mobile (Android Rust-only, iOS) is declared v2; this crate only keeps the entry point mobile-ready (library entry plus thin `main`), without building those targets.

**API sketch.**
```text
polaris-desktop [--connect <ENDPOINT>] [--config PATH] [--theme dark|system]
```
```rust
fn main() -> miette::Result<()>;                                   // thin; builds runtime, installs aws-lc-rs, starts UI
pub fn run(cfg: PolarisConfig, mode: KernelMode) -> Result<(), DesktopError>;
pub enum KernelMode { Embedded, Remote { endpoint: ClientTransport } }
struct Bridge { /* subscribes to events, calls polaris_ui_model::reduce, pushes snapshots to Slint via invoke_from_event_loop */ }
pub enum DesktopError { /* thiserror: Slint platform, Kernel, Rpc, Config */ }
```

**Dependencies.** Internal: `polaris-kernel`, `polaris-ui-gui`, `polaris-rpc`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `slint = 1.18.1`, `slint-build = 1.18.1`, `rustls = 0.23.46`. (A Windows GUI-subsystem build setting is not a lint attribute and does not conflict with D10; confirm the exact mechanism at implementation.)

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: runtime/UI thread bridge, kernel mode selection, shutdown sequence.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris-desktop`.
- [ ] Install the rustls aws-lc-rs provider and choose the allocator at startup, same as the CLI.
- [ ] Implement the bridge: kernel event stream to `reduce` to Slint model update on the event loop; commands from Slint callbacks to the kernel. No blocking calls on the Slint thread.
- [ ] Implement embedded and remote modes behind the same `KernelApi` generic; remote reuses `polaris-rpc::connect`.
- [ ] Implement graceful shutdown: close window, cancel kernel, Reaper teardown (sandboxes, temp, workers) before process exit.
- [ ] Persist window geometry and theme in `.polaris`/`~/.polaris` config through `polaris-config` (no secrets).
- [ ] Packaging notes for Windows (exe + `onnxruntime.dll` beside it only if linked in the same bundle; the desktop itself should not load ORT).
- [ ] v2 (declared): Android/iOS targets and touch-first review, Quick Chips; not built in v1.
- [ ] BLOCKED-ON-ACCESS (display/GPU environment for CI): headless software-renderer tests only until a GUI runner exists.

**Tests.**
| Category | Cases |
|---|---|
| unit | kernel-mode selection from args/config; shutdown ordering (mock kernel) |
| smoke | app starts with the software renderer in a headless session, renders first frame, exits 0 on programmatic close (UNVERIFIED how Slint exposes headless start; confirm) |
| contract | remote mode against `polaris-rpc` loopback server in `polaris-tests` |
| security | refuses plaintext remote endpoints; token never shown in UI strings or logs |
| stress | rapid event flood (10k events/s) keeps UI thread responsive (target to measure) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Kernel Mode Selection | function | `select_mode` | `apps/polaris-desktop/src/mode.rs` | `defaults_to_embedded`, `connect_flag_selects_remote`, `plaintext_remote_rejected`, `invalid_endpoint_is_error` |
| UI Thread Bridge | struct | `Bridge` | `apps/polaris-desktop/src/bridge.rs` | `events_reduced_in_order`, `ui_updates_only_via_event_loop`, `lagging_ui_drops_to_snapshot_not_unbounded_queue` |
| Shutdown Sequence | async_function | `shutdown` | `apps/polaris-desktop/src/shutdown.rs` | `reaper_runs_before_exit`, `second_close_is_idempotent` |

**Risks & open questions.**
- UNVERIFIED: Slint plus Tokio integration details; dedicated runtime thread is the intended shape.
- VERIFIED (plan): Slint licence decision pending (P0); GUI is P9 (could), not MVP.
- UNVERIFIED: GPU renderer on the GTX 1650 test machine and in CI; software renderer is the fallback.
- LIKELY: desktop and CLI duplicate startup code; extract to a shared bootstrap in `polaris-config`/kernel or accept small duplication (open).

**Chatlog / demo sources.** msg 0/1 (Slint desktop on three OSes), msg 3 (latency graphs and sub-agent tree), msg 35 (desktop over gRPC UDS), msg 37 (mobile via Slint instead of the chatlog's Tauri v2 Mobile; D6). Corrected: UDS on Windows becomes named pipe; Tauri removed.

### `polaris-web` — Axum + Leptos SSR server `polaris-web`: streaming pages, SSE token streams, WebSocket terminal, PWA

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-web` | L7 | app | none | P9 |

**Purpose.** The web server application. It serves server-rendered Leptos pages (immediate shell, `Suspense` blocks streamed out of order, wasm hydration of interactive islands), the PWA manifest and the single hand-written JavaScript file (service worker), and two real-time channels backed by the kernel: SSE for one-way token/event streams and WebSocket for the PTY terminal. The page components come from `polaris-ui-web`; the kernel is embedded or reached through `polaris-rpc`. It binds to loopback by default; anything else requires TLS and authentication. It is built by cargo-leptos with the dual `ssr` / `hydrate` feature layout and is excluded from `default-members` once real dependencies land.

**API sketch.**
```text
polaris-web [--bind 127.0.0.1:3000] [--connect ENDPOINT] [--config PATH]
Routes (workspace.toml): /  /chat  /sessions  /research  /usage  /settings
Added from chatlog:      /context-map  /parallel-dag
Channels:                GET /api/events?from_seq=N   (SSE, resumable)     GET /api/terminal (WebSocket upgrade, PTY)
PWA:                     /manifest.webmanifest  /sw.js  /pkg/* (wasm + js glue, generated)
```
```rust
fn main() -> miette::Result<()>;
pub fn router<K: KernelApi>(kernel: K, cfg: &WebConfig) -> Result<axum::Router, WebError>;   // leptos_axum routes + API + static
async fn events_sse(/* State, Query<from_seq>, auth */) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, WebError>;
async fn terminal_ws(/* WebSocketUpgrade, auth, origin check */) -> Result<impl IntoResponse, WebError>;
pub enum WebError { /* thiserror: Auth, Origin, Kernel, Render */ }
```
(Axum 0.8 / leptos_axum 0.8.10 handler and extractor signatures are to be confirmed at implementation.)

**Dependencies.** Internal: `polaris-kernel`, `polaris-ui-web`, `polaris-rpc`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `axum = 0.8.9` (ws, macros), `tower-http = 0.6.11` (trace, cors, fs, compression-br, compression-zstd), `leptos = 0.8.22`, `leptos_axum = 0.8.10`, `leptos_router = 0.8.17`, `leptos_meta = 0.8.7`, `wasm-bindgen = 0.2.129`. The chatlog's "single binary via `include_dir!`" needs an embedding crate that is not in `workspace.toml`; either add one through the TOML or serve `pkg/` and assets from disk with `tower-http` `fs`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: router assembly, auth middleware, SSE endpoint, WebSocket terminal, security headers, static asset serving.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris-web`; configure cargo-leptos metadata (`ssr`, `hydrate`) and exclude from `default-members`.
- [ ] Build the router with `leptos_axum` for the page routes and plain Axum routes for `/api/*`.
- [ ] Verify out-of-order streaming in a browser: shell first, heavy blocks arrive later (spike `polaris-spike-web` measures TTFB and time-to-shell; report numbers with conditions).
- [ ] SSE: `/api/events` with `from_seq` resume, heartbeat comment frames, per-client bounded buffer, drop-to-snapshot for slow clients.
- [ ] WebSocket: `/api/terminal` bridging to `AttachTerminal`; enforce `Origin` check (cross-site WebSocket hijacking) and the same auth as HTTP; message size limits.
- [ ] Auth: loopback-only default, per-launch random token in a cookie (`HttpOnly`, `SameSite=Strict`) or header; `Host` allow-list to resist DNS rebinding; refuse non-loopback bind without TLS.
- [ ] Security headers/CSP compatible with wasm hydration; no inline scripts except what cargo-leptos requires (to check).
- [ ] Compression: brotli/zstd via `tower-http`; cache headers for hashed assets; never cache API or streams.
- [ ] Windows service worker scope and PWA install check (manifest valid, offline shell works).
- [ ] BLOCKED-ON-ACCESS (cargo-leptos not installed; wasm target): build and browser checks start once tools exist (P0 tool install list).

**Tests.**
| Category | Cases |
|---|---|
| unit | Origin/Host validators; token compare is constant-time; SSE frame encoder |
| smoke | server starts, `GET /` returns 200 with the exact heading, `/manifest.webmanifest` valid, `/sw.js` served with correct content type |
| contract | `/api/events` frames match the `polaris.v1` event schema; `from_seq` replay gap-free |
| security | request without token 401; WebSocket with foreign `Origin` refused; bad `Host` refused; XSS strings in model output are escaped in SSR HTML; CSRF on state-changing POST |
| load | N SSE clients (1/16/256) receiving a token stream; p99 inter-event latency with hdrhistogram (targets to measure) |
| scalability | memory per connected client; 1k idle connections |
| stress | client disconnect/reconnect storm with resume; slow reader does not block the kernel broadcast |
| spike | `polaris-spike-web`: connection floods and SSE fan-out bursts |
| fuzz | WSL2 nightly: SSE query parser, WebSocket frame handler (message size caps) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Origin And Host Guard | function | `check_origin_host` | `apps/polaris-web/src/guard.rs` | `loopback_origin_allowed`, `foreign_origin_rejected`, `rebinding_host_rejected`, `missing_origin_on_ws_rejected` |
| SSE Event Stream | async_function | `events_sse` | `apps/polaris-web/src/sse.rs` | `resumes_from_seq`, `heartbeat_sent_when_idle`, `slow_client_gets_snapshot_not_backlog`, `unauthenticated_401` |
| Terminal WebSocket | async_function | `terminal_ws` | `apps/polaris-web/src/terminal.rs` | `stdin_forwarded_to_pty`, `oversized_message_closes_socket`, `closes_pty_on_disconnect` |
| Router Assembly | function | `router` | `apps/polaris-web/src/router.rs` | `all_routes_registered`, `api_not_cached`, `static_assets_compressed`, `non_loopback_without_tls_is_error` |

**Risks & open questions.**
- VERIFIED (plan): D6 chooses Axum + Leptos SSR + PWA; Tauri is dropped.
- UNVERIFIED: the chatlog's "TTFB < 2 ms", "shell in < 1.5 ms", per-route wasm chunk splitting and "single binary via include_dir" are claims, not facts; they are measured or decided in the spike and P9.
- LIKELY: a localhost web server is a CSRF / DNS-rebinding / WebSocket-hijack target; the guards above are mandatory, not optional.
- Open: browser auth UX (cookie token printed in the terminal as a one-time URL vs. a login page).
- Open: SSE vs. WebSocket split is from the chatlog (SSE for tokens, WS for terminal); the kernel also has `AttachSession` bidi, which the web server can multiplex internally.
- Declared v2: WebPush, Telegram bot, WireGuard mesh, voice (msg 37).

**Chatlog / demo sources.** msg 32 (SSR with async page loading), msg 33 (Axum + Leptos SSR, out-of-order `Suspense`, progressive hydration, SSE + WebSocket, chunk splitting, single binary), msg 37 (PWA/mobile client, Telegram, push: v2), msg 35 (web over gRPC localhost); demo `webui/*` for pages. Corrected: Tauri v2 wrapper dropped; routes list taken from workspace.toml plus `/context-map` and `/parallel-dag` (msg 23).

### `polaris-daemon` — headless kernel daemon `polarisd`: local named pipe / UDS or remote TCP + mTLS

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-daemon` | L7 | app | none | P9 |

**Purpose.** Hosts one kernel instance and serves it through `polaris-rpc` so that the CLI (`--connect`), the desktop and the web server can share sessions, sandboxes and caches. It is also the "Remote Headless Daemon" from the chatlog: the same binary can run on a Nebius AI Cloud VM (msg 35 "cloud bursting"; msg 37 self-hosting is declared v2) with mTLS and Biscuit tokens. It adds no business logic: it wires config, logging, the composition root, a transport, a shutdown path and a health endpoint.

**API sketch.**
```text
polarisd [--pipe NAME | --uds PATH | --listen ADDR]
         [--tls-cert PATH --tls-key PATH --client-ca PATH]   # mandatory for non-loopback --listen
         [--token-file PATH] [--config PATH]
polarisd token issue --scope <scope> --ttl <duration>          # prints an attenuated Biscuit; root key never printed
polarisd doctor
```
```rust
fn main() -> miette::Result<()>;
async fn serve_daemon(cfg: PolarisConfig, args: DaemonArgs) -> Result<(), DaemonError>;   // builds Kernel<B>, calls polaris_rpc::serve
pub enum DaemonError { /* thiserror: Config, Tls, Bind, AlreadyRunning, Kernel */ }
```

**Dependencies.** Internal: `polaris-kernel`, `polaris-rpc`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `tonic = 0.14.6`, `rustls = 0.23.46`, `clap = 4.6.7`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: argument validation (TLS required off loopback), single-instance lock, token issue command, shutdown sequence, health reporting.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polarisd`.
- [ ] Install the aws-lc-rs provider; enforce `cargo tree -i ring` empty for this binary.
- [ ] Single-instance guard per workspace/user (named pipe `first_pipe_instance` or lock file via `fs4`, which is pinned in workspace.toml but not in this crate's ext list; API to confirm); second start fails with a clear message.
- [ ] Implement `token issue` using `polaris-security-capability` (attenuated, short TTL); root key stored via the secrets vault, never on the command line.
- [ ] Graceful shutdown: signal handling (Ctrl+C / CTRL_BREAK on Windows, SIGTERM on Linux), drain streams, Reaper teardown of sandboxes and workers.
- [ ] Health RPC / `doctor`: kernel up, workers alive, WSL2 gateway reachable (reuses platform/openshell doctors).
- [ ] Remote profile: run as a service (Windows service wrapper is out of scope; systemd unit / container image are v2 deliverables).
- [ ] BLOCKED-ON-ACCESS (Nebius AI Cloud VM + quota): deploy `polarisd` on a VM and connect over the internet with mTLS (execution tier E4 is separate: the remote OpenShell gateway).
- [ ] v2 (declared): musl static binary, Nix flake / Docker Compose, embedded WireGuard mesh, Telegram gateway.

**Tests.**
| Category | Cases |
|---|---|
| unit | argument validator table; token scope parser |
| smoke | start on a temp pipe name, `polaris --connect` runs `/status`, SIGINT-equivalent stops cleanly (Windows leg; Linux leg in WSL2) |
| security | remote bind without TLS refused; client without token refused; expired token refused; token file permissions documented/checked |
| stress | 100 client connect/disconnect cycles leave no leaked pipe instances or tasks (measure with Reaper leak report) |
| load | N clients streaming events (shared with `polaris-rpc` load suite) |
| spike | `polaris-spike-web` reuses the daemon as its target |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Daemon Argument Validation | function | `validate_args` | `apps/polaris-daemon/src/args.rs` | `loopback_listen_allows_no_tls`, `public_listen_requires_mtls`, `pipe_and_listen_mutually_exclusive`, `missing_key_file_is_error` |
| Single Instance Guard | function | `acquire_instance` | `apps/polaris-daemon/src/instance.rs` | `second_instance_refused`, `lock_released_on_drop`, `stale_lock_recovered` |
| Token Issuance | function | `issue_token` | `apps/polaris-daemon/src/token.rs` | `ttl_enforced`, `scope_attenuated_not_widened`, `root_key_never_printed` |
| Daemon Shutdown | async_function | `shutdown` | `apps/polaris-daemon/src/shutdown.rs` | `drains_streams_then_reaper`, `second_signal_forces_exit_after_grace` |

**Risks & open questions.**
- VERIFIED (plan): default named-pipe DACL is too broad, so token auth is mandatory even locally.
- UNVERIFIED: Windows graceful stop is limited (no SIGTERM); CTRL_BREAK where available, then kill (plan).
- UNVERIFIED: "latency of a few milliseconds" to a remote H100 daemon (msg 35) is a target, not measured; also the chatlog's "8x H100 instance" is not verified as available to this account.
- LIKELY: a long-lived daemon makes session-state restart (`/session resume`) and event-log retention more important; coordinate with `polaris-storage-eventlog`.
- Open: one daemon per user vs. per workspace.
- P9 placement: the daemon is late; v1 demo uses the embedded kernel.

**Chatlog / demo sources.** msg 35 ("Remote Headless Daemon (Cloud Bursting)", mTLS, gRPC daemon), msg 37 (`polaris-daemon` headless gRPC core; self-hosting, NixOS/Docker, WireGuard, resume: all v2). Corrected: the chatlog crate name `polaris-server` vs `polaris-daemon`: the workspace names the app `polaris-daemon` with binary `polarisd`.

### `polaris-worker` — supervised native worker process, one OS process per mode: `s1` (ort inference), `ingest` (parsers + OCR), `nano` (Needle 3 bridge)

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-worker` | L7 | app | none | P4 |

**Purpose.** Isolates the FFI-heavy and crash-prone work from the kernel process (decision D3: `panic = "unwind"` plus native work in supervised children). `--mode s1` hosts the GLiNER2 engine (Decide plus PII-NER) on ONNX Runtime; `--mode ingest` runs document parsers and OCR on untrusted files (the most crash-prone); `--mode nano` bridges the optional Needle 3 sidecar. It serves the `WorkerService` gRPC API over a Windows named pipe, reports health and memory to the supervisor, lazy-loads models, and evicts them on idle TTL, memory pressure or a force-evict request before heavy builds. A crash kills only this process; the kernel's supervisor restarts it within intensity limits.

**API sketch.**
```text
polaris-worker --mode s1|ingest|nano --pipe <NAME> [--config PATH] [--idle-ttl 300s] [--rss-limit 3GiB]
#### Parent passes the one-time auth token through an inherited handle or stdin, not argv (argv is readable by other local processes).
```
```rust
fn main() -> miette::Result<()>;
enum Mode { S1, Ingest, Nano }
async fn serve_mode(mode: Mode, cfg: WorkerConfig, pipe: PipeName, token: AuthToken) -> Result<(), WorkerError>;
trait ModeHandler { /* decide / ingest / health / evict, implemented per mode by the engine crates */ }
pub enum WorkerError { /* thiserror: Pipe, Auth, ModelLoad, Ingest, Evicted, OutOfMemory */ }
// gRPC: polaris.v1.WorkerService { Health, Decide, Ingest (server-stream), Evict, Shutdown } (see polaris-rpc outline)
```

**Dependencies.** Internal: `polaris-system-one-engine`, `polaris-system-one-nano`, `polaris-ingest`, `polaris-proto`, `polaris-config`, `polaris-log`. External: `tokio = 1.54.0`, `tonic = 0.14.6`, `ort = =2.0.0-rc.13` (workspace.toml; the PLAN core (§0–§12) text says rc.15, a discrepancy to resolve in the dependency probe), `clap = 4.6.7`. The ORT DLL is loaded dynamically from beside the executable, never from System32.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: mode dispatch, named-pipe server with token auth, health/memory reporting, eviction policy, ingest isolation limits.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris-worker`.
- [ ] Reuse the pipe acceptor from `polaris-rpc` or define it in `polaris-proto`-level helper; avoid duplicating the pre-created-next-instance logic (open: which crate owns the shared pipe helper, since the worker does not depend on `polaris-rpc`).
- [ ] Authenticate the supervisor: one-time token via inherited handle; reject any other peer; no secrets on argv.
- [ ] `--mode s1`: load engine lazily on first `Decide`, batch requests, return probabilities; honour per-request deadline so kernel fail-open/closed policy is deterministic.
- [ ] Eviction: idle TTL, memory-pressure signal from the supervisor, `Evict { force }` before cargo/nvcc builds; unload really frees RSS (measure).
- [ ] `--mode ingest`: wrap each document in a limit set (max bytes, max pages, wall-clock timeout); child-of-worker or in-process catch only for task panics; crashes are expected and recovered by the supervisor.
- [ ] `--mode nano`: supervise the Needle engine (`--serve`) or Python package; keep disabled until spike S0-6.
- [ ] Heartbeat/health to the supervisor; exit code taxonomy (clean, auth failure, model load failure, OOM).
- [ ] Accept shutdown (graceful) and exit when parent pipe closes (no orphan workers).
- [ ] BLOCKED-ON-ACCESS (Hugging Face model download and the S0-4 ONNX export): real model inference; before that, run engine tests on tiny synthetic ONNX graphs.
- [ ] BLOCKED-ON-ACCESS (CUDA EP on GTX 1650): GPU path measured only on the dev machine; CPU path first.

**Tests.**
| Category | Cases |
|---|---|
| unit | mode parsing; eviction policy table (idle, pressure, force); exit-code mapping |
| contract | `WorkerService` descriptor snapshot; `Health` reports mode and RSS |
| smoke | spawn each mode, `Health` OK within a deadline, `Shutdown` exits 0 (nano marked SKIPPED until S0-6) |
| security | wrong token refused; token absent from process command line; second client refused |
| fuzz | WSL2 nightly: ingest parsers on mutated docx/xlsx/pdf/pptx/html (crash must not escape the worker) |
| stress | crash loop: kill the worker 50 times under load, supervisor restarts within intensity limits, circuit breaker opens, DLQ receives work; memory pressure forces eviction |
| load | `Decide` batch throughput and latency with hdrhistogram; S1 batch sizes 1/8/32 (targets to measure; vendor CPU latency is about 167 ms on 48 cores, so the chatlog's under-15 ms is NOT a target) |
| spike | `polaris-spike-s1` bursts decisions and kills the worker mid-burst |
| fixtures | tiny synthetic ONNX models and labelled Decide cases; PII samples generated at test time |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Worker Mode Dispatch | function | `serve_mode` | `apps/polaris-worker/src/mode.rs` | `s1_mode_starts_engine_lazily`, `unknown_mode_rejected`, `nano_mode_disabled_without_engine` |
| Worker Pipe Auth | function | `authenticate_peer` | `apps/polaris-worker/src/auth.rs` | `correct_token_accepted`, `wrong_token_rejected`, `token_not_in_argv` |
| Model Eviction | struct | `EvictionPolicy` | `apps/polaris-worker/src/evict.rs` | `idle_ttl_unloads`, `pressure_unloads_least_recent`, `force_evict_before_build`, `reload_after_evict_works` |
| Ingest Limits | function | `ingest_with_limits` | `apps/polaris-worker/src/ingest_limits.rs` | `oversized_file_rejected`, `timeout_kills_parse`, `parser_panic_reported_not_propagated` |

**Risks & open questions.**
- VERIFIED (plan, crates.io probe): GLiNER2.5-Decide ships as 1.95 GB fp32 safetensors with no ONNX file; the S0-4 export is unproven. Fallback: supervised Python `gliner2` sidecar.
- VERIFIED (plan): vendor CPU latency about 167 ms on 48 cores; the machine is a Ryzen 7 2700 with 16 GB RAM and a GTX 1650 4 GB, so S1 latency budgets must be measured, and eviction matters.
- VERIFIED (plan): named pipes need a custom connector and token auth; ORT DLL beside the exe.
- RESOLVED (VERIFIED by the resolution probe): `ort` is pinned `=2.0.0-rc.13` because `oar-ocr-core 0.10` requires exactly rc.13 and `ort-sys` declares `links` (one version per graph); `gliner2-rs 0.9.6` accepts it (requires >= rc.13). The earlier rc.15 figure in the draft plan is superseded (PLAN ADR-015).
- UNVERIFIED: whether killing a worker mid-request leaves ORT/CUDA state clean (process isolation should make this safe; test in stress).
- Open: one worker process per mode means three ORT-capable processes; memory budget per mode needs a number from measurement.
- Open: Windows Job Object memory limits for the ingest worker (via the supervisor's process-wrap), rather than relying on cooperative checks.

**Chatlog / demo sources.** msg 1 (System 1 as a local ONNX engine), msg 33 (Needle 3 as tier 0), msg 38 (user request: a model that is not in use is switched off to free memory for others), plan D3. Corrected: the msg-1 "15-40 ms" and msg-33 "under 25 ms" S1 latency are not supported by the vendor figure.

### `polaris-guest` — Linux in-sandbox agent: protocol handshake, structured exec, sync endpoint, browser screenshots and a11y trees

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `apps/polaris-guest` | L7 | app | none | P7 |

**Purpose.** A small Linux binary that runs inside the Polaris sandbox image (built in WSL2, baked into the OpenShell image by `polaris-openshell-nemoclaw`). It serves `GuestService` to the host: a `protocol_version` handshake first, then structured command execution (stdout/stderr/exit as events), byte-exact sync delta application, and, for the browser tier, Chromium screenshots and accessibility-tree outlines through `polaris-browser`. It only accepts host-initiated connections (the plan allows only host-initiated traffic) and holds no secrets. It is excluded from Windows `default-members` and built in WSL2/Linux only. In the MVP, sandboxed `cargo test`/`pytest` can run through OpenShell exec without it; the guest becomes required for the browser/screenshot features (P7).

**API sketch.**
```text
polaris-guest [--listen <ENDPOINT>] [--workspace /sandbox/work] [--protocol 1]
#### Endpoint transport is decided by spike S0-12 (UNVERIFIED): candidates are an OpenShell exec/stdio channel,
#### a policy-allowed TCP port, or vsock if the libkrun vm driver exposes it.
```
```rust
fn main() -> miette::Result<()>;
const PROTOCOL_VERSION: u32;                                      // from polaris-proto
async fn handshake(req: HandshakeRequest) -> Result<HandshakeResponse, GuestError>;      // mismatch => fail closed, no other RPC served
async fn exec(req: ExecRequest) -> Result<impl Stream<Item = ExecEvent>, GuestError>;   // structured output, timeouts, process-tree kill
async fn apply_sync(deltas: impl Stream<Item = SyncDelta>) -> Result<SyncAck, GuestError>; // blake3 verified, byte-exact, path-confined
async fn screenshot(req: ScreenshotRequest) -> Result<ScreenshotResponse, GuestError>;
async fn a11y_tree(req: A11yRequest) -> Result<A11yResponse, GuestError>;
pub enum GuestError { /* thiserror: VersionMismatch, PathEscape, Exec, Browser, Sync */ }
```

**Dependencies.** Internal: `polaris-sdk`, `polaris-proto`, `polaris-browser`, `polaris-log`. External: `tokio = 1.54.0`, `chromiumoxide = 0.9.1` (feature bytes), `prost = 0.14.4`. The server side of tonic is needed for `GuestService`; `tonic` is not in this crate's `ext` in workspace.toml (it arrives transitively through `polaris-proto`): add it explicitly via the TOML if the direct dependency is required.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: handshake, path confinement, exec event stream, sync apply, screenshot request limits.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris-guest`; keep it Linux-only with `cfg(target_os = "linux")` at the module level so Windows `cargo check` of the skeleton stays valid (or exclude from default-members per plan).
- [ ] Spike S0-12: build in WSL2, bake into a sandbox image, start through OpenShell, perform the handshake from the Windows host; record the transport that works.
- [ ] Implement handshake: refuse every RPC until a compatible `protocol_version` is accepted; include build id; fail closed on mismatch.
- [ ] Implement `exec`: run in the workspace dir, no shell injection (argv vector, not string, where possible), wall-clock and output-size limits, kill the whole process tree on timeout (Linux process groups), exit code and signal reported.
- [ ] Implement `apply_sync`: verify blake3 per file, write atomically, preserve bytes exactly, reject `..`, absolute paths, symlink escapes and case/reserved-name clashes; report conflicts instead of overwriting.
- [ ] Implement `screenshot` and `a11y_tree` through `polaris-browser` contract (Chromium headless via chromiumoxide; Xvfb only if headful is required); cap PNG size; no network access outside policy.
- [ ] Add resource self-limits (memory, open files) and orderly shutdown on parent disconnect.
- [ ] Image notes: guest runs as non-root, read-only root where possible, no keys mounted; secrets reach nothing in the sandbox (provider-profile substitution happens at the OpenShell gateway).
- [ ] BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 + OpenShell v0.1.4 gateway): build, bake, handshake and every smoke test below.
- [ ] BLOCKED-ON-ACCESS (nested KVM, spike S0-2): only for the vm-driver variant.

**Tests.**
| Category | Cases |
|---|---|
| unit | handshake version table; path-confinement function on hostile path set; output truncation markers |
| proptest | any sequence of sync deltas leaves the workspace equal to the manifest or returns an error with no partial file; confinement: no generated path resolves outside the root |
| contract | `GuestService` descriptor snapshot; host `GuestClient` against an in-process guest |
| smoke | (WSL2) guest starts, handshake OK, `exec echo` returns exit 0, screenshot of a local static page returns a PNG header |
| security | traversal / symlink / reserved-name corpus (generated at test time, never committed); exec output injection of terminal escapes is escaped by the host UI, not the guest; no env secrets visible to the command |
| fuzz | WSL2 nightly: gRPC frames into the guest, sync delta decoder, a11y tree serializer |
| stress | 1k short `exec` calls in a row without fd or process leaks; parent disconnect kills children |
| spike | `polaris-spike-sandbox` drives create/exec/destroy bursts through the guest when present |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Guest Handshake | async_function | `handshake` | `apps/polaris-guest/src/handshake.rs` | `matching_version_accepted`, `older_host_rejected`, `newer_host_rejected`, `rpc_before_handshake_refused` |
| Path Confinement | function | `confine` | `apps/polaris-guest/src/confine.rs` | `dotdot_rejected`, `absolute_path_rejected`, `symlink_escape_rejected`, `case_clash_reported`, `reserved_name_reported` |
| Sync Apply | async_function | `apply_sync` | `apps/polaris-guest/src/sync.rs` | `hash_mismatch_aborts_file`, `bytes_preserved_exactly`, `atomic_write_no_partial_file`, `crlf_not_translated` |
| Structured Exec | async_function | `exec` | `apps/polaris-guest/src/exec.rs` | `reports_exit_code`, `timeout_kills_process_tree`, `output_cap_truncates_with_marker`, `stderr_separate_from_stdout` |

**Risks & open questions.**
- UNVERIFIED (S0-12): the host-to-guest transport. The chatlog's vsock/Firecracker design (msg 25, 35) is replaced by the OpenShell sandbox (D1/D2); whether the libkrun `vm` driver offers vsock is unknown. Docker driver in WSL2 offers exec/network channels.
- VERIFIED (plan): the OpenShell Windows `mxc` driver has no interactive exec and no OCI images; the guest is Linux/WSL2-only.
- UNVERIFIED: Chromium inside the Polaris sandbox image: size, GPU-less headless operation and sandbox flags under Landlock; spike required.
- UNVERIFIED: chromiumoxide 0.9.1 API for a11y snapshots (`Accessibility` domain) - confirm at implementation; the chatlog's "80,000 DOM tokens to 600 tokens" is an illustration, not a measurement.
- LIKELY: the guest is a high-value target (it executes untrusted code on the host's behalf); confinement and handshake must be fuzzed.
- Declared P7; the MVP does not depend on it.

**Chatlog / demo sources.** msg 25 (`polaris-guest-daemon`, Xvfb :99, framebuffer capture, headless Chromium, CDP via chromiumoxide, a11y tree, pHash `/ui-diff`), msg 35 (gRPC over vsock to the guest), plan (`protocol_version` handshake; guest built in WSL2). Corrected: Firecracker/AF_VSOCK dropped; the name `polaris-guest-daemon` becomes `polaris-guest`; "Linux only" build leg.

### `polaris-tests` — cross-crate test package: common helpers (lib), fixtures, suites (smoke, e2e, contract, security, load, scalability, stress), MVP acceptance test

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests` | L8 | test | none | P0 |

**Purpose.** One package for everything that spans crates. The library half (`tests/src`) is the `common` toolbox: temp workspaces, a controllable clock, a mock Token Factory, tracing capture, a counting allocator, a PTY/ConPTY harness, hostile-fixture generators and histogram helpers. The suites live in `tests/tests/*.rs`; heavy ones are gated behind nextest profiles or the `heavy` feature so `cargo test --workspace` stays fast. Fixtures live in `tests/fixtures` with provenance and licence per file; anything hostile or secret-shaped is generated at test time and never committed, to avoid antivirus quarantine and secret scanners. Per-crate unit tests stay in their crates; this package never re-tests internals.

**Test-suite layout.**
```text
tests/                                  package polaris-tests (lib + integration suites)
  Cargo.toml                            features: heavy, keyed, wsl, conpty (all off by default)
  src/lib.rs                            pub mod common;
  src/common/                           helpers (table below)
  tests/smoke.rs                        binaries start, doctor, TUI --dump, daemon health, WSL2 sandbox create/exec/destroy
  tests/e2e.rs                          MVP scripted demo, session resume/replay, directive round trips
  tests/contract.rs                     SSE parser vs fixtures, proto descriptor snapshots, tool schemas, policy YAML schema, UI metadata vs directives grammar
  tests/security.rs                     injection corpora, PII-egress capture, capability attenuation, secrets-never-leak, sync deny-list
  tests/load.rs                         (heavy) bus, S1 batches, SSE parse, storage, rate limiter with hdrhistogram
  tests/scalability.rs                  (heavy) 1 to 32 frames; 1k / 10k / 100k-file repos
  tests/stress.rs                       (heavy) memory pressure with eviction, worker crash loops, sandbox exhaustion, disk full
  fixtures/                             committed data with PROVENANCE.md (see catalogue)
  spike/{ratelimit,bus,s1,sandbox,web}  five separate burst-test packages (documented below)
  fuzz/                                 README only now; excluded from the workspace; cargo-fuzz in WSL2 nightly
.config/nextest.toml                    profiles: default, ci, heavy, keyed, wsl, conpty (cargo-nextest not installed yet: P0)
```
A `[[test]]` entry exists only for a file that exists. Heavy files start with `#![cfg(feature = "heavy")]` (a cfg gate, not a lint attribute). Tests needing a key or a gateway are `#[ignore = "<reason>"]` and run only by their profile, so they show as ignored, never as passed: **a missing key can never produce a green check**.

**Common helpers (`tests/src/common`).**
| Helper | Purpose |
|---|---|
| `temp_workspace()` | `tempfile` dir with optional git init and a fixture repo copied in; short path under the cargo `target-dir` to avoid the 260-char limit |
| `FakeClock` / `tokio::time::pause` wrapper | deterministic time for rate limiter, TTLs, GCRA, idle eviction |
| `MockTokenFactory` (wiremock 0.6.5) | OpenAI-compatible server: streaming SSE, tool-call deltas, reasoning deltas, usage with and without cached tokens, `429` with `Retry-After`, rate-limit headers (as a configurable guess until S0-5), scripted multi-turn conversations. Everything it emits is labelled SYNTHETIC |
| `MockEmbeddings`, `MockTavily` | deterministic embeddings and search responses |
| `TraceCapture` | tracing subscriber capturing events for assertions (and for "no secret in logs" checks) |
| `OutboundCapture` | records every request body the mock servers receive (PII-egress and secret-egress tests) |
| `CountingAlloc` + `assert_no_alloc(|| ...)` | zero-alloc hot-path checks; see allocator risk below |
| `PtyHarness` | spawn the real binary in a pseudo-terminal (ConPTY on Windows) and write terminal bytes; reads back a screen model; needs `portable-pty` (pinned in workspace.toml, not in this package's ext list) |
| `hostile::paths()`, `hostile::fake_keys()`, `hostile::malicious_diffs()`, `hostile::injection_corpus()` | generated at call time, never written to the repo tree |
| `gen_repo(files, depth)` | synthetic repos of 1k / 10k / 100k files for scalability |
| `Hist` | hdrhistogram wrapper: record, p50/p99/p99.9/max, JSON report, budget assert that prints the measured value |
| `skip_reason()`, `require_env()` | helpers for ignore-with-reason tests and explicit `SKIPPED: <reason>` output |
| `events::script(path)` | loads a JSON event script into `sdk::Event`s for UI and replay tests |
| `golden::dump(cmd, WxH)` | runs `polaris --dump WxH` and returns text for insta |

**Fixtures catalogue.**
| Directory | Committed content | Notes |
|---|---|---|
| `fixtures/sse/` | synthetic Token Factory streams: plain text, tool call split across deltas, reasoning deltas, `[DONE]`, usage-only final chunk, CRLF framing, multi-byte character split across chunks, malformed JSON line, oversized line | header comment `SYNTHETIC: not recorded from a live endpoint`; re-record when keys exist; contract tests cannot report PASS against reality |
| `fixtures/repos/` | `rust-slugify`, `py-dedupe`, `cuda-reduce` (MVP tasks), `rust-workspace-small`, `git-history-small` | tiny, human-written, permissive licence; `PROVENANCE.md` per repo |
| `fixtures/docs/` | small docx, xlsx, csv, pdf (text + scanned image with known text), pptx, html, md | hand-made or generated by us; no third-party documents unless licence allows |
| `fixtures/directives/` | valid and invalid `$` / `/` / `@` inputs, nested parentheses, quoted lists, effort ranges, both `=[a,b]` and `(a,b)` spellings, code-span exclusions | shared with the kernel-directives fuzz seeds |
| `fixtures/policies/` | OpenShell policy YAML (valid, over 256 fs paths, bare-root rw, unknown fields), blueprint YAML | schema limits from the plan |
| `fixtures/rtk/` | `cargo --message-format=json` samples, text samples, duplicate spans, registry notes | for the RTK filter |
| `fixtures/ui/` | JSON event scripts (welcome, conversation, error, goal, tool expand) | golden `--dump` snapshots stay beside their crates (insta) |
| `fixtures/s1/` | labelled Decide cases (conservative start), tiny synthetic ONNX graphs for engine plumbing | Python `gliner2` goldens are generated by `xtask fixtures` (BLOCKED-ON-ACCESS: Hugging Face download) |
| `fixtures/mvp/` | scripted model turns (SSE) for the demo, expected event sequences, the falsified-completion turn | see acceptance test |
| generated at test time | reserved names (CON, NUL, trailing dot/space), case clashes, `..` traversal, over-long paths, fake keys and PEM-like blobs, malicious diffs, prompt-injection strings, PII samples | never committed |

**MVP scripted-demo acceptance test (`tests/e2e.rs::mvp_scripted_demo`).** Definition of done for the end of P4 (plan: "MVP = scripted demo").
1. Three fixed tasks run in sequence in one session: **Rust** (`rust-slugify`: failing unit test, fix in `src/lib.rs`), **Python** (`py-dedupe`: failing pytest, fix in module), **CUDA-flavoured** (`cuda-reduce`: a `.cu` reduction with a `__syncthreads()` inside a divergent branch plus a CPU reference implementation). Verification for CUDA is: the local zero-token static audit flags the divergent barrier before the fix and not after; the CPU reference test compiles with the sandbox's host compiler and passes; an `nvcc` compile runs only if the image has `nvcc` (UNVERIFIED) and is otherwise reported `SKIPPED`, never PASS.
2. Each task: edit by the model, tests run **inside a sandbox** (OpenShell in WSL2, or Contree via MCP, or the E4 remote gateway as fallback for judges without WSL2), `ProofOfDone` passes, then the task reaches `Completed`.
3. **Falsified completion is blocked.** In a scripted turn the model claims success while having commented out a failing assertion (or marked it `#[ignore]` / `@pytest.mark.skip`) or without running tests. Required: the event stream contains `CompletionAttempted` then `CompletionBlocked { reason }` and never `Completed` for that attempt; the process exit code of `polaris run` is non-zero for a run that ends there; after the model's next (honest) turn the gate passes.
4. The TUI streams the session: `--dump WxH` snapshots at fixed checkpoints (task start, tool expanded, blocked completion, final) match insta goldens; `/diff`, `/stats` and `/status` open and close.
5. The transcript is recorded (JSONL events plus final ViewState hash); `/session replay` reproduces the same hash.
6. Secret hygiene: the transcript, logs and captured outbound bodies contain none of the generated fake keys or PII samples; sandbox environment contains no key.
Modes: **scripted** (MockTokenFactory, deterministic, keyless; needs a real sandbox to count as PASS, and is `#[ignore]` without WSL2) and **live** (`#[ignore = "needs NEBIUS_API_KEY"]`, real Nemotron on Token Factory, three runs recorded with raw output). Time budget for the scripted run is a target to measure (not asserted until measured).

**Dependencies.** Internal: `polaris-sdk`, `polaris-config`, `polaris-storage` (as listed in workspace.toml). Needed in addition, to be added through the TOML: `polaris-kernel`, `polaris-ui-cli`/`polaris-ui-model`, `polaris-kernel-directives`, `polaris-ratelimit`, `polaris-system-one-client` (e2e, contract and load suites); binaries from other packages (`polaris`, `polarisd`, `polaris-worker`) must be built before the suites run (`xtask test` does this), because `assert_cmd` does not build other packages. External: `tokio = 1.54.0`, `tempfile = 3.27.0`, `wiremock = 0.6.5`, `proptest = 1.12.0`, `insta = 1.49.0`, `rstest = 0.28.0`, `assert_cmd = 2.2.2`, `predicates = 3.1.4`, `hdrhistogram = 7.6.0`; proposed: `portable-pty = 0.9.0`, `serde_json = 1.0.154`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: temp workspace, MockTokenFactory, CountingAlloc, hostile generators, histogram budget assert, MVP demo runner.
- [ ] Keep `tests/src/lib.rs` as the lib; add `common` modules one by one, each with its own unit tests and no panics (return `Result`).
- [ ] Write `.config/nextest.toml` profiles (default, ci, heavy, keyed, wsl, conpty) once cargo-nextest is installed (P0); until then document `cargo test --features ...` equivalents.
- [ ] Add fixtures with `PROVENANCE.md` and licence per directory; add `.gitattributes`/`.gitignore` negations so fixtures are tracked and generated hostile data never is.
- [ ] Implement `MockTokenFactory` and the SSE fixtures; label SYNTHETIC; add the re-record procedure (a `keyed` test that captures a live stream into a fixture draft) - BLOCKED-ON-ACCESS (Token Factory key).
- [ ] Implement `PtyHarness` and port the demo's ConPTY method (spawn the real exe, write bytes, check cell grid): Shift+Enter CSI-u, Shift+Tab, bracketed paste, Esc/Return in popups, Ctrl+C twice (feeds S0-10).
- [ ] Implement `smoke.rs`: version/help, `doctor`, `--dump`, daemon health (P9), WSL2 sandbox create/exec/destroy (ignored without a distro).
- [ ] Implement `contract.rs`: SSE parser vs fixtures, proto descriptor snapshots, UI command metadata vs `polaris-kernel-directives`, policy schema.
- [ ] Implement `security.rs`: injection corpus, PII-egress capture (assert masked values never appear in outbound bodies), attenuation, secrets-never-in-logs/argv/env, sync deny-list.
- [ ] Implement `load.rs`, `scalability.rs`, `stress.rs` behind `heavy`; every budget printed with the measured value and conditions (CPU, RAM, driver, versions).
- [ ] Implement the MVP scripted demo (scripted mode first; add the live mode as ignored).
- [ ] Implement zero-alloc helpers on `dhat` 0.3.3 testing mode (ADR-003): one integration-test binary per hot-path check.
- [ ] BLOCKED-ON-ACCESS (WSL2 Ubuntu + OpenShell): sandbox smoke, MVP scripted mode PASS, sandbox exhaustion stress. BLOCKED-ON-ACCESS (Token Factory key): live mode, SSE re-recording. BLOCKED-ON-ACCESS (Contree key): Contree variant of the demo.

**Tests.** (Meta-tests of the harness itself; the suites above are the product.)
| Category | Cases |
|---|---|
| unit | `Hist` percentile math; hostile path generator yields the documented classes; `temp_workspace` cleans up; MockTokenFactory serves scripted streams in order |
| proptest | generated repos have exactly the requested file count; mock SSE encoder/decoder round trip |
| insta snapshot | harness-level only (screen model rendering) |
| smoke | the harness smoke: `polaris --version` through `assert_cmd` |
| fixtures | provenance check: every fixture directory has `PROVENANCE.md`; no generated hostile pattern is present in the git tree (grep gate in `xtask`) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Temp Workspace Builder | function | `temp_workspace` | `tests/src/common/workspace.rs` | `creates_isolated_dir`, `copies_fixture_repo`, `cleans_up_on_drop`, `path_stays_under_limit` |
| Mock Token Factory | struct | `MockTokenFactory` | `tests/src/common/token_factory.rs` | `streams_scripted_chunks_in_order`, `emits_429_with_retry_after`, `tool_call_split_across_deltas`, `records_outbound_bodies`, `marked_synthetic` |
| Hostile Fixture Generator | module | `hostile` | `tests/src/common/hostile.rs` | `reserved_names_generated`, `case_clash_pairs_generated`, `never_written_inside_repo_tree`, `fake_keys_match_scanner_patterns` |
| MVP Scripted Demo | pipeline | `mvp_scripted_demo` | `tests/tests/e2e.rs` | `three_tasks_complete_after_proof_of_done`, `falsified_completion_is_blocked`, `blocked_run_exits_nonzero`, `transcript_replay_hash_matches`, `no_secret_in_transcript_or_outbound_bodies` |

**Spike packages (`tests/spike/*`; burst scenarios and pass criteria; criteria are targets to calibrate with the first measurement, never loosened afterwards to pass).**
| Package (phase) | Burst scenario | Pass criteria |
|---|---|---|
| `polaris-spike-ratelimit` (P3) | 50 sub-agents request at once with mixed priorities (InteractiveUser, Verifier, SubAgent, Background) against a configured RPM/TPM; then a 429 storm with `Retry-After` 1 s to 30 s; then a Retry-After flood (many identical headers); then token-weighted TPM exhaustion mid-burst | InteractiveUser requests are never starved by lower priorities; admitted rate never exceeds configured RPM/TPM by more than the documented burst; Background is shed first near limits; no request waits longer than its deadline without an explicit error; jittered backoff keeps thundering-herd retry spread (measured); p99 `acquire` latency recorded with hdrhistogram; no task leak after the burst |
| `polaris-spike-bus` (P1) | 1M event publishes with 1 / 8 / 64 subscribers, one deliberately slow; command flood into the bounded mpsc; UI-client flood (N subscribers connect and drop repeatedly) | no lost commands (or an explicit backpressure error for each rejected one); memory stays bounded (RSS delta recorded); the slow subscriber is lagged/dropped without slowing others; publish p99 recorded; zero-alloc publish proven in the allocator test, not asserted by the spike |
| `polaris-spike-s1` (P4) | 500 decisions in 1 s with batching; worker killed mid-burst; cache stampede on 100 identical inputs; timeout path per consumer | batching reduces model invocations (count recorded); stampede computes once; fail-closed consumers (safety gate, PII) return the deterministic DFA path within their timeout, fail-open consumers proceed after timeout; supervisor restarts the worker within intensity limits and the next batch succeeds; no unbounded queue |
| `polaris-spike-sandbox` (P2) | 20 concurrent sandbox create/exec/destroy cycles; `git checkout` of a 10k-file branch during sync producing a file-event flood (notify overflow must force a rescan); sandbox quota exhaustion | semaphore caps concurrent sandboxes; Reaper leaves zero leaked sandboxes/temp dirs/pipes (leak report empty); after the flood the sandbox manifest equals the workspace manifest (blake3), including after an overflow-forced rescan; requires the WSL2 gateway (BLOCKED-ON-ACCESS until installed) |
| `polaris-spike-web` (P9) | 1k idle SSE clients, then 256 clients each receiving a token stream; connect/disconnect churn; WebSocket terminal flood | p99 event latency and RSS per client recorded; slow clients get snapshot-or-drop, never unbounded buffers; kernel broadcast unaffected by a stalled client; TTFB and time-to-shell reported as measured with browser/OS conditions (the chatlog's "TTFB < 2 ms" is a hypothesis to test) |

**Risks & open questions.**
- VERIFIED (plan + AGENTS.md): a test that cannot fail is a lie; skipped keyed tests must show as ignored/SKIPPED, and fixtures from synthetic SSE must stay labelled synthetic.
- RESOLVED at merge: zero-alloc checks use the vetted `dhat` 0.3.3 testing mode (`dhat::Alloc` + `HeapStats`), one integration-test binary per check (ADR-003); no own `unsafe`. Original concern, kept for the record: a hand-written `GlobalAlloc` wrapper needs `unsafe impl`, which is not in the declared unsafe-exception list. Options: declare a test-only exception, use a vetted allocator-stats crate (none is pinned in workspace.toml), or measure allocation with a profiler-free alternative. Decide before the first zero-alloc test.
- LIKELY: `assert_cmd` does not build binaries from other packages; `xtask test` must build first or tests must run under `cargo nextest` with a workspace build step.
- UNVERIFIED: cargo-nextest, cargo-fuzz, cargo-mutants are not installed (plan, machine facts); profiles are untested.
- UNVERIFIED: AV behaviour on ConPTY-spawning test binaries and generated hostile files on this machine.
- Open: should fixtures from the demo repo (golden `--dump` text) be re-derived or ported? Plan says design only; derive fresh snapshots from our renderer.

**Chatlog / demo sources.** Plan test strategy; msg 17 (backpressure/concurrency pools), msg 31 (Proof-of-Done: compile clean, tests pass, S1 verifies assertions not falsified or commented out), msg 23 (parallelism), msg 35/33 (hacker audit with 0 tokens); demo `docs/keyboard-verification.md` and `color-policy.md` (real-executable-under-ConPTY method, styled-cell comparison instead of escape sequences, private environment blocks for child processes). Corrected: the demo's "217 / 228 tests passed" figures are the demo's own evidence and say nothing about Polaris.

### `polaris-spike-ratelimit` — burst tests for the rate limiter: 50 sub-agents at once, 429 storms, Retry-After floods

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests/spike/ratelimit` | L8 | spike | none | P3 |

**Purpose.** A "spike test" here means a sudden load burst, not a research spike. This package drives `polaris-ratelimit` (`DualLimiter`: GCRA RPM plus token-weighted TPM, priorities, Retry-After, 15-minute window tracking) with the traffic shapes the chatlog implies (many sub-agents, 429 storms) and asserts ordering, bounded latency and no leaks. It uses a fake clock for deterministic scenarios and a real-time run for latency histograms. No network or key is needed: the "server" is a scripted limiter oracle or the `MockTokenFactory` from `polaris-tests`.

**API sketch.** (test harness only; the package exports nothing)
```rust
struct Burst { agents: usize, priorities: Vec<Priority>, est_tokens: Range<u32>, deadline: Duration }
async fn run_burst(limiter: &DualLimiter, burst: &Burst, clock: &impl Clock) -> Result<BurstReport, SpikeError>;
struct BurstReport { admitted: Vec<Admission>, shed: Vec<Shed>, wait_hist: Hist, max_rate_rpm: f64, leaked_tasks: usize }
fn assert_priority_order(report: &BurstReport) -> Result<(), SpikeError>;      // returns an error with the measured values, no panic
```

**Dependencies.** Internal: `polaris-ratelimit` (plus `polaris-tests` helpers once the package may depend on it; add through the TOML). External: `tokio = 1.54.0`, `hdrhistogram = 7.6.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: burst driver, priority-order assertion, 429-storm scenario, TPM-exhaustion scenario.
- [ ] Replace the doc-only lib skeleton with `tests/burst.rs` (+ `[[test]]` only when the file exists); gate under `heavy` if the real-time run exceeds the fast-suite budget.
- [ ] Scenario A: 50 sub-agents in one tick with mixed priorities; assert InteractiveUser and Verifier are admitted before SubAgent and Background, and that admitted rate never exceeds RPM plus the documented burst.
- [ ] Scenario B: 429 storm: server returns `Retry-After` from 1 s to 30 s for 60 s; assert jittered backoff spreads retries (variance above a measured floor) and nothing retries before its Retry-After.
- [ ] Scenario C: TPM exhaustion mid-burst with oversized estimates; assert reconcile-after-usage returns unused budget and later requests proceed.
- [ ] Scenario D: Retry-After header flood (duplicate/contradictory headers, negative, huge values); assert parse errors are typed and clamped, no panic.
- [ ] Record p50/p99 `acquire` latency, shed counts and leaked tasks; write a JSON report to `target/spike/ratelimit.json`.
- [ ] Calibrate the numeric pass criteria after the first run and freeze them; document conditions (CPU, load).
- [ ] BLOCKED-ON-ACCESS (Token Factory key, spike S0-5): replace the assumed header names and 15-minute-window behaviour with the observed ones; scenarios stay valid with the mock until then.

**Tests.**
| Category | Cases |
|---|---|
| spike | scenarios A to D above; 100 repetitions each under the fake clock for determinism |
| load | sustained 10 req/s for 10 min (real clock; ignored by default) |
| stress | burst while the limiter's window rolls over; cancellation of half the waiters mid-burst |
| proptest | for any arrival sequence the admitted count in any 60 s window is at most RPM plus burst |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Priority Burst | async_function | `run_burst` | `tests/spike/ratelimit/tests/burst.rs` | `fifty_agents_priority_order_holds`, `rate_never_exceeds_limit_plus_burst`, `background_shed_first`, `no_leaked_tasks_after_burst` |
| 429 Storm | async_function | `storm_429` | `tests/spike/ratelimit/tests/storm.rs` | `no_retry_before_retry_after`, `jitter_spreads_retries`, `recovers_after_storm_ends` |
| Retry-After Parsing Flood | function | `flood_retry_after` | `tests/spike/ratelimit/tests/flood.rs` | `negative_values_clamped`, `huge_values_capped`, `contradictory_headers_resolved_deterministically` |

**Risks & open questions.**
- VERIFIED (plan): Token Factory rate limits are soft caps in 15-minute windows; header names and behaviour are unverified (S0-5).
- UNVERIFIED: any numeric RPM/TPM used in scenarios is an assumption, not Nebius's limits.
- LIKELY: fake-clock tests prove ordering, not real latency; the real-clock run is needed for the histogram.
- Open: whether the "50 sub-agents" figure from the chatlog is a hard product limit (`$concurrency`) or only a stress shape; treated as a stress shape.

**Chatlog / demo sources.** msg 17/18 (concurrency pools, rate limits), plan ("50-subagent bursts, 429 storms"), prompt.md §5.2 (pre-allocation pools). Corrected: none of the chatlog's numeric limits is used as fact.

### `polaris-spike-bus` — burst tests for the event bus: event floods, slow subscribers, UI client floods

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests/spike/bus` | L8 | spike | none | P1 |

**Purpose.** Hammers `polaris-kernel-bus` (bounded mpsc for commands, broadcast for events) with the load patterns UIs create: publish floods, one stalled subscriber among many, and clients connecting/disconnecting in bulk. It proves backpressure and bounded memory, and gives the first latency histograms for the bus. It is the earliest spike (P1) because every later component rides on the bus.

**API sketch.**
```rust
struct Flood { publishers: usize, events_per_publisher: u64, payload: usize, subscribers: Vec<SubscriberKind> }
enum SubscriberKind { Fast, Slow { delay: Duration }, Stalled, Churning { period: Duration } }
async fn run_flood(bus: &Bus, f: &Flood) -> Result<FloodReport, SpikeError>;
struct FloodReport { published: u64, delivered: Vec<u64>, lagged: Vec<u64>, rejected_commands: u64, rss_delta_bytes: i64, publish_hist: Hist }
```

**Dependencies.** Internal: `polaris-kernel-bus` (plus test helpers via the TOML). External: `tokio = 1.54.0`, `hdrhistogram = 7.6.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: flood driver, subscriber kinds, memory-bound assertion.
- [ ] Replace the doc-only lib skeleton with `tests/flood.rs`.
- [ ] Scenario A: 1M events, 1/8/64 fast subscribers; record publish p50/p99 and delivered counts.
- [ ] Scenario B: add one `Stalled` subscriber; assert the others are unaffected and the stalled one is lagged/dropped with an explicit signal.
- [ ] Scenario C: command flood into the bounded mpsc; every command is either processed or rejected with a typed backpressure error (count adds up exactly).
- [ ] Scenario D: UI churn: 1k subscribers connect and drop in a loop while events flow; no leaked receivers, no panic.
- [ ] Memory: measure RSS before/after with `sysinfo` (pinned in workspace.toml; add to the package ext via the TOML); assert the delta stays under a limit calibrated after the first run.
- [ ] Feed results to the loom model tests for the bus (loom lives with the bus crate, not here).
- [ ] Linux leg: run the same spike in WSL2 to compare scheduling behaviour.

**Tests.**
| Category | Cases |
|---|---|
| spike | scenarios A to D |
| load | 10M events overnight (ignored, `heavy`) |
| stress | publisher panic inside a task (JoinError) while others continue; subscriber that panics on receive |
| proptest | delivered + lagged + rejected equals published for any interleaving |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Event Flood | async_function | `run_flood` | `tests/spike/bus/tests/flood.rs` | `all_events_accounted_for`, `slow_subscriber_does_not_slow_others`, `stalled_subscriber_is_lagged_explicitly`, `memory_bounded_under_flood` |
| Command Backpressure | async_function | `command_flood` | `tests/spike/bus/tests/commands.rs` | `no_command_lost_silently`, `rejections_are_typed`, `accepted_plus_rejected_equals_sent` |
| Subscriber Churn | async_function | `churn` | `tests/spike/bus/tests/churn.rs` | `no_leaked_receivers`, `late_subscriber_misses_only_past_events` |

**Risks & open questions.**
- LIKELY: `tokio::sync::broadcast` drops oldest items for lagging receivers (receiver gets a `Lagged` error); if exact delivery is required for the event log, persistence must be separate from broadcast. Confirm at implementation.
- UNVERIFIED: the zero-allocation-per-publish claim is checked by the allocator test (see `polaris-tests` risk about the allocator wrapper), not by this spike.
- UNVERIFIED: RSS sampling accuracy on Windows with `sysinfo`.
- Open: acceptable lag policy for UI subscribers (drop vs snapshot resync).

**Chatlog / demo sources.** msg 13 (broadcast channel to all frontends), msg 17 (backpressure, no unbounded `tokio::spawn`), msg 23 (bus code sketch), plan (UI client floods).

### `polaris-spike-s1` — burst tests for System One: decision bursts, worker restart under load, cache stampedes

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests/spike/s1` | L8 | spike | none | P4 |

**Purpose.** Exercises `polaris-system-one-client` (batching, xxh3 plus quick_cache result cache, timeouts, worker IPC) against a real `polaris-worker --mode s1` (or a synthetic-model worker when the real model is not available) and asserts the fail-open/fail-closed matrix from the plan: safety gate and PII masking fail closed to the deterministic DFA path; triage, tool pruning, watchdog and sifter fail open after a timeout. The burst shapes: many simultaneous decisions, worker crash mid-burst, identical-input stampede.

**API sketch.**
```rust
enum Consumer { SafetyGate, PiiMask, Triage, ToolPrune, Watchdog, Sifter }
struct DecisionBurst { count: usize, window: Duration, distinct_inputs: usize, consumers: Vec<Consumer> }
async fn run_decisions(client: &S1Client, b: &DecisionBurst, faults: &FaultPlan) -> Result<S1Report, SpikeError>;
struct FaultPlan { kill_worker_at: Option<Duration>, add_latency: Option<Duration> }
struct S1Report { model_invocations: u64, cache_hits: u64, fail_open: u64, fail_closed: u64, restarts: u32, latency: Hist }
```

**Dependencies.** Internal: `polaris-system-one-client` (plus the worker binary built by `xtask`, and helpers via the TOML). External: `tokio = 1.54.0`, `hdrhistogram = 7.6.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: decision burst driver, fault plan, fail-open/closed assertions, stampede check.
- [ ] Replace the doc-only lib skeleton with `tests/decisions.rs`.
- [ ] Build a synthetic ONNX model fixture so the burst can run before the real Decide export exists (S0-4); label results SYNTHETIC-MODEL.
- [ ] Scenario A: 500 decisions in 1 s, assert batching reduces model invocations and all callers get answers or typed timeouts.
- [ ] Scenario B: kill the worker mid-burst; assert supervisor restart within intensity limits, circuit breaker behaviour, and per-consumer fail-open/closed outcomes.
- [ ] Scenario C: stampede of 100 identical inputs; the model runs once and the rest hit the cache (xxh3 key).
- [ ] Scenario D: timeout injection per consumer; SafetyGate/PiiMask return the DFA path, others proceed.
- [ ] Re-run Scenarios A and B with the real model after S0-4; record latency on CPU and on the GTX 1650 with conditions.
- [ ] BLOCKED-ON-ACCESS (S0-4 ONNX export + Hugging Face download): the real-model run.

**Tests.**
| Category | Cases |
|---|---|
| spike | scenarios A to D |
| stress | worker crash loop (kill every 2 s for 2 min); memory pressure forcing eviction mid-burst |
| security | fail-closed consumers never return "allow" on timeout |
| proptest | for any fault timing, every request resolves exactly once (answer, typed timeout, or typed error) |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Decision Burst | async_function | `run_decisions` | `tests/spike/s1/tests/decisions.rs` | `batching_reduces_invocations`, `every_request_resolves_exactly_once`, `latency_recorded` |
| Worker Crash Recovery | async_function | `crash_mid_burst` | `tests/spike/s1/tests/crash.rs` | `worker_restarted_within_intensity`, `fail_closed_consumers_never_allow`, `fail_open_consumers_proceed_after_timeout`, `breaker_opens_on_crash_loop` |
| Cache Stampede | async_function | `stampede` | `tests/spike/s1/tests/stampede.rs` | `identical_inputs_invoke_model_once`, `different_inputs_not_conflated` |

**Risks & open questions.**
- VERIFIED (plan): vendor CPU latency for Decide is about 167 ms on 48 cores; the chatlog's "under 15 ms" is false for this model. Latency assertions are measured on this machine, never copied.
- UNVERIFIED: ≥99% top-1 parity of the ONNX export (S0-4) decides whether the real-model run is possible; fallback is a Python sidecar.
- LIKELY: synthetic-model results prove plumbing only.
- Open: conservative default thresholds for fail-open consumers until labelled fixtures accrue.

**Chatlog / demo sources.** msg 3/5 (System 1 jobs), msg 38 (S1 for routine verification/security), plan (S1 consumers' failure modes). Corrected: S1 latencies of 15 to 40 ms in the chatlog are not used.

### `polaris-spike-sandbox` — burst tests for sandboxes: concurrent create/exec/destroy, checkout file-event floods during sync

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests/spike/sandbox` | L8 | spike | none | P2 |

**Purpose.** Verifies the execution plane under load: many sandboxes created, used and destroyed at once through `polaris-openshell`, the Reaper leaving nothing behind, and the sync layer staying consistent when `git checkout` rewrites thousands of files (a notify overflow must force a full rescan instead of silently missing changes). It needs the WSL2 gateway, so every real run is BLOCKED-ON-ACCESS until Ubuntu 24.04 and OpenShell are installed; a lighter variant against a fake gateway checks the semaphore and Reaper logic only.

**API sketch.**
```rust
struct SandboxBurst { concurrent: usize, cycles: usize, exec_cmd: Vec<String>, tier: Tier }
async fn run_sandboxes(backend: &OpenShellBackend, reaper: &Reaper, b: &SandboxBurst) -> Result<SandboxReport, SpikeError>;
struct SandboxReport { created: usize, failed: Vec<String>, peak_concurrent: usize, leaked: LeakReport, create_hist: Hist }
async fn checkout_flood(ws: &Workspace, sync: &SyncEngine, files: usize) -> Result<SyncReport, SpikeError>;
struct SyncReport { overflow_seen: bool, rescan_forced: bool, manifest_equal: bool, duration: Duration }
```

**Dependencies.** Internal: `polaris-openshell` (plus `polaris-kernel-reaper`, `polaris-storage-sync`, `polaris-platform` via the TOML). External: `tokio = 1.54.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: sandbox burst driver, leak report, checkout flood driver, manifest equality check.
- [ ] Replace the doc-only lib skeleton with `tests/sandbox_burst.rs` and `tests/checkout_flood.rs`.
- [ ] Fake-gateway variant (in-process): assert the concurrency semaphore and Reaper teardown logic; label it FAKE-GATEWAY, not evidence for OpenShell.
- [ ] Real variant: 20 concurrent create/exec/destroy cycles, 5 cycles each; assert peak concurrency never exceeds the cap and the leak report is empty.
- [ ] Checkout flood: 10k-file repo, `git checkout` between two branches while sync runs; assert the sandbox manifest equals the workspace manifest after the flood, including the overflow-forced rescan path.
- [ ] Quota exhaustion: request sandboxes beyond the cap; assert queueing or a typed error, never a hang.
- [ ] Record create latency histogram and total resource use (WSL2 memory vs the 6 GB cap).
- [ ] BLOCKED-ON-ACCESS (WSL2 Ubuntu 24.04 with `.wslconfig` memory cap, OpenShell v0.1.4 gateway): every real-variant item.

**Tests.**
| Category | Cases |
|---|---|
| spike | burst, checkout flood, quota exhaustion |
| stress | kill the gateway mid-burst; WSL2 memory pressure; disk full on the sandbox volume |
| security | no key or `.env` ever appears in the sandbox filesystem after sync; deny-list holds under the flood |
| fixtures | generated 10k-file repo (`gen_repo`) with hostile names mixed in |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Sandbox Burst | async_function | `run_sandboxes` | `tests/spike/sandbox/tests/sandbox_burst.rs` | `peak_concurrency_within_cap`, `no_leaked_resources`, `failures_are_typed`, `create_latency_recorded` |
| Checkout Flood | async_function | `checkout_flood` | `tests/spike/sandbox/tests/checkout_flood.rs` | `manifest_equal_after_flood`, `overflow_forces_rescan`, `deny_list_survives_flood` |
| Quota Exhaustion | async_function | `exhaust` | `tests/spike/sandbox/tests/quota.rs` | `excess_requests_queue_or_error`, `no_hang_beyond_deadline` |

**Risks & open questions.**
- VERIFIED (plan): WSL2 currently has only `docker-desktop` (no Ubuntu, no `.wslconfig`); nothing here has run.
- UNVERIFIED: notify overflow behaviour on Windows (`ReadDirectoryChangesW` buffer) for a 10k-file checkout; the forced-rescan rule is the plan's design response.
- UNVERIFIED: OpenShell sandbox create latency; no number is claimed. The chatlog's "5 to 10 ms" Firecracker boot is not applicable to the chosen drivers.
- Open: sandbox cap value (derived from the 16 GB RAM and 6 GB WSL cap).

**Chatlog / demo sources.** msg 25 (sandbox tiers, microVM boot claims, corrected), msg 17 (semaphores), plan (checkout file-event floods). Corrected: Firecracker timings dropped.

### `polaris-spike-web` — burst tests for the web/daemon surface: client connection floods and SSE fan-out

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `tests/spike/web` | L8 | spike | none | P9 |

**Purpose.** Checks that `polaris-web` / `polarisd` keep bounded latency and memory when many clients appear suddenly (browser tabs, mobile reconnects) and when the kernel emits token streams to all of them. It also measures the hypotheses from the chatlog (time-to-shell, TTFB) under stated conditions. The package depends on `polaris-ui-web` per workspace.toml (for the server-rendered shell) and will need the web app binary built by `xtask`.

**API sketch.**
```rust
struct ClientFlood { clients: usize, ramp: Duration, behaviour: ClientBehaviour }
enum ClientBehaviour { IdleSse, ReadAll, SlowReader { delay: Duration }, Churn { lifetime: Duration }, WebSocketTerminal }
async fn run_clients(base: &Url, f: &ClientFlood) -> Result<WebReport, SpikeError>;
struct WebReport { connected: usize, refused: usize, event_latency: Hist, rss_per_client: u64, ttfb: Hist, shell_time: Hist }
```
HTTP client choice for the spike (reqwest 0.13.5 is pinned in workspace.toml; `eventsource-stream` 0.2.3 for SSE) must be added through the TOML.

**Dependencies.** Internal: `polaris-ui-web` (plus the `polaris-web` binary and test helpers). External: `tokio = 1.54.0`, `hdrhistogram = 7.6.0`.

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: client flood driver, behaviours, report schema, bounded-memory assertion.
- [ ] Replace the doc-only lib skeleton with `tests/web_flood.rs`.
- [ ] Scenario A: 1k idle SSE clients, ramp 10 s; record RSS per client and connection refusals.
- [ ] Scenario B: 256 clients reading one token stream at 100 events/s; p99 inter-event latency.
- [ ] Scenario C: slow readers and churn; assert slow clients receive a snapshot-or-drop, never unbounded buffers, and the kernel broadcast is unaffected.
- [ ] Scenario D: WebSocket terminal flood with oversize messages; assert the size cap closes the socket.
- [ ] Measure TTFB and time-to-shell for `/` with and without compression, loopback only; report as measured with machine, OS and build profile.
- [ ] BLOCKED-ON-ACCESS (cargo-leptos + wasm target for a real hydrated build; a daemon on a Nebius VM for the remote variant).

**Tests.**
| Category | Cases |
|---|---|
| spike | scenarios A to D |
| scalability | 10 / 100 / 1000 clients: latency and RSS curve |
| security | connection floods do not bypass auth; per-IP/connection caps hold |
| stress | repeated daemon restarts while clients reconnect with `from_seq` |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Client Flood | async_function | `run_clients` | `tests/spike/web/tests/web_flood.rs` | `idle_clients_bounded_memory`, `fan_out_latency_recorded`, `slow_reader_gets_snapshot_not_backlog`, `auth_enforced_under_flood` |
| Terminal Socket Flood | async_function | `terminal_flood` | `tests/spike/web/tests/terminal.rs` | `oversized_message_closes_socket`, `disconnect_kills_pty` |
| Shell Timing | async_function | `measure_shell` | `tests/spike/web/tests/shell.rs` | `ttfb_reported_with_conditions`, `shell_time_reported_with_conditions` |

**Risks & open questions.**
- UNVERIFIED: the chatlog's "TTFB < 2 ms" and "<1.5 ms shell" claims; the spike measures, it does not assert them.
- UNVERIFIED: client-side load generation from the same machine skews results; report that.
- Declared v2: mobile reconnect over 5G/Wi-Fi and WireGuard are not simulated here.
- Open: connection cap policy for the daemon.

**Chatlog / demo sources.** msg 33 (SSR streaming, SSE + WebSocket), msg 35 (gRPC clients), msg 37 (zero-drop session: v2), plan ("UI client floods").

### `xtask` — repository automation in Rust: workspace generation and checks, on-demand lint, fixtures, docs seed, test scaffolding

| Path | Layer | Kind | Parent | Phase |
|---|---|---|---|---|
| `xtask` | L8 | xtask | none | P0 |

**Purpose.** One `cargo xtask <subcommand>` entry point that replaces the interim `@prime/tools/workspace.py` (Python 3.14 stdlib, run with `-I`) and carries the rules that deliberately do not live in `Cargo.toml` or `clippy.toml` (D10). It has no internal crate dependencies: it reads `@prime/tools/workspace.toml` and `cargo metadata`. Until it exists, the Python script is the checker of record; the Rust port must prove parity on the real repository before the Python script is retired. It never writes lint configuration into the repository.

**API sketch (CLI surface).**
```text
cargo xtask gen                    # recreate apps/ crates/ tests/spike/ xtask skeletons + root Cargo.toml from workspace.toml
cargo xtask check-members          # every workspace glob match has a Cargo.toml; no stray files at glob level
cargo xtask check-layers           # from `cargo metadata`: no upward/sideways edges, no child->parent edges
cargo xtask audit-versions         # crates.io sparse index: exists, not yanked, rust_version <= 1.99; git tag -> rev for openshell-sdk
cargo xtask probe                  # dep-probe workspace: cargo generate-lockfile, cargo tree -d, cargo tree -i ring (expected empty)
cargo xtask chatlog-inventory      # regex-extract $directives, /commands, @mentions, polaris-* names, crate names from chatlog.json; each must appear in PLAN.md or its rejected list
cargo xtask lint [--crate NAME] [--deny]     # run the prompt.md lint list on demand (never committed)
cargo xtask fixtures [--regen s1-goldens] [--check]   # provenance check; regenerate goldens (needs uv + Hugging Face access)
cargo xtask seed-docs [--out DIR]  # build the NVIDIA docs seed (llms.txt / .md pages) for polaris-nvidia-docs
cargo xtask scaffold-tests [--descriptors specs/descriptors] [--check]   # JSON descriptor -> test file (only if the external utility is not provided)
cargo xtask test [--profile default|ci|heavy|keyed|wsl|conpty]   # build the app binaries, then run nextest with that profile
cargo xtask check-unsafe           # grep-count own `unsafe`; compare with the declared exception list
cargo xtask check-consistency      # workspace.toml <-> PLAN sections <-> PHASES tasks <-> STATUS rows, one to one
```
```rust
fn main() -> miette::Result<()>;                                 // clap derive (clap 4.6.7), no unwrap/expect
mod workspace { pub fn load(path: &Path) -> Result<WorkspaceModel, XtaskError>; }   // toml 1.1.8 + serde
mod layers    { pub fn check(model: &WorkspaceModel, meta: &CargoMetadata) -> Result<Vec<Violation>, XtaskError>; }
mod descriptor{ pub fn parse(json: &str) -> Result<Descriptor, XtaskError>; pub fn render(d: &Descriptor) -> String; }
pub enum XtaskError { /* thiserror: Io, Toml, Json, Cargo(exit status + captured stderr), Network, Violation */ }
```
`cargo xtask` needs an alias in `.cargo/config.toml` (added when `main` exists).

**`lint` design (D10-compliant).** The list in prompt.md section 1 (the `forbid`/`deny` attribute set and the `disallowed-methods` table) is stored as data under `xtask/lints/` (extracted verbatim, with a test that fails if prompt.md and the data drift) and passed to `cargo clippy` on the command line. `disallowed-methods` is a clippy config key, so xtask writes a temporary `clippy.toml` outside the repository and points clippy at it with `CLIPPY_CONF_DIR` (LIKELY supported; confirm). Output: per-crate counts to `target/xtask/lint-report.json`. Default mode reports and exits 0; `--deny` exits non-zero on any finding. The full list is meant for review discipline ("think about them while writing"), not as a CI gate on day one.

**Dependencies.** Internal: none. External: `clap = 4.6.7` (derive), `serde_json = 1.0.154`, `toml = 1.1.8`. Needed in addition, to be added through the TOML (all already pinned there): `serde = 1.0.229`, `thiserror = 2.0.21`, `miette = 7.6.0`, `jsonschema = 0.59.0` (descriptor schema), `reqwest = 0.13.5` (sparse-index and docs fetch; or shell out to `curl`), `blake3` / `sha`-type hashing for the docs seed manifest (`blake3 = 1.8.7`), `regex = 1.13.1` (inventory).

**TODO.**
- [ ] Write JSON descriptors (prompt.md §6.3) for: workspace model loader, layer checker, descriptor parser/renderer, lint driver, version auditor, inventory extractor.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target (keep `src/lib.rs` for testable logic); add the `cargo xtask` alias in `.cargo/config.toml`.
- [ ] Port `check-members`, `check-layers`, `gen` from `workspace.py`; run both implementations on the repo and diff outputs byte for byte (parity test) before retiring Python.
- [ ] Port `audit-versions` (sparse index `index.crates.io`: exists, not yanked, `rust_version <= 1.99`; resolve git tag to rev for `openshell-sdk`); network-dependent, so tests use recorded index lines and one live run recorded as evidence.
- [ ] Implement `probe`: drive `@prime/tools/dep-probe/` (`cargo generate-lockfile`, `cargo tree -d`, `cargo tree -i ring`); fail if `ring` appears unexplained.
- [ ] Port `chatlog-inventory` and add the traceability-matrix coverage check (100% of items adopted/corrected/rejected/deferred).
- [ ] Implement `scaffold-tests`: validate descriptors against the prompt.md section 6.3 schema (pure JSON: the prompt's `//` comments are not valid JSON), render `tests/<logic_name_snake_case>.rs` into the owning crate's `tests/` (or `tests/tests/` for cross-crate), `#[tokio::test]` for async types else `#[test]`; generated tests return `Err` until implemented and are `#[ignore]`d (no `todo!()`/panic, prompt.md section 6.1); `--check` fails if a descriptor has no test file or a test name is missing.
- [ ] Implement `lint` (above) with the drift test against prompt.md.
- [ ] Implement `fixtures`: provenance check (every directory has `PROVENANCE.md`), "no hostile data committed" grep gate, optional regeneration of Python `gliner2` goldens (BLOCKED-ON-ACCESS: Hugging Face + `uv`).
- [ ] Implement `seed-docs`: fetch NVIDIA `llms.txt` and referenced `.md` pages (keyless; `llms.txt` existence VERIFIED for docs.nvidia.com, CUDA, OpenShell, NemoClaw), store with blake3 manifest, fetch politely (rate limit, identify the client), no scraping of non-doc pages.
- [ ] Implement `test`: build `polaris`, `polarisd`, `polaris-worker` first, then `cargo nextest run --profile <p>` (nextest not installed yet: P0 tool install).
- [ ] Implement `check-unsafe` (count against the declared exception list: named-pipe security descriptor, memory locking, fuzz-macro expansion, ConPTY only if S0-10 fails; plus whatever the allocator decision adds) and `check-consistency`.
- [ ] BLOCKED-ON-ACCESS (network for `audit-versions` / `seed-docs`, Hugging Face for goldens): live runs; keyless offline paths are tested with recorded data.

**Tests.**
| Category | Cases |
|---|---|
| unit | layer rule table (facade to child, sibling in same parent, child to parent, upward edge); semver/`rust_version` comparison; descriptor to test-name snake-casing; sparse-index line parser; lint-data drift vs prompt.md |
| proptest | descriptor round trip (parse, render, parse) preserves test names; layer checker never panics on random graphs |
| insta snapshot | rendered scaffold test files for sample descriptors; `gen` output for a 3-crate sample model |
| fuzz | WSL2 nightly: descriptor JSON parser, workspace.toml loader, sparse-index parser |
| smoke | each subcommand `--help`; `check-members` and `check-layers` on this repository; `gen` followed by `check-members` is idempotent (second `gen` leaves a clean git diff) |
| contract | parity: Rust xtask output equals `workspace.py` output on the repository for `gen`, `check-members`, `check-layers` |
| security | `scaffold-tests` refuses output paths outside the workspace; `seed-docs` writes only under `--out`; `lint` never writes into the repo |
| fixtures | `fixtures --check` passes on committed data and fails on a planted missing `PROVENANCE.md` |

**Test-first descriptors.**
| logic_name | type_of | name | file_path | test names |
|---|---|---|---|---|
| Layer Rule Checker | function | `check` | `xtask/src/layers.rs` | `facade_to_child_allowed`, `sibling_same_parent_allowed`, `child_to_parent_rejected`, `upward_edge_rejected`, `sideways_adapter_edge_rejected` |
| Descriptor Scaffolder | function | `render` | `xtask/src/descriptor.rs` | `async_function_uses_tokio_test`, `file_named_after_logic_snake_case`, `generated_tests_are_ignored_and_return_err`, `invalid_json_with_comments_rejected`, `path_outside_workspace_rejected` |
| Lint Driver | function | `run_lint` | `xtask/src/lint.rs` | `lint_data_matches_prompt_md`, `clippy_conf_dir_is_outside_repo`, `report_lists_per_crate_counts`, `deny_mode_exits_nonzero_on_finding` |
| Version Auditor | async_function | `audit_versions` | `xtask/src/versions.rs` | `yanked_version_fails`, `rust_version_above_1_99_fails`, `git_tag_resolved_to_rev`, `offline_mode_uses_recorded_index` |

**Risks & open questions.**
- VERIFIED (prompt.md, read this session): the lint list contains `forbid(clippy::pedantic)` and `forbid(clippy::nursery)` plus many mutually hard-to-satisfy lints; `forbid` cannot be overridden by macro-expanded `allow`. Derive macros (thiserror, serde, clap, tokio) and generated code (tonic/prost, Slint) may produce findings that cannot be fixed. LIKELY: the report will be noisy; treat it as a review aid, exclude generated crates/modules explicitly in xtask data, and say so.
- VERIFIED (prompt.md): the same document says the list should not be put in `Cargo.toml`, and D10 extends that to `lib.rs` and `clippy.toml`; the on-demand xtask is the compliant way.
- UNVERIFIED: `CLIPPY_CONF_DIR` behaviour and whether clippy accepts `-F` flags for each listed lint on rustc 1.99.0; test on the first run.
- UNVERIFIED: prompt.md says the descriptor utility is "provided separately"; if it arrives before P1, `scaffold-tests` is deleted or becomes a wrapper, and that is declared.
- LIKELY: `audit-versions` and `seed-docs` need the network and may hit rate limits; recorded fixtures keep CI keyless.
- Open: should `check-consistency` read PHASES_STATUS.md evidence cells and fail on PASS without a command/raw-output block (supports the evidence-discipline rule)?
- Open: tools not installed (cargo-deny/vet/audit, cargo-leptos, nextest, fuzz, mutants, uv, protoc, clang): `cargo xtask doctor` could list them; propose as a small addition.

**Chatlog / demo sources.** Plan "Execution steps" (workspace.py subcommands, dep-probe, mechanical consistency pass), prompt.md sections 1, 6.3 and 7 (lint set, test-first descriptors, Miri/Kani/fuzz), workspace.toml (`xtask` responsibilities), chatlog msg 26 (spdlog/regex requests: inventory items), msg 38 and 40 (user additions that the inventory must cover). Demo: `scripts/build_static.py` and `scripts/test_*.py` show the demo's own verification scripting; no code reused.

---

## 14. Chatlog traceability matrix (semantic)

Every requirement or idea from `@prime/chatlog.json` (42 messages) with its disposition.
**A** = adopted · **C** = adopted with correction (see §4) · **R** = rejected (reason) · **D** = deferred to v2.
Owner = crate (prefix `polaris-` omitted). Phase per PHASES.md.

### 14.1 User messages (requirements — none dropped)

| Msg | User requirement | Disp. | Owner(s) | Phase |
|---|---|---|---|---|
| 0 | Build an own coding agent (Claude Code / Antigravity / Codex class) for the Nebius×NVIDIA hackathon, Coding & Agentic track | A | whole workspace | P0–P10 |
| 0 | OpenShell + NemoClaw as security guard and runtime | C (roles clarified C7) | openshell-{client,policy,nemoclaw} | P2 |
| 0 | Built-in System One model (typesafe.ai concept) | A | system-one-* | P4 |
| 0 | `$` session settings; bare `$name` = on; list arguments | A (D11) | kernel-directives | P1 |
| 0 | `$websearch(on\|off)`, `$subagents(on\|off)`, `$subagent-models(model(effort-effort), …)`, `$links(…)`, `skills(…)`→`$skills`, `mcp(…)`→`$mcp`, `rag(…)`→`$rag` | A | kernel-directives, search, runtime-frames, models, ext-*, storage-vector | P1–P7 |
| 0 | `/model`, `/usage`, `/stats`, `/quit(exit)`, `/plan`, `/goal`, "…" | A | kernel, ui, telemetry, runtime | P1–P3 |
| 0 | Stack: GUI Slint, Web Tauri, CLI ratatui, Server axum, S1 `fastino/GLiNER2.5-Decide` | C (Tauri dropped, D6) | ui-*, apps, system-one | P1–P9 |
| 0 | Platforms Windows, macOS, Linux (Windows first) | A | platform-*, sdk PlatformOps | P0–P10, v2 |
| 0 | Initial crate list (desktop, web, cli, kernel, sdk, platform, model-register→models, openshell, runtime, nebius-cloud, ui + cli/gui/web; later audio/stt) | A (rename: `polaris-model-register` → `polaris-models`) | workspace.toml | P0 |
| 0 | `polaris-audio`, `polaris-stt` later | D | v2 | v2 |
| 2 | S1 for source relevance, file analysis, websearch/research help, sub-agent effort routing, "and much more" | A | system-one-router, search-sift | P4–P5 |
| 4 | Verification by S1 | A | system-one-verifier, kernel-verifier | P3–P4 |
| 4 | Parsing module + API for all NVIDIA documentation; make the agent an NVIDIA expert | A | nvidia-docs | P5 |
| 6 | `$nvidia(cuda, hopper, openshell)`, `$nvidia-docs(sync)`, `/nv-profile`, `/nv-verify` | A | nvidia-docs, system-one-verifier | P5 |
| 6 | `$nvidia(auto)` — S1 decides what is needed | A | nvidia-docs (auto_profile) | P5 |
| 8 | "Don't forget Nebius — what can we do for Nebius?" | A | nebius-cloud-* | P3, P6 |
| 10 | 7,000+ ready environments (SWE-bench Verified, SWE-rebench) without local downloads, combined with S1 determiner | A (VERIFIED F6) | nebius-cloud-sandboxes, system-one-router | P3, P6 |
| 10 | Large list of security check templates + S1 | A | security-rules, system-one-verifier | P4 |
| 10 | System 2: batching, caching, DB (lancedb / duckdb / redb / lmdb / serverless cockroach) | C (redb+SQLite+LanceDB; DuckDB/LMDB/Cockroach rejected for v1, ADR-007) | storage-*, token-factory | P1–P5 |
| 10 | Agents must not endlessly overwrite one file: responsibility zones, file queue, S1 verifier comparing changes and adjacency | A | runtime-leases, system-one-verifier (arbiter) | P2–P4 |
| 12 | Event-driven architecture with Biscuit tokens for change requests; checker = math module + S1 | A (C17) | kernel-bus, security-capability, kernel-verifier | P1–P3 |
| 14 | blake3 + xxhash for fast comparison; hash table | A | storage-cas, storage-cache | P1 |
| 16 | Async everywhere (search, API, docs) + semaphores | A | runtime-dag pools, search-sift | P3–P7 |
| 18 | Rate-limit module; comprehensive global DBs (session paths, caches, history, sessions) | A (C21) | ratelimit, storage-registry, storage-eventlog | P1–P3 |
| 20 | `/research` with Karpathy's algorithm, robust for very long work | C (C33) | runtime-research | P7 |
| 20 | A couple of OCR engines; ready-made parsing libraries for `@docx` and other documents | A | ingest (ocrs + oar-ocr; docx-rs, calamine, pdf-extract, pdfium-render) | P5 |
| 20 | PII protection so critical data never reaches the model | A | security-pii | P4 |
| 20 | Reliability/fault tolerance; hybrid microservices + event-driven but without many processes; restart modules on failure while the system keeps working | C (D3: in-process supervision + per-mode worker processes) | kernel-supervisor, apps/polaris-worker | P1, P4 |
| 22 | Never lose parallelism | A | runtime-dag, kernel-bus | P1–P7 |
| 22 | High-quality context compression that forgets nothing | A (C28, C29) | runtime-context, storage-cas | P3–P5 |
| 24 | Embed Firecracker microVMs for testing and screenshots; interactive browser | C (D2: OpenShell vm driver) | openshell-client (E2), browser, apps/polaris-guest | P7 |
| 26 | Something like spdlog for Rust | A (ADR-010) | log | P1 |
| 26 | Rust regex | A | security-automata (regex-automata/regex-syntax), tools-builtin | P1–P2 |
| 26 | Agent tooling: math functions, interpolation, calculator, others | A (C14) | tools-math, tools-builtin | P1–P3 |
| 26 | Protobuf | A | proto | P1 |
| 26 | iceoryx2 | C (later optimization, S0-8) | rpc/worker IPC | P7+ |
| 26 | RAM encryption and wiping | A (C18) | security-secrets | P3 |
| 26 | Deallocation of processes, agents, memory, etc. | A | kernel-reaper, platform-process, models (eviction) | P1–P4 |
| 28 | Agents are contexts; one model; same for S1; S1 always checks agent states | A | runtime-frames, system-one-watchdog | P3–P4 |
| 30 | Take a couple of specific Anthropic "mods" (not the mod system) | A (C32: Blast Radius, Proof-of-Done, Reflect/Rule Distiller, Token Weather) | runtime-guards, runtime-context, telemetry, ui | P2–P7 |
| 32 | Web version SSR with async page loading | A (D6) | apps/polaris-web, ui-web | P9 |
| 32 | `regex-automata = "0.4.18"`, `regex-syntax = "0.8.11"` | A (C19 features corrected) | security-automata | P2 |
| 32 | More super-small models like Cactus Needle 3 | A (gated S0-6) | system-one-nano | P8 |
| 32 | Semantic search | A | storage-vector, search, system-one-nano | P5 |
| 32 | Hacker model/agent: breaks code, finds architectural inconsistencies and bottlenecks | A | hacker | P7 |
| 34 | gRPC | A | proto, rpc | P1, P9 |
| 34 | Hacker must be a built-in tiny checker, not Nemotron, to save tokens | A | hacker (zero-token; S2 only with PoC) | P7 |
| 36 | v1 = first version; later self-hosting and coding from the phone | D | v2 roadmap §16 | v2 |
| 38 | Token economy: Nemotron only for deep intelligence; routine work local (S1 + Needle + DFA) | A | system-one-*, security-automata, ratelimit | P3–P8 |
| 38 | Functional operators | A | kernel-operators | P3 |
| 38 | Unused models are turned off to free memory | A | models (lifecycle), apps/polaris-worker | P4 |
| 40 | Embed Nix | A (C16) | devenv-nix | P8 |
| 40 | Correct, fast Docker | A | devenv-docker | P8 |
| 40 | Fast testing via Dagger + built-in BuildKit CI/CD ("builtinkitcicd") | A | devenv-cicd | P8 |

### 14.2 Model-message ideas (design content)

| Msg | Idea / component | Disp. | Owner | Phase |
|---|---|---|---|---|
| 1 | Dual-engine cognition S1/S2; escalate on low confidence or complexity | A (thresholds calibrated, not 0.70 fixed) | system-one-router | P4 |
| 1 | S1 tasks: intent routing, zero-token tool selection (top-k), injection gatekeeper | A | system-one-router, system-one-verifier | P4 |
| 1 | S2 streaming client with live reasoning/diffs | C (C1, C2) | token-factory | P3 |
| 1 | Credential custody; egress proxy; keys only at authorized endpoints | C (provider profiles, F11) | openshell-policy | P2 |
| 1 | Sandbox per session; CoW workspace; sub-agents inherit limits | C (sync + per-frame quotas) | storage-sync, openshell-policy, runtime-frames | P2–P3 |
| 1 | Execution loop: triage → plan → sandbox exec → verify (max 3 retries) → audit/telemetry | A | runtime, kernel-operators, telemetry | P3 |
| 1 | `/diff`, `/rollback` commands; directives vs commands distinction | A | vcs, kernel-directives | P1–P2 |
| 1 | Multi-platform table (Landlock/Job Objects/ConPTY/FS watchers/DirectML-CUDA-CoreML) | C (OpenShell enforces isolation; platform crates) | platform-*, system-one-engine | P2, P4 |
| 1 | Workspace layout v1 (17 members) | A (expanded to 93) | workspace.toml | P0 |
| 3 | S1 as pervasive cognitive co-processor: file triage, RAG filter, web research distiller, effort router (1–5), speculative diff/safety | A | system-one-router, search-sift, system-one-verifier | P4–P5 |
| 3 | Complexity-to-model mapping via `$subagent-models` | A | models | P3 |
| 3 | S1 indexes MCP methods and whitelists per step | A | ext-mcp, system-one-router | P7 |
| 5 | Two-tier verifier: pre-flight (diff sanity, sandbox, secrets, CUDA safety) / post-flight (semantic assertion, diagnostics sifter, regression invariant) | A | system-one-verifier | P4 |
| 5 | `polaris-nvidia-docs` with scraper (cuda, tensorrt, openshell, nemotron, hardware), storage, parser, indexer, advisor | C (llms.txt/.md/MCP ingestion, F20) | nvidia-docs | P5 |
| 5 | Offline embedded seed of NVIDIA APIs | C (local artifact, not in git/binary; ADR-014) | nvidia-docs, xtask | P5 |
| 7 | Auto-detection pipeline (workspace probe, hardware resolver via NVML, task intent scorer); invariants; doc slices | A | nvidia-docs | P5 |
| 7 | `$nvidia` syntax matrix (auto, manual, security focus, off) | A | kernel-directives, nvidia-docs | P5 |
| 9 | Contree sandboxes (branching, 7,000+ envs, VM isolation, MCP server) | A (F6/F7) | nebius-cloud-sandboxes | P3, P6 |
| 9 | Speculative decoding on Token Factory | C (C15 cascade drafting) | models | P6 |
| 9 | FinOps budgeting | A (`$budget`) | telemetry | P6 |
| 9 | Data Lab flywheel (trajectory export, LoRA fine-tune) | A (API UNVERIFIED) | nebius-cloud-data-lab | P6 |
| 9 | Cloud bursting on AI Cloud (H100), Slurm, MK8s | C (remote OpenShell gateway E4; Slurm/MK8s deferred) | nebius-cloud-ai-cloud | P6 / v2 |
| 9 | `$nebius-sandbox`, `$nebius-branching`, `$nebius-datalab`, `$nebius-speculative`, `/nebius-status`, `/contree-branches`, `/datalab-sync` | A | §9 catalog | P3–P6 |
| 11 | S1 ConTree environment resolver (manifest hash, SWE-bench match) | A | nebius-cloud-sandboxes | P6 |
| 11 | S1 security sieve (Aho-Corasick/AST + GLiNER arbiter); CWE-tagged knowledge base | A | security-rules, system-one-verifier | P4 |
| 11 | Request batcher/coalescer; prompt prefix cache | A (caching UNVERIFIED, S0-5) | token-factory, runtime-context | P3 |
| 11 | AST-scope leases, FIFO file queue, invalidation notices, S1 diff merge arbiter, cycle breaker (>3 edits) | A | runtime-leases, system-one-verifier | P2–P4 |
| 13 | Commands/events contracts; CQRS; reactive UI bus | A | sdk, kernel-bus, ui-model | P1 |
| 13 | Biscuit authority, attenuation (resource, churn, TTL, no-unsafe) | A | security-capability | P3 |
| 13 | Hybrid mutation verifier (Datalog, churn/Δcyclomatic/tree-edit, S1 congruence) | A | kernel-verifier, code-intel | P3 |
| 15 | Two-tier hashing (xxh3 dirty-check, BLAKE3 CAS), AST subtree fingerprints, BLAKE3 diff fact in Biscuit, OCC on base hash, incremental sandbox sync | A | storage-cas, code-intel, security-capability, runtime-leases, storage-sync | P1–P3 |
| 17 | Concurrency controller with named semaphores; speculative race with S1 validation and abort; `$concurrency`, `/concurrency-stats` | A | runtime-dag, search-sift | P5–P7 |
| 19 | `polaris-storage` (registry SQLite FTS5, events redb, CAS, LanceDB L3 cache); `polaris-ratelimit` (dual RPM/TPM, priorities, header parsing) | A (C21) | storage-*, ratelimit | P1–P3 |
| 19 | `/sessions`, `/session resume`, `/session replay`, `/cache stats\|purge`, `/quota`, `$ratelimit`, `$storage-level` | A | §9 catalog | P1–P5 |
| 19 | Trajectory warehouse (ShareGPT/JSONL) | A | nebius-cloud-data-lab | P6 |
| 21 | `/research` status/report/resume, evidence ledger, adversarial critique, checkpoints | A (C33) | runtime-research | P7 |
| 21 | Ingest pipeline (native parse → OCR → S1 sieve); `@docx(path)` `@pdf(path)` `@xlsx` `@img` `@scans` | A | ingest, kernel-directives | P5 |
| 21 | Reversible PII vault (regex + entropy + S1), unmask at sandbox boundary | A | security-pii | P4 |
| 21 | Supervision trees, one-for-one, circuit breaker (N failures / window), DLQ | A (C9) | kernel-supervisor | P1 |
| 23 | DAG work-stealing scheduler; lock-free MVCC snapshots (arc-swap); speculative branch-and-prune | A (C22) | runtime-dag, runtime-leases | P7 |
| 23 | Lossless context virtualization: deterministic folding §ref + `polaris_expand`, skeleton + PageRank repo map, epistemic scratchpad, prefix-cache preservation, `/context-map`, `/context-expand`, `/parallel-dag` | A (C28, C29) | runtime-context, code-intel | P3–P7 |
| 25 | Three-tier execution matrix (OpenShell / microVM / Contree) and task router | C (E0–E4, D2) | openshell-*, system-one-router | P2–P7 |
| 25 | Headless browser (CDP, a11y tree token squeezer, network interception), visual verification loop, `/screenshot`, `/browser goto/tree`, `/ui-diff`, `$browser`, `$sandbox(firecracker\|auto)` | C (C10; microvm instead of firecracker) | browser, apps/polaris-guest | P7 |
| 27 | spdlog-style async logging bridged from tracing | A | log | P1 |
| 27 | iceoryx2 + prost IPC for large payloads | C (S0-8; gRPC first) | rpc, proto | P7+ |
| 27 | Secure RAM vault (lock, ChaCha20-Poly1305, zeroize) | C (C18) | security-secrets | P3 |
| 27 | Reaper deallocation matrix (cancellation, cgroups/Job Objects, microVM teardown, shm, temp, secrets) | A | kernel-reaper, platform-process | P1–P2 |
| 27 | Math engine (evaluator, interpolation, NVIDIA hardware calculus) | C (C14) | tools-math | P3 |
| 29 | Single-substrate swarm (frames), meta-cognitive watchdog (drift, loops, role boundary, fatigue) | A | runtime-frames, system-one-watchdog | P3–P4 |
| 31 | Blast radius (shell + AST), Proof-of-Done barrier, Auto-Reflect/Rule Distiller (`.polaris/RULES.md`, invariants), Token Weather & KV-cache HUD, keep-warm ping | A (keep-warm gated on S0-5) | runtime-guards, kernel-operators (ReflectOp), runtime-context, telemetry, ui-cli | P2–P7 |
| 33 | Leptos SSR streaming, hydration, chunked routes, SSE + WebSocket | A (D6) | apps/polaris-web, ui-web | P9 |
| 33 | AOT DFA security scanner, HIR ReDoS guard | A (C19) | security-automata | P2 |
| 33 | Tier 0 Needle 3 (function calling, embeddings), `$needle`, `/search-local` | A (S0-6) | system-one-nano, search | P8 |
| 33 | Hacker agent attack vectors (chaos, boundary probing, bottleneck hunter), `/hack`, `/redteam` | A (C23) | hacker | P7 |
| 35 | gRPC daemon (UDS/localhost/mTLS, vsock to guest), proto `PolarisKernelService` | C (C24, C25) | proto, rpc, apps/polaris-daemon | P9 |
| 35 | Zero-token hacker engine (static DFA + Needle fuzzer + sandbox stress); escalate only on confirmed crash | A | hacker | P7 |
| 37 | Self-hosting (static binary, NixOS flake, Docker Compose), WireGuard/Tailscale mesh, mobile client, voice loop, Telegram bot, touch diff review, cellular-resilient gRPC | D | v2 roadmap | v2 |
| 39 | Functional operator pipeline (pipe, fork_branch, fold, assert_guard), operator families | A | kernel-operators | P3 |
| 39 | Model lifecycle (cold/lease/idle eviction, force-evict before compile) | A (C26) | models, apps/polaris-worker | P4 |
| 41 | Nix dev shells + flake synthesis, bollard Docker with warm pool and rootless Podman, Dagger + BuildKit cache mounts, `$env(nix\|docker)`, `$ci(dagger)`, `/ci`, `/ci-run`, `/nix-flake` | C (C16) | devenv-* | P8 |

---

## 15. Mechanical inventory dispositions

Produced by `python -I @prime/tools/workspace.py chatlog-inventory` (396 items; regex extraction over all
42 messages). `--check @prime/PLAN.md` verifies every item appears in this document.

### 15.1 Directives (25)

Adopted (see §9.1): `$websearch` `$subagents` `$subagent-models` `$links` `$skills` `$mcp` `$rag`
`$nvidia` `$nvidia-docs` `$sandbox` `$nebius-sandbox` `$nebius-branching` `$nebius-datalab`
`$nebius-speculative` `$concurrency` `$ratelimit` `$storage-level` `$browser` `$needle` `$env` `$ci`.
Not directives: `$directive` (generic placeholder word in prose), `$hash`, `$lines`, `$time` (Datalog
variables inside Biscuit `check if …` rules — owned by `polaris-security-capability`).

### 15.2 Slash commands (35)

Adopted (see §9.2): `/browser` `/cache` `/ci` `/ci-run` `/concurrency-stats` `/context-expand`
`/context-map` `/contree-branches` `/datalab-sync` `/diff` `/exit` `/goal` `/hack` `/model`
`/nebius-status` `/nix-flake` `/nv-profile` `/nv-verify` `/parallel-dag` `/plan` `/proof-of-done` `/quit`
`/quota` `/redteam` `/research` `/rollback` `/screenshot` `/search-local` `/session` `/sessions` `/stats`
`/ui-diff` `/usage`. Not commands: `/command` (generic placeholder), `/proc` (Linux path referenced by
the safety gate's sandbox-escape patterns — owned by `polaris-security-rules`).

### 15.3 Mentions (6)

`@docx` `@file` `@img` `@pdf` `@scans` `@xlsx` — adopted (§9.3; `polaris-ingest`, `polaris-kernel-directives`).

### 15.4 Polaris crate names in the chatlog (45) → workspace mapping

| Chatlog name | Mapped to |
|---|---|
| polaris-cli, polaris-desktop, polaris-web, polaris-daemon, polaris-guest | apps/ (same names) |
| polaris-guest-daemon | apps/polaris-guest |
| polaris-kernel, polaris-sdk, polaris-platform, polaris-models, polaris-openshell, polaris-runtime, polaris-nebius-cloud, polaris-ui, polaris-ui-cli, polaris-ui-gui, polaris-ui-web, polaris-system-one, polaris-nvidia-docs, polaris-storage, polaris-ratelimit, polaris-security, polaris-tools, polaris-hacker, polaris-rpc, polaris-ingest, polaris-browser | same names (crates/) |
| polaris-model-register | renamed **polaris-models** (key matched the directory) |
| polaris-nemoclaw | polaris-openshell-nemoclaw |
| polaris-mcp | polaris-ext-mcp |
| polaris-supervisor | polaris-kernel-supervisor |
| polaris-nix, polaris-docker, polaris-cicd | polaris-devenv-nix, polaris-devenv-docker, polaris-devenv-cicd |
| polaris-firecracker | replaced by OpenShell `vm` driver (D2) in polaris-openshell-client |
| polaris-expand | model tool `polaris_expand` (polaris-runtime-context) |
| polaris-kernel-service-server | tonic-generated `polaris_kernel_service_server` module (polaris-proto / polaris-rpc) |
| polaris-state | `polaris_state.redb` file name in chatlog → `events.redb` (polaris-storage-eventlog) |
| polaris-server | v2 self-hosting packaging of polaris-daemon |
| polaris-net | example crate name in a voice-intent example (msg 37); not a Polaris crate |
| polaris-audio, polaris-stt, polaris-telegram, polaris-wireguard, polaris-mobile | v2 (§16) |

### 15.5 Cargo dependencies named in the chatlog (51)

Kept and re-pinned (§10): axum, biscuit-auth, blake3, bollard, chacha20poly1305, chromiumoxide, crossterm,
dashmap, evalexpr (→ replaced by exmex: evalexpr ≥ 12 is AGPL-3.0-only), futures, iceoryx2, image, lancedb (opt-in, S0-9), ort, prost, prost-types, pulldown-cmark, rand,
ratatui, redb, regex, regex-automata, regex-syntax, reqwest, rusqlite, scraper, secrecy, serde,
serde_json, slint, spdlog-rs, tokenizers, tokio, tokio-stream, tonic, tower, tracing, tracing-subscriber,
tree-sitter, uuid, xxhash-rust, zeroize. Deferred to v2: boringtun, cpal, whisper-rs. Dropped: sled (C12),
ed25519-dalek (included by biscuit-auth), nix (POSIX-only; process-wrap/rustix cover it). Not dependencies:
`edition`, `resolver`, `version` (Cargo manifest keys matched by the extractor).

### 15.6 Rust types from chatlog code (106) → owner

| Owner crate | Chatlog types |
|---|---|
| polaris-sdk (+ system-one-schemas) | DecisionSchema, QuestionType, DecisionResult, DecisionValue, PolarisCommand, PolarisEvent |
| polaris-system-one-engine / -client | SystemOneEngine |
| polaris-system-one-router | SubagentRoutingDecision, EffortLevel, FileRelevanceScore, FileTriageEngine |
| polaris-system-one-verifier | VerificationReport, SafetyViolation, SystemOneVerifier, MergeOutcome, SystemOneDiffArbiter, SystemOneSecuritySieve |
| polaris-system-one-watchdog | AgentHealthVerdict, MetaCognitiveWatchdog |
| polaris-search-sift | RawChunk, FilteredContext, ContextDistiller, SearchHit, AsyncSearchPipeline |
| polaris-nvidia-docs | NvidiaDocEntry, NvidiaTopic, NvidiaKnowledgeEngine, NvidiaAutoProfile, NvidiaStack, NvidiaAutoDetector |
| polaris-nebius-cloud-sandboxes | ContreeCheckpoint, ContreeSessionManager, ExecutionResult, ContreeEnvironmentResolver |
| polaris-nebius-cloud-token-factory | NebiusClient, NemotronChatRequest, ChatMessage |
| polaris-security-rules | SecurityRule |
| polaris-security-automata | SecurityDfaScanner |
| polaris-security-capability | BiscuitAuthority |
| polaris-security-pii | PiiVault |
| polaris-security-secrets | EphemeralKey, EncryptedRamSecret |
| polaris-runtime-leases | LeaseScope, FileLeaseManager, FileLeaseGuard |
| polaris-runtime-frames | AgentRole, AgentContextFrame |
| polaris-runtime-context | ContextVirtualizer, EpistemicScratchpad |
| polaris-kernel-operators | RuleDistiller (as `ReflectOp`; invariants stored via eventlog, mounted into the scratchpad by runtime-context) |
| polaris-runtime-dag | ConcurrencyController, TaskNode, DagEngine |
| polaris-runtime-guards | BlastRadiusReport, ProofOfDoneResult, NativeAugmentationGuard |
| polaris-kernel-bus | PolarisEventBus |
| polaris-kernel-verifier | HybridMutationVerifier, VerificationDecision |
| polaris-kernel-supervisor | SupervisorStrategy, TaskSupervisor |
| polaris-kernel-reaper | ResourceReaper |
| polaris-kernel-operators (+ sdk trait) | FunctionalOperator, Pipeline, BoxFuture |
| polaris-kernel-directives | DirectiveParser, SessionDirective, SubagentModelEffort, SlashCommand |
| polaris-storage-cas / -cache | FastFingerprint, CryptoContentId, FingerprintEngine, CachedAnalysis, ContentAddressableStore |
| polaris-storage-registry | SessionMetadata, GlobalSessionRegistry |
| polaris-storage (facade) / -eventlog | EmbeddedStorage |
| polaris-ratelimit | Priority, DualRateLimiter |
| polaris-ingest | DocumentContent, DocumentIngestEngine, IngestError |
| polaris-tools-math | MathEngine, InterpolationPoint |
| polaris-models | ModelLease, EvictableModelSlot |
| polaris-openshell-client / -policy | OpenShellSandbox, SandboxPolicy, CommandOutput, MicroVmConfig, FirecrackerHandle (→ vm-driver sandbox handle, D2) |
| polaris-browser / apps/polaris-guest | HeadlessBrowserEngine |
| polaris-hacker | HackerAgent, VulnerabilityExploitReport, ZeroTokenAuditReport, ZeroTokenHackerEngine |
| polaris-rpc | PolarisGrpcImpl, AttachSessionStream |
| polaris-devenv-docker | FastDockerEngine |
| polaris-devenv-cicd | PipelineStep, DaggerCiEngine |
| polaris-devenv-nix | NixEnvironment |
| Not domain types | Error, Output, Target (associated-type names in trait snippets) |

### 15.7 Rust functions from chatlog code (102) → owner

| Owner crate | Chatlog functions |
|---|---|
| polaris-ratelimit | acquire_budget, reconcile_actual_usage |
| polaris-runtime-dag | acquire_disk_permit, acquire_docs_permit, acquire_nebius_permit, acquire_web_permit, execute_parallel |
| polaris-search-sift | race_knowledge_search, fetch_codebase_rag, fetch_nvidia_docs, fetch_websearch_snippets, distill_context |
| polaris-security-capability | append_blake3_diff_fact, attenuate_for_subagent, issue_agent_token |
| polaris-kernel-verifier | verify_mutation_request |
| polaris-system-one-verifier | verify_diff, arbitrate_concurrent_diffs |
| polaris-system-one-router | fast_route, evaluate_task, rank_files_for_query |
| polaris-system-one-watchdog | inspect_agent_state, is_looping |
| polaris-hacker | attack_patch, audit_patch, execute_untrusted (via SandboxBackend) |
| polaris-devenv-cicd | build_rust_test_pipeline |
| polaris-devenv-docker | connect_local, exec_in_container |
| polaris-devenv-nix | evaluate_dev_shell |
| polaris-browser / guest | capture_screenshot, click_element, inspect_url |
| polaris-storage-cas / -cache | check_or_update, put_blob, quick_hash, quick_hash_128, cryptographic_cid, hash_file_mmap, to_hex |
| polaris-storage-registry | search_history, upsert_session |
| polaris-storage-eventlog | get_cached_completion, open |
| polaris-kernel-reaper | child_token, shutdown_all |
| polaris-kernel-supervisor | spawn_resilient |
| polaris-kernel-bus | publish_event, subscribe |
| polaris-kernel-directives | parse_directive, parse_model_efforts |
| polaris-kernel-operators | pipe, fork_branch, execute |
| polaris-tools-math | evaluate_expression, lerp, cubic_spline, calculate_h100_bandwidth |
| polaris-tools-rtk / runtime-context | compress_compiler_diagnostics, fold_tool_output, fold_lines_head_tail, skeletonize_file_content |
| polaris-runtime-guards | estimate_blast_radius, verify_proof_of_done |
| polaris-kernel-operators | distill_from_user_prompt (Rule Distiller, `ReflectOp`) |
| polaris-runtime-leases | acquire_lease |
| polaris-models | evict_if_idle, force_evict, deref (ModelLease), new |
| polaris-nvidia-docs | detect_profile, probe_workspace, resolve_hardware_target, extract_relevant_guidelines, compile_verification_invariants, query_guidelines, sync_upstream_docs |
| polaris-nebius-cloud-sandboxes | spawn_base_environment, execute_in_branch, rollback_branch, resolve_exact_image |
| polaris-nebius-cloud-token-factory | stream_chat |
| polaris-openshell-client | create, execute_tool, spawn |
| polaris-ingest | ingest_file, parse_docx, parse_pdf_with_ocr_fallback, parse_spreadsheet, run_local_ocr |
| polaris-security-pii | mask_prompt, mask_pattern, unmask_for_execution |
| polaris-security-secrets | with_decrypted |
| polaris-security-automata | compile_rules, scan_bytes |
| polaris-rpc | start_grpc_server |
| polaris-ui-cli / apps | init |
| Not requirements | unwrap (the forbidden call itself — banned by prompt.md), signatures (prose word), compute / test_compute (example lease scopes `fn compute()` / `fn test_compute()` in msg 11) |

### 15.8 Proto items (7)

`PolarisKernelService`, `AttachSession`, `ExecuteZeroTokenAudit`, `AuditRequest`, `AuditResponse`,
`SessionCommand`, `SessionEvent` — adopted and corrected in `polaris-proto` / `polaris-rpc` (C24).

### 15.9 Model names (19)

| Chatlog model name | Disposition |
|---|---|
| GLiNER, GLiNER2.5, GLiNER2.5-Decide | S1 (F16) |
| Needle 3 | Tier 0 (F18, S0-6) |
| Nemotron, Nemotron-70B, Nemotron-70B-Instruct, Nemotron-340B, Nemotron-4-340B, Nemotron-Mini, Nemotron-3-Super | S2 → current Token Factory ids (C1, F2); Nemotron-3-Super maps to `nvidia/nemotron-3-super-120b-a12b` |
| NemotronChatRequest, NemotronOp, NemotronOptimization | type/operator/enum names, not models (token-factory, kernel-operators, nvidia-docs) |
| PaddleOCR, TrOCR, Tesseract | OCR: PaddleOCR via `oar-ocr` (adopted); TrOCR (not selected; ocrs covers the pure-Rust slot); Tesseract (rejected: native libs on Windows) |
| Whisper, Moonshine | v2 voice (`whisper-rs`; Moonshine ONNX evaluated then) |

---

## 16. v2 roadmap (declared, not dropped)

| Item | Source | Notes |
|---|---|---|
| Self-hosting: headless daemon on Nebius AI Cloud / home server; Docker Compose; NixOS flake; per-target packaging (musl static only where native deps allow, C38) | msg 36/37 | builds on polaris-daemon + polaris-rpc mTLS |
| Mobile: Slint mobile (Android Rust; iOS since Slint 1.12, LIKELY), PWA (already in polaris-web) | msg 37, D6 | quick chips, touch diff review (swipe approve/reject) |
| Voice: `polaris-audio` (cpal), `polaris-stt` (whisper-rs / Moonshine) | msg 0, 37 | S1 converts speech to structured tasks |
| Telegram/Discord bot gateway (`polaris-telegram`) | msg 37 | commit / try-again inline buttons |
| Zero-trust mesh (`polaris-wireguard`, boringtun) | msg 37 | no public IPs |
| Cellular-resilient session resumption (redb tokens) | msg 37 | replay missed events on reconnect |
| Push notifications (WebPush/APNs) | msg 37 | |
| macOS and Linux first-class support | init | seams exist from P1 |
| Slurm / Managed Kubernetes on Nebius AI Cloud | msg 9 | after remote gateway |

---

## 17. Glossary

| Term | Meaning |
|---|---|
| S1 / System One | Local typed-decision layer (GLiNER2 family) answering Choice / Score / Noul questions with probabilities. |
| S2 / System Two | Nemotron on Nebius Token Factory (reasoning, code). |
| T0 | Tier-zero nano model (Cactus Needle 3). |
| Frame | An agent = a context frame (role, token, scratchpad, history) over shared S1/S2. |
| CAS | Content-addressable store (BLAKE3 ids). |
| §ref | Short CAS reference replacing folded tool output in prompts. |
| E0–E4, E1w | Execution tiers (§6.6). |
| Contree | Nebius Token Factory Sandboxes. |
| Provider profile | OpenShell mechanism injecting credentials only for approved endpoints/binaries. |
| Blueprint | NemoClaw-style versioned YAML describing sandbox shape, policies and inference profiles. |
| Proof-of-Done | Gate that blocks task completion until build + tests pass in a sandbox and S1 sees no assertion tampering. |
| Blast radius | Pre-execution estimate of files/call sites affected by a command or change. |
| RTK | Diagnostic-stream filter + atomic issue ledger (port of the `harness rtk` reference). |
| PASS evidence | Command + raw output + conditions pasted into PHASES_STATUS.md. |
