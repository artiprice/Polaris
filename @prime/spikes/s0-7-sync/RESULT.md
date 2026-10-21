# S0-7 — Workspace sync Windows → OpenShell sandbox

**RESULT: PASS** (EOL, exec bit and deny-list fidelity; case-fidelity **not tested**) · raw output: `RESULT.raw.txt` ·
run 2026-10-20T21:35 · labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | 10k-file repo: initial and incremental sync time; byte-exact EOL/case; deny-list (`.polarisignore` defaults) |
| Kill criterion | no-op incremental > 5 s, or any byte mismatch |
| Fallback | tar-stream snapshot per run — **not needed** |

## Method

`src/main.rs`: deterministic synthetic repo (10,000 files in 1,000 dirs: `.rs` text, CRLF `.txt`, binary `.bin`,
`run.sh`, plus `.env`, `server.pem`, `.git/config` that must not leave Windows) → blake3 manifest → tar chunks
(≤ 896 KiB) piped through `ExecSandbox` stdin into `tar -xf - -C /sandbox/ws` (OpenShell has no upload RPC) →
sha256 of every file on both sides → change 3 files → incremental manifest/diff/push.

## Measured (VERIFIED, `RESULT.raw.txt`; a LanceDB `cargo build -j4` ran concurrently, so times are pessimistic)

| Metric | Result | Criterion |
|---|---|---|
| manifest #1 (blake3, 10,001 files) | 1,409 ms | — |
| no-op manifest + diff | **1,102 ms**, 0 changed | ≤ 5 s ✔ |
| initial push | 10,001 files, 61.4 MiB, 74 chunks, **10,445 ms** (≈ 5.9 MiB/s) | — |
| verification | local 10,001 = remote 10,001, **0 mismatches** | 0 ✔ |
| deny-list | 3 files denied locally, **0 present in sandbox** | 0 ✔ |
| EOL fidelity | `od -c` of a CRLF file shows `\r \n` preserved | byte-exact ✔ |
| exec bit | `run.sh` mode `755` preserved | ✔ |
| incremental (3 changed files) | manifest+diff 1,296 ms; push 1 chunk, 14,336 B, **74 ms** | — |

## Findings

1. **Gateway request limit = 1 MiB decoded (VERIFIED).** First attempt with 3 MiB chunks failed:
   `OutOfRange: decoded message length too large: found 2995802 bytes, the limit is: 1048576 bytes`.
   Chunks must stay below 1 MiB including protobuf framing.
2. **Error paths leak sandboxes (VERIFIED).** The failed attempt left `polaris-s0-7` behind (`AlreadyExists` on the
   next run) — the reaper / RAII guard in `polaris-kernel-reaper` must delete sandboxes on every exit path.
3. **Manifest cost dominates no-op sync** (≈ 1.1 s for 10k files, VERIFIED) — the P2 implementation should cache
   `(path, mtime, size) → blake3` and re-hash only changed entries (LIKELY ≤ 100 ms for no-op; UNVERIFIED).
4. **Not tested:** case-only renames / case-clashing paths (Windows is case-insensitive, Linux is not) and
   symlinks — move to the P2 `polaris-storage-sync` test matrix (hostile-paths fixtures, generated at test time).

Conditions: Windows 11 LTSC 2024, rustc 1.99.0 (release build), openshell-sdk rev 5a7728c3, gateway 0.1.4 docker
driver in WSL2 Ubuntu 24.04.5; Ryzen 7 2700, NVMe; concurrent compile load as noted.
