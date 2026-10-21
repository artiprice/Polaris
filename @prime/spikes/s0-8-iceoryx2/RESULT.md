# S0-8 — iceoryx2 0.10 between two Windows processes

**RESULT: PASS** (latency, crash robustness, dead-node cleanup) **with a security condition** (shared-memory root
ACL) · raw output: `RESULT.raw.txt` · run 2026-10-20T21:45 · labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | iceoryx2 0.10 between two Windows processes: latency, robustness, cleanup after a crash |
| Kill criterion | crashes or leaks |
| Fallback | gRPC over named pipes only — remains the **control plane** regardless (PLAN ADR-013); iceoryx2 stays the later data-plane optimization |

## Method

`src/main.rs`: a driver process spawns a `pong` child; `u64` publish-subscribe both ways (`polaris/s08/ping`,
`polaris/s08/pong`); busy-poll round trips. Phase 1: 1,000 warm-up + 20,000 timed round trips. Phase 2: kill
the child (`TerminateProcess`), publish into the dead peer for 200 ms, then `Node::list` →
`NodeState::Dead(view).try_remove_stale_resources()`. Phase 3: respawn, reconnect, 5,000 round trips, graceful
stop. `inspect` mode lists nodes and files left under the shared-memory root after all processes exit.

## Measured (VERIFIED, `RESULT.raw.txt`; a LanceDB `cargo build -j4` ran concurrently)

| Metric | Result |
|---|---|
| build (release, `-j 2`) | 1 min 32 s; no libclang/bindgen or other extra per-PC install needed |
| first echo after spawn (incl. process start) | 88 ms |
| round trip, 20,000 samples | **p50 2.0 µs, p95 2.8 µs, p99 4.7 µs, max 536 µs**, 0 timeouts |
| kill peer → publish 200 ms | 439,082 sends OK, 0 errors (no back-pressure from a dead subscriber) |
| dead-node census after kill | alive 1, dead 1, **cleaned 1** |
| reconnect after respawn | 64 ms; 5,000 round trips p50 2.1 µs, p99 4.1 µs, 0 timeouts |
| graceful stop | child exit 0 |
| leftovers after all exits | 0 nodes; 1 file `iox2_…_node.0_10_0.global_mgmt` (16 B) — LIKELY a persistent management file, not a growing leak |

## Findings

1. **Security (VERIFIED): machine-wide shared-memory root.** On Windows iceoryx2 hard-codes `C:\Temp\iceoryx2\`
   (`iceoryx2-pal-configuration` `ICEORYX2_ROOT_PATH`) and **created `C:\Temp` itself** (absent before the run). It
   inherits `NT AUTHORITY\Authenticated Users:(M)` from `C:\`, so any local user can read, modify or delete the
   IPC files. Condition for adoption: set `Global::set_root_path` (exists, `config.rs:282`, VERIFIED) to a per-user
   directory such as `%LOCALAPPDATA%\polaris\iox2` with an owner-only ACL. UNVERIFIED: whether the separate
   `SHARED_MEMORY_DIRECTORY` constant follows the root path — test before adoption.
2. **Noisy logging (VERIFIED):** every process warns `User::from_uid(4294967295)` (no POSIX user database on
   Windows) and `No config file was loaded`; dead-node cleanup logs benign `FindNextFileA … os error 18`
   ("no more files") as errors. Route iceoryx2's logger into `polaris-log` at `warn+` with these filtered.
3. Publishing never blocks or fails on a dead subscriber (VERIFIED) — the supervisor must detect peer death
   itself (process handle / heartbeat), not rely on send errors.
4. Side effect cleaned: `C:\Temp\iceoryx2` removed after the run. The now-empty `C:\Temp` folder **remains**
   (removal of that path is blocked by the tool sandbox); it can be deleted manually if unwanted.

Conditions: Windows 11 LTSC 2024, rustc 1.99.0, iceoryx2 0.10.0 (default config), Ryzen 7 2700, concurrent compile load.
