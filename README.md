# Polaris

Polaris is a local-first coding agent written in Rust. A kernel owns every session as an ordered,
durable event log; the agent reads your workspace with bounded tools, proposes file changes that you
apply or decline one by one, and runs tests and CI only inside a sandbox. The same session is
available from a terminal UI, a native desktop window, a web page on loopback, and a daemon that
other frontends attach to.

> Status: active development. What works, what is verified and what is open is tracked phase by
> phase in [`@prime/PHASES_STATUS.md`](@prime/PHASES_STATUS.md); the design is in
> [`@prime/PLAN.md`](@prime/PLAN.md). Nebius services (Token Factory, Contree, AI Cloud, Data Lab)
> are not available to the team, so their integration is written but has never run against the
> real services (D10 in [`@prime/PHASES.md`](@prime/PHASES.md)); see "Nebius services" below.
> Models run through OpenRouter or a local server.

## Frontends

| Binary | What it is |
|---|---|
| `polaris` | Terminal UI (Ratatui) and the command line: `polaris web`, `polaris task`, `polaris doctor`, `polaris hack`, `polaris ci-run`, … |
| `polaris-desktop` | Native window (Slint) over the same session store; `--connect` attaches to `polarisd` |
| `polaris-web` | Web UI (Leptos SSR + hydration, installable PWA) on `127.0.0.1`; `--connect` attaches to `polarisd` |
| `polarisd` | Headless daemon: one kernel session served over a `polaris-*` named pipe (Unix socket elsewhere) with Biscuit token auth |

Every frontend shows the same session: typed messages and `$directives` are recorded in the kernel's
event log, replies stream from the model, and each proposed change waits in a review panel with its
diff until you apply or decline it.

## Requirements

- Windows 11 (the reference platform; Linux builds are being adapted), 16 GB RAM recommended.
- Rust **1.99.0** (pinned in `rust-toolchain.toml`) to build from source.
- A model provider for replies: an **OpenRouter** key (`OPENROUTER_API_KEY`), or a local
  OpenAI-compatible server such as Ollama on `127.0.0.1:11434`. Without one, messages are still
  recorded and the analysis commands below still work.
- Optional: **Docker Desktop** for `/ci-run` (build the CI image once, see below); an NVIDIA GPU and
  `nvidia-smi` for the local GPU notes of `/nv-profile`.

## Quick start

```powershell
# keys live in a dotenv file you pass explicitly (a repository .env is never read implicitly)
'OPENROUTER_API_KEY=sk-or-...' | Out-File -Encoding utf8 $HOME\.polaris\.env

polaris                              # terminal UI in the current folder
polaris web --port 8787              # web UI; prints http://127.0.0.1:8787/#token=…
polaris-desktop                      # desktop window
polaris doctor                       # what is configured, what is missing (installs nothing)
```

Attach frontends to one daemon session:

```powershell
polarisd --state-dir .polaris\daemon --workspace . --session 42 --frame 7 `
         --pipe polaris-main --client-token-out .polaris\client.token
polaris-web --connect polaris-main --session 42 --token-file .polaris\client.token
polaris-desktop --connect polaris-main --session 42 --token-file .polaris\client.token
```

The daemon answers each message with a read-only turn (keys from `--env-file` or
`~/.polaris/.env`; `--no-replies` only records them), and attached windows show the reply when it
finishes. A daemon started with `--listen host:port --tls-cert … --tls-key … --client-ca …` is
reached with `--connect tcp://host:port` and the client's files in `POLARIS_TLS_CA`,
`POLARIS_TLS_CERT` and `POLARIS_TLS_KEY`.

A recorded walk-through with prepared sample workspaces is in [`@prime/DEMO.md`](@prime/DEMO.md).

## Commands answered locally

These run on the session's workspace without the model, in every frontend and from the command line:

| Command | What it does |
|---|---|
| `/hack` · `polaris hack` | Race candidates (a value checked under one lock and written under another; atomic load then store), blocking calls in `async fn`, unsafe constructs, credentials and destructive shell commands in source, CUDA barrier misuse |
| `/nv-profile` · `polaris nv-profile` | NVIDIA stack of the workspace (CUDA, TensorRT-LLM, Triton, NCCL…), build target architectures, local GPUs |
| `/nv-verify` · `polaris nv-verify` | CUDA advisor: unchecked launches and API calls, missing grid bounds, divergent barriers, strided access, bank conflicts |
| `/ci` · `polaris ci` | The workspace's CI pipeline (format → check → test), its cache volumes and whether Docker can run it |
| `/ci-run` · `polaris ci-run` | Runs that pipeline in Docker: workspace mounted read-only, cargo home and target on cache volumes |
| `/search-local <words>` · `polaris search-local "<words>"` | Ranks workspace passages (lexical BM25, no embeddings) and shows each as `path:lines` with its best lines |
| `/nv-docs-sync` (`$nvidia-docs(sync)`) · `polaris nv-docs-sync` | Downloads the NVIDIA documentation listings (`llms.txt`) into a local seed in `~/.polaris/docs/nvidia`, incrementally, with each page's provenance |
| `/nv-docs <words>` · `polaris nv-docs "<words>"` | Searches that seed and shows each passage with its source URL |
| `/search-web <words>` · `polaris search-web "<words>"` | Web results from a local SearXNG (`@prime/tools/searxng/run.ps1`; `POLARIS_SEARXNG_URL`), query redacted first |
| `/diff`, `/undo` | The changes this session applied; restore the newest one whose file is unchanged since |

`$rag` adds the best workspace passages for each question to the request (as data, with their
`path:lines`), so replies can cite them; `$websearch` does the same with SearXNG results, and
`$nvidia(auto)` replies also get the best passages of the documentation seed once it exists.

`$nvidia(auto)` (the default) adds the NVIDIA profile and CUDA invariants to a reply's context when
a stack is found; `$nvidia(list)` keeps it out, `$nvidia(off)` disables both. Findings are
heuristics with a stated coverage, never proof.

Build the CI image once (needs Docker Desktop and pulls `rust:1.99.0-bookworm`):

```powershell
docker build -t polaris-ci-rust:1.99.0 @prime/tools/ci
```

## System One and the OpenShell sandbox

**System One** is a local classifier (GLiNER2.5-Decide, ONNX on CPU) that every session starts as a
worker process. It screens each proposed change against the safety schemas (tampering with
review/CI/test files, secrets in the new text, a diff that does not match its stated intent);
once a schema is calibrated, changes it accepts are applied without asking, everything else still
waits for you. Point it at the models in `~/.polaris/config.toml` (global layer only):

```toml
[system_one]
decide_model_dir = "D:/polaris-models/gliner2.5-decide-onnx-fp32"
ort_dylib = "D:/polaris-models/onnxruntime/onnxruntime.dll"
pii_model_dir = "D:/polaris-models/gliner2-pii-fp32"   # optional
# worker = "…/polaris-worker.exe"                      # default: next to the program
```

Calibrate once on a labelled set (`@prime/tools/system-one/make_safety_fixtures.py` writes a
synthetic one); a schema is kept only where its measured precision reaches 99 %:

```powershell
polaris s1-calibrate D:\polaris-models\calibration\safety-v1.jsonl
```

**OpenShell** (NVIDIA, v0.1.4, Docker driver in WSL Ubuntu) runs the model's `run_tests`: the
reply asks to run a test command, you approve the exact commands in the review panel, the
workspace is copied into an isolated sandbox with a deny-by-default policy, and the result is a
Proof of Done verdict. Bring it up once, then describe the gateway in `~/.polaris/sandbox.json`:

```powershell
wsl -d Ubuntu -u root -- bash /mnt/d/Polaris/@prime/tools/setup/setup-wsl-root.sh <user>
wsl -d Ubuntu -- bash /mnt/d/Polaris/@prime/tools/sandbox/bringup-wsl.sh /mnt/d/Polaris
```

```json
{
  "endpoint": "https://127.0.0.1:17670",
  "server_name": "localhost",
  "mtls_dir": "\\\\wsl.localhost\\Ubuntu\\home\\<user>\\.config\\openshell\\gateways\\openshell\\mtls",
  "policy": "D:/Polaris/@prime/tools/sandbox/acceptance-policy.yaml",
  "image": "sha256:<image_id printed by bringup-wsl.sh>",
  "wsl_distro": "Ubuntu"
}
```

The status of both is shown at startup and on the desktop's Settings page; a session whose
gateway is unreachable opens without the sandbox and says why.

## Nebius services

Everything Nebius-specific is opt-in and uses your own access; without it the commands say what is
missing. The integration follows the published API, MCP and CLI documentation but has not been run
against the live services from this repository.

| Service | What Polaris does with it | Setup |
|---|---|---|
| **Token Factory** | First provider of the chat chain (Nemotron by effort); `/nebius-status` lists models and fine-tuning jobs | `NEBIUS_API_KEY` in `~/.polaris/.env`; `[nebius]` in `~/.polaris/config.toml` |
| **Embeddings** | `/search-local` and `$rag` passages re-ranked by Token Factory embeddings of masked text | `"embedding_model": "BAAI/bge-en-icl"` in `~/.polaris/nebius.json` |
| **Data Lab** | `/datalab-sync` exports the session's trajectories with a passed Proof of Done (consent recorded by the command, masked, rescanned) and uploads them through the Files API; `/datalab-finetune [model]` starts a fine-tuning job | `data_lab` {`base_model`, `license`, `suffix`, `hyperparameters`} |
| **Contree** | `/contree-branches`; `polaris nebius contree-run [--sync] <image> <cmd>…` runs up to 8 branches from one image and workspace state; `polaris nebius swe-run <owner/repo> <commit> <cmd>` resolves the SWE environment image first | `contree-mcp --mode http` running locally; `contree` {`endpoint`, `egress_consent`}; trusted image annotations in `~/.polaris/contree-images.json` |
| **AI Cloud** | `polaris nebius cloud-up \| cloud-status \| cloud-down`: a remote OpenShell gateway VM under a budget reservation, a hard lifetime and an owner label, through the official `nebius` CLI | `ai_cloud` {`parent_id`, `subnet_id`, `platform`, `preset`, `hourly_usd`, `budget_usd`, `max_hours`} |
| **`$budget`** | `$budget(usd=…)` / `$budget(tokens=…)` stops new turns once the session's model spending reaches it | — |

```json
{
  "embedding_model": "BAAI/bge-en-icl",
  "contree": { "endpoint": "http://127.0.0.1:9452/mcp", "egress_consent": true },
  "data_lab": { "base_model": "meta-llama/Llama-3.1-8B-Instruct", "license": "Apache-2.0",
                "suffix": "polaris", "hyperparameters": { "n_epochs": 3, "lora": true } },
  "ai_cloud": { "parent_id": "project-…", "subnet_id": "vpcsubnet-…", "platform": "<platform>",
                "preset": "<preset>", "hourly_usd": "0.50", "budget_usd": "10", "max_hours": 4 }
}
```

## Safety model, in short

- File changes are proposed by the model and written only after you apply them; each change is
  verified against the exact file revision it was made for.
- Tests, CI and audits run in sandboxes (Docker locally); workspace code never runs on the host.
- The web UI listens on loopback only, checks a per-run token and the `Host` header, and sends a
  strict CSP. The daemon accepts only credentials from its own issuer, scoped to its session.
- Provider keys are read from an explicit dotenv file or the environment, never from the
  repository; logs pass through a redactor first.

## Build from source

```powershell
cargo build --release -p polaris-cli -p polaris-daemon -p polaris-desktop
# web UI (wasm + server): rustup target add wasm32-unknown-unknown; cargo install cargo-leptos --locked
cd apps/polaris-web; cargo leptos build --release
```

Distribution builds use `--profile dist`; the packaged binaries, their SHA-256 sums and build
information are described in [`@prime/evidence/p10`](@prime/evidence/p10).

Tests live under `tests/` mirroring the crate tree: `cargo test --workspace`. Workspace structure,
layers and dependency rules: `python -I @prime/tools/workspace.py check-layers`.

## Documentation

- [`@prime/PLAN.md`](@prime/PLAN.md) — architecture, crates, decisions (ADRs), threat model
- [`@prime/PHASES.md`](@prime/PHASES.md) and [`@prime/PHASES_STATUS.md`](@prime/PHASES_STATUS.md) — phases, exit criteria and evidence
- [`@prime/WEB_UI_PLAN.md`](@prime/WEB_UI_PLAN.md), [`@prime/UI_PLAN.md`](@prime/UI_PLAN.md) — interfaces
- [`docs/tool-catalog.md`](docs/tool-catalog.md) — the agent's tools

## Licence

Apache-2.0 — see [LICENSE](LICENSE). Slint is used under its royalty-free licence (ADR-014); the
attribution is shown in the desktop app's About panel.
