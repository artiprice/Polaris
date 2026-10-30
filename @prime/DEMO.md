# Polaris demo script (P10-5)

A recorded walk-through of what Polaris does today, on synthetic workspaces only (the free
OpenRouter endpoint logs prompts, so no private code is shown to it). About 12 minutes. The
original MVP script (PLAN §11.7) used Token Factory and an OpenShell sandbox; both are out of
reach (D10, P2 paused), so models come from OpenRouter and tests run in the local Docker sandbox.

## Before recording

```powershell
powershell -File @prime/tools/demo/prepare.ps1          # demo workspaces in $env:TEMP\polaris-demo
$demo = Join-Path $env:TEMP "polaris-demo"
# keys in an explicit dotenv file, never a repository .env
'OPENROUTER_API_KEY=sk-or-...' | Out-File -Encoding utf8 $HOME\.polaris\.env
docker build -t polaris-ci-rust:1.99.0 @prime/tools/ci   # once; needs Docker Desktop
powershell -File @prime/tools/searxng/run.ps1            # optional, scene 5
polaris nv-docs-sync                                      # optional, scene 4 (a few minutes)
```

Record at 1920×1080, terminal font 16 pt, reduced motion off. Keep the OpenRouter free tier in
mind: about 50 requests a day, so rehearse scenes 2 and 6 sparingly.

## Scenes

| # | Minutes | Scene | Commands | What the viewer should see |
|---|---|---|---|---|
| 1 | 1 | Start | `polaris --workspace $demo\rust-bug` | Welcome aurora, provider line (`openrouter`, Nemotron), empty session |
| 2 | 3 | Rust bug fix with review | type: *"is_prime says 9 is prime; find and fix the bug"* | The model reads `src/lib.rs` with its tools, proposes a one-line `edit_file`; the review panel shows the diff; **Apply**; the turn ends with "1 file changed" |
| 3 | 2 | Tests in the sandbox | `/ci` then `/ci-run` | The pipeline (fmt → check → test) and its cache volumes; the run in Docker with the workspace mounted read-only, tests green; a second `/ci-run` is faster (warm cache) |
| 4 | 2 | CUDA advice | `polaris --workspace $demo\cuda`, then `/nv-profile`, `/nv-verify`, `/nv-docs grid stride loop` | Stack and build targets (sm_75, sm_90) against the local GPU; one finding per kernel (unchecked launch, missing bounds check, divergent barrier); documentation passages with their source URLs |
| 5 | 1 | Audit and search | `polaris --workspace $demo\hack`, then `/hack`, `/search-local balance update`, `/search-web cuda syncthreads divergence` | Two races, two blocking calls in async handlers and the CUDA barrier, each with `path:line`; ranked local passages; SearXNG results marked untrusted |
| 6 | 2 | One session, three windows | `polarisd --state-dir $demo\d --workspace $demo\rust-bug --session 42 --frame 7 --pipe polaris-demo --client-token-out $demo\client.token`; then `polaris-web --connect polaris-demo --session 42 --token-file $demo\client.token` and `polaris-desktop --connect polaris-demo --session 42 --token-file $demo\client.token` | A message typed in the web page appears in the desktop window; the daemon answers it (read-only) and both show the reply |
| 7 | 1 | Undo and history | back in scene 2's session: `/diff`, `/undo`, `/ci-run` | The applied change listed; `/undo` restores the buggy line; the test run is red again |

## What to say (one line each)

1. Polaris keeps every session as an ordered, durable event log; every frontend shows the same one.
2. Nothing is written without the person: the model proposes, the diff waits for Apply or Decline.
3. Workspace code never runs on the host: tests run in Docker with the workspace read-only.
4. NVIDIA knowledge is local: the profile, the CUDA advisor and a documentation seed with provenance.
5. Findings are heuristics with their location, never proof.
6. The daemon authenticates every frontend with its own short-lived credential.
7. The session log keeps every change, so it can be listed and undone.

## Fallback paths

- **No Docker:** skip scene 3 and the last step of 7; `/ci` still shows the pipeline and why Docker
  is missing. Everything else runs on the host without executing workspace code.
- **No model key or the free quota is spent:** scenes 1 and 3–5 and `/diff` work without a model;
  messages are still recorded. A local OpenAI-compatible server (Ollama on `127.0.0.1:11434`) can
  stand in once `local.model` is configured.
- **No WSL2:** nothing in this script needs it (the OpenShell/microVM paths are not part of it).
- **No network:** skip `/search-web` and `/nv-docs-sync`; an existing docs seed keeps working.

## Not in this script

ProofOfDone blocking a falsified completion needs the OpenShell sandbox profile (P2 paused), and the
calibrated System One gates need labelled data (P4-X-2); neither is shown. The video itself is
recorded by a person (P10-5 stays open until it exists).
