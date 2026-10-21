# S0-9 — LanceDB 0.40 on Windows MSVC (build cost + query latency)

**RESULT: FAIL on build cost → fallback chosen: usearch + SQLite is the default `VectorStore`; LanceDB becomes
an opt-in feature.** · raw output: `RESULT.raw.txt`, `usearch-probe/RESULT.raw.txt` · runs 2026-10-20 ·
labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | LanceDB 0.40 build on Windows (protoc, CMake 4, RAM, time) + query latency |
| Kill criterion | build fails, or > 30 min, or > 12 GB RAM |
| Fallback | usearch + SQLite metadata behind `VectorStore` — **verified on Windows in this spike** |

## LanceDB (VERIFIED, `RESULT.raw.txt`)

| Attempt | Outcome |
|---|---|
| 1 | failed after 1,614 s: `Could not find protoc` (pitfall W7) — lance build scripts need `protoc` on PATH or `PROTOC` |
| 2 | failed after 1,102 s: **lancedb 0.40.0 does not compile with default features** — `src/job.rs` uses `Error::Http`, which only exists under `#[cfg(feature = "remote")]` (E0599). 0.40.0 is the latest release (sparse index, 2026-10-20) |
| 3 | `features = ["remote"]` workaround: **OK** — 2,174 s (36 min 13 s) at `-j 4`, peak compiler working set **6.36 GB**, lowest free RAM 2.56 GB, `target/` **4.83 GB**, exe **249 MB** |

Query run (`s0-9-lancedb.exe 10000 384`, temp dir): connect 2 ms; create table 5,000 × 384 in 32 ms; add 5,000 in
32 ms; `count_rows` 10,000; **flat kNN (no index), k = 10, 50 queries: p50 8.65 ms, p95 9.39 ms**, max 40.2 ms;
peak working set 76 MB.

Against the kill criterion: no build failure once both workarounds are applied; RAM 6.4 GB < 12 GB ✔;
**time 36 min > 30 min ✘**. Caveat: measured at `-j 4` with other spikes and a Docker build running
concurrently — a clean `-j 16` build was not measured (UNVERIFIED; LIKELY under 30 min). The 4.8 GB target
directory and the 249 MB binary do not depend on that caveat.

## Fallback: usearch 2.26.5 + rusqlite 0.40.2 (bundled) (VERIFIED, `usearch-probe/RESULT.raw.txt`)

| Metric | Result |
|---|---|
| clean build (`-j 4`) | **1 min 41 s**; exe **4.3 MB**; no protoc, no extra per-PC install |
| SQLite metadata, 10,000 rows | 11 ms |
| HNSW add, 10,000 × 384 (cosine, f32, single thread) | 10.0 s (≈ 1 ms/vector — parallel insert is a P5 TODO) |
| **kNN HNSW + SQLite lookup, k = 10, 50 queries** | **p50 0.475 ms, p95 0.600 ms**, max 0.681 ms |
| peak working set | 27 MB |

## Decision and consequences

1. `polaris-storage-vector` default backend: **usearch (HNSW) + SQLite** (the registry DB already uses SQLite).
   LanceDB stays behind an opt-in cargo feature (`lancedb`), off in default and CI builds.
2. If the `lancedb` feature is ever enabled: `lancedb = { version = "0.40.0", features = ["remote"] }` until upstream
   gates `job.rs`; `protoc` on PATH (member-setup `win-protoc` becomes optional).
3. Upstream issue to file: lancedb 0.40.0 `job.rs` uses `Error::Http` without `cfg(feature = "remote")`.
4. P5 TODO: parallel HNSW insert; persistence (`index.save/load`) with atomic rename; recall@10 vs exact search
   on a labelled fixture (not measured here).

Conditions: Windows 11 LTSC 2024, rustc 1.99.0 release builds, CMake 4.3.0-rc3 with `CMAKE_POLICY_VERSION_MINIMUM=3.5`,
protoc 36.0, Ryzen 7 2700, 16 GB RAM, NVMe; concurrent load during the LanceDB builds as noted.
