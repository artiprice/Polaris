# Polaris — Phase status

> **Document:** `@prime/PHASES_STATUS.md` · **Snapshot:** 2026-10-20 · **Branch:** `mvp` (uncommitted
> working tree) · **Machine:** Windows 11 LTSC 2024, Ryzen 7 2700, 16 GB RAM, GTX 1650 4 GB,
> rustc/cargo 1.99.0.
>
> Rules: `PASS` requires the command, its raw output and the conditions (Evidence log §E). `BLOCKED-ON-ACCESS`
> = needs a key/account that does not exist yet (D9). A doc-only skeleton is **not** an implementation.
> Crate rows are generated from `@prime/tools/workspace.toml` (`workspace.py phase-tables --kind status`).

## Summary

| Status | Rows | Notes |
|---|---|---|
| PASS | 10 | P0-SKEL-1…9 + milestone M0, each with evidence (§E) |
| IN PROGRESS | 0 | |
| BLOCKED-ON-ACCESS | 9 | waiting for Nebius/Tavily keys and accounts |
| NOT STARTED | 121 | all implementation, environment and spike tasks |

Current phase: **P0 — Foundations & de-risking**. Next actions (in order): P0-ACC-1..4 (request keys —
user action), P0-ENV-1 (Ubuntu WSL2), P0-ENV-2 (OpenShell gateway), S0-1, S0-7, S0-9, S0-4, S0-10.

## P0 — tasks

| Task | Status | Evidence / notes |
|---|---|---|
| P0-SKEL-1 `.gitignore` anchored, Cargo.lock tracked | PASS | §E1 (paths not ignored; private files still ignored) |
| P0-SKEL-2 `.gitattributes` | PASS | file present (LF default; CRLF for .ps1/.bat/.cmd) |
| P0-SKEL-3 `workspace.toml` (93 packages, 156 ext deps) | PASS | §E2 |
| P0-SKEL-4 `workspace.py` tooling | PASS | §E2–§E7 (all subcommands exit 0) |
| P0-SKEL-5 root `Cargo.toml` + `rust-toolchain.toml` generated | PASS | §E3 (`cargo metadata`: 93 packages) |
| P0-SKEL-6 doc-only skeleton compiles | PASS | §E4 (`cargo check --workspace --all-targets` with `-Dwarnings`; `cargo doc` with `-Dwarnings`; `cargo fmt --check`) |
| P0-SKEL-7 member + layer checks | PASS | §E5 (93 members; 252 edges; no violations) |
| P0-SKEL-8 version audit + resolution probe | PASS | §E6 (156/156 OK; lockfile resolved — 1,699 packages; see probe history) |
| P0-SKEL-9 PLAN.md / PHASES.md / PHASES_STATUS.md; inventory 100 % covered | PASS | §E8 (`check-docs` OK; `chatlog-inventory --check @prime/PLAN.md`: all 396 items present) |
| P0-ACC-1 Token Factory API key | BLOCKED-ON-ACCESS | user action: request key |
| P0-ACC-2 Token Factory Sandboxes (Contree) access | BLOCKED-ON-ACCESS | user action |
| P0-ACC-3 Nebius AI Cloud quota | BLOCKED-ON-ACCESS | user action |
| P0-ACC-4 Tavily API key | BLOCKED-ON-ACCESS | user action |
| P0-ENV-1 Ubuntu 24.04 in WSL2 + `.wslconfig` | NOT STARTED | today only `docker-desktop` distro exists; no `.wslconfig` |
| P0-ENV-2 OpenShell v0.1.4 gateway in WSL2 | NOT STARTED | depends on P0-ENV-1 |
| P0-ENV-3 tooling (nextest, fuzz, mutants, leptos, deny, vet, audit, uv, kani) | NOT STARTED | none installed (verified absent) |
| P0-ENV-4 `.cargo/config.toml` env, short target-dir, longpaths | NOT STARTED | |
| P0-ENV-5 Linux `cargo check` leg in WSL2 | NOT STARTED | depends on P0-ENV-1 |
| P0-ENV-6 local OpenAI-compatible server (Ollama/llama.cpp) | NOT STARTED | |
| P0-ENV-7 `deny.toml`, cargo-vet, Slint licence decision | NOT STARTED | |
| P0-XT-1 Rust xtask port of checks | NOT STARTED | `xtask` is a doc-only lib today |

## P0 — spikes

| Spike | Status | Result / fallback chosen |
|---|---|---|
| S0-1 openshell-sdk on windows-msvc + WSL2 gateway | NOT STARTED | — |
| S0-2 nested KVM + vm driver | NOT STARTED | — |
| S0-3 NemoClaw custom blueprint | NOT STARTED | — |
| S0-4 GLiNER2.5-Decide ONNX parity + latency | NOT STARTED | — |
| S0-5 Token Factory capabilities | BLOCKED-ON-ACCESS | needs P0-ACC-1 |
| S0-6 Needle 3 on Windows/WSL2 | NOT STARTED | — |
| S0-7 workspace sync performance/fidelity | NOT STARTED | — |
| S0-8 iceoryx2 on Windows | NOT STARTED | — |
| S0-9 LanceDB build on Windows | NOT STARTED | resolution only verified (§E6) |
| S0-10 crossterm Shift+Enter on ConPTY | NOT STARTED | — |
| S0-11 OpenShell mxc driver | NOT STARTED | — |
| S0-12 guest build + handshake | NOT STARTED | — |
| S0-13 Contree MCP auth + branching | BLOCKED-ON-ACCESS | needs P0-ACC-2 |

## Cross-cutting phase tasks

| Task | Status |
|---|---|
| P1-X-1, P1-X-2, P1-X-3 | NOT STARTED |
| P2-X-1, P2-X-2 | NOT STARTED |
| P3-X-1 | NOT STARTED |
| P3-X-2, P3-X-3, P3-X-4 | BLOCKED-ON-ACCESS |
| P4-X-1, P4-X-2, P4-X-3 | NOT STARTED |
| P5-X-1, P5-X-3 | NOT STARTED |
| P5-X-2, P5-X-4 | BLOCKED-ON-ACCESS |
| P6-X-1, P6-X-2, P6-X-3 | BLOCKED-ON-ACCESS |
| P7-X-1, P7-X-2, P7-X-3 | NOT STARTED |
| P9-X-1, P9-X-2 | NOT STARTED |
| P10-1 … P10-6 | NOT STARTED |

## Milestones

| Milestone | Status |
|---|---|
| M0 skeleton compiles, layers enforced, versions audited, probe green | PASS (§E3–§E6) |
| M1 … M6 | NOT STARTED |

## Crate tasks (generated)

Every package currently exists **only as a documentation skeleton** (P0-SKEL-6). No crate is implemented.

### P0

<!-- generated: P0 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-tests` | `polaris-tests` | P0 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:xtask` | `xtask` | P0 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P1

<!-- generated: P1 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-cli` | `polaris-cli` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-sdk` | `polaris-sdk` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-proto` | `polaris-proto` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-config` | `polaris-config` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-log` | `polaris-log` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-telemetry` | `polaris-telemetry` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-code-intel` | `polaris-code-intel` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-storage-cas` | `polaris-storage-cas` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-storage-eventlog` | `polaris-storage-eventlog` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-storage-registry` | `polaris-storage-registry` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-storage-cache` | `polaris-storage-cache` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-storage` | `polaris-storage` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-bus` | `polaris-kernel-bus` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-directives` | `polaris-kernel-directives` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-supervisor` | `polaris-kernel-supervisor` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel-reaper` | `polaris-kernel-reaper` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-tools-builtin` | `polaris-tools-builtin` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-tools-rtk` | `polaris-tools-rtk` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-tools` | `polaris-tools` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-kernel` | `polaris-kernel` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui-model` | `polaris-ui-model` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui-cli` | `polaris-ui-cli` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-ui` | `polaris-ui` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-spike-bus` | `polaris-spike-bus` | P1 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |

### P2

<!-- generated: P2 crate tasks -->
| Task | Crate | Phase | Status | Evidence |
|---|---|---|---|---|
| `crate:polaris-storage-sync` | `polaris-storage-sync` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
| `crate:polaris-security-automata` | `polaris-security-automata` | P2 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
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
| `crate:polaris-kernel-verifier` | `polaris-kernel-verifier` | P3 | NOT STARTED | doc-only skeleton (see P0-SKEL-6) |
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
```
