# S0-4 — GLiNER2.5-Decide in Rust (`gliner2-rs`) vs Python `gliner2`

**RESULT: PASS** (conditional — see "Conditions of the PASS": one text pre-processing rule must be replicated, and the RSS margin is thin)

Date of runs: 2026-10-20. Labels: VERIFIED = observed in this session's output; LIKELY = inferred; UNVERIFIED = not observed.

## Verdict against the kill criterion

| Criterion | Measured | Status |
|---|---|---|
| top-1 parity >= 99 % vs Python | **260/260 = 100.00 %** task-level (200/200 items), fp32 ONNX, max abs prob diff **4.0e-6** (with the text rule below). Without the rule: 244/260 = 93.85 % (FAIL) | VERIFIED |
| Rust resident memory <= 2.5 GB | steady after load **2113 MiB**; peak **2409 MiB** (in-process sampler) / **2407 MiB** (external PeakWorkingSet64 poll) = 2.35 GiB = 2.52 GB (decimal) | VERIFIED; PASS in GiB, 0.02 GB over if "GB" is decimal — borderline |

The 100 % is parity against the Python reference **on this synthetic golden set**; it says nothing about model quality (see Open issues).

### Conditions of the PASS
1. **Trailing-period normalisation (VERIFIED).** Python `gliner2` 2.1.0 `SchemaTransformer._normalize_text` appends `.` to any text not ending in `.`, `!` or `?` (empty -> `"."`). `gliner2-rs` 0.9.6 does not. Fed raw text, Rust tokenises one token fewer (checked on ids g000, g060, g126, g186, g187: Python 61/75/43/96/93 tokens vs Rust 60/74/42/95/92) and parity drops to 93.85 %. The harness applies the same rule (`main.rs`, arg `normalize`); the production S1 client MUST do the same. Raw-text result kept in `artifacts/compare_fp32_raw.txt` / `golden_rust_fp32_raw.jsonl`.
2. **Memory** 2.35 GiB peak leaves ~150 MiB headroom under 2.5 GiB. Any extra resident item (second session, larger batch, longer text) can exceed it. fp16 does NOT help on CPU (below). LIKELY: a PII-NER model in the same worker would break the budget (UNVERIFIED, not measured).
3. CUDA EP not measured (see Open issues).

## Conditions (hardware / versions) — VERIFIED from tool output
- Ryzen 7 2700 (8C/16T), 16 GB RAM shared (a LanceDB build and other jobs ran concurrently — latencies are noisy), GTX 1650 4 GB, driver 617.42 (CUDA UMD 13.4), Windows 11 Enterprise LTSC 24H2.
- Threads: 4 intra-op (torch `set_num_threads(4)`, ORT `with_intra_threads(4)`). CPU only.
- Model: `fastino/GLiNER2.5-Decide` rev `9d1bfb848cd16d93ee56ccbc59cd92bf0f284d90`; config `"architecture": "span"`, deberta-v3-large, max_width 8, `counting_layer=count_lstm`, `MAX_COUNT=20`.
- Python 3.12.15 (uv venv in spike dir), `gliner2` 2.1.0, `torch` 2.14.1+cpu, `transformers` 5.20.2, `onnx` 1.23.2, `onnxscript` 0.7.2, python `onnxruntime` 1.31.0 (not used for inference).
- Rust: cargo 1.99.0; `gliner2-rs =0.9.6` (default-features off), `ort =2.0.0-rc.13` (+ `ort-sys` 2.0.0-rc.13, from `rust-harness/Cargo.lock`), features `std, ndarray, load-dynamic`.
- ONNX Runtime DLL: official `onnxruntime-win-x64-1.23.2.zip` (github.com/microsoft/onnxruntime v1.23.2, zip sha256 `0b38df9a…c9f3cd`), `onnxruntime.dll` copied beside the exe (sha256 prefix `dec964ab1ee36cc9`), `ORT_DYLIB_PATH` set to that absolute path. Engine reported `exec=IoBinding layout=Flat` (crate's own choice; on CPU).

## Export method (VERIFIED)
Used the exporter that `gliner2-rs` ships, unchanged: `research/gliner2-rs-src/onnx_conversion_scripts/export_span_v3.py` (repo dariofinardi/gliner2-rs @ `7626d237d8c58f0fcfadd094e8360e7d5ae9c8ee`). It rejects `boundary` checkpoints and accepts this `span` one. It exports 8 fragments (encoder, token_gather, span_rep, schema_gather, count_pred_argmax, count_lstm_fixed, scorer, classifier) via legacy TorchScript `torch.onnx.export(dynamo=False)`, opset 17, plus fp16 variants. Total export wall time ~6 min.
- Wrapper `run_export.py` only imports the script and adds a peak-RSS sampler and a `MemoryError` guard around the fp16 conversion. Peak RSS of export: 8.38 GB (!) — the first two attempts failed: (1) `UnicodeEncodeError` (cp1252 console; fixed with `python -X utf8`), (2) `RuntimeError: Could not allocate bytes object` in `torch.onnx.export` for the 1.7 GB encoder when free RAM was ~5-7 GB (the same command succeeded later with more free RAM), (3) a `MemoryError` in the fp16 `infer_shapes` step on the 3rd attempt, which the guard would have skipped; the 4th run completed everything. Exporting needs roughly 8 GB free RAM — conflicts with "peak under 6 GB" and with concurrent builds.
- Sizes (VERIFIED): fp32 fragments total **1,947,765,929 B = 1.81 GiB** (encoder 1,737,503,821 B); fp16 (keep_io fp32) total 975,633,484 B = 0.91 GiB. fp16 iobinding variants were produced and deleted for disk space.
- On disk now: only the fp32 variant is kept in `C:\Users\Admin\.polaris\models\gliner2.5-decide-onnx\` (+ `tokenizer.json`). fp16 files deleted after measurement (disk request).

## Golden set
`gen_golden.py` (seed 20261020) -> `golden_inputs.jsonl` (sha256 `b17872dd…c00033`, 200 items): 60 Choice (intent over 5 labels), 60 Noul (`"Does this diff disable or delete a test assertion?"` -> yes/no on synthetic diffs), 50 Score (labels "0".."10"), 30 multi3 (Choice + Noul + Score in ONE call; 90 task-level decisions). Total 260 task-level decisions. `ref_python.py` -> `golden_python.jsonl` (label + full softmax distribution captured by spying on `_decode_classification_probabilities`; label is the public `classify_text` output).

## Commands run (exact, from the spike dir `C:\mDev\Polaris\@prime\spikes\s0-4-gliner`)
```
python -m uv venv .venv --python 3.12
python -m uv pip install --python .venv/Scripts/python.exe "gliner2[local]" onnx onnxruntime psutil onnxscript onnxconverter-common numpy   (UV_CACHE_DIR=C:\Users\Admin\.polaris\models\uv-cache)
git clone --depth 1 https://github.com/dariofinardi/gliner2-rs.git research/gliner2-rs-src
HF_HOME=C:\Users\Admin\.polaris\models\hf  .venv/Scripts/python.exe -X utf8 -P run_export.py research/gliner2-rs-src/onnx_conversion_scripts/export_span_v3.py fastino/GLiNER2.5-Decide C:/Users/Admin/.polaris/models/gliner2.5-decide-onnx
.venv/Scripts/python.exe -X utf8 -P gen_golden.py
HF_HUB_OFFLINE=1 .venv/Scripts/python.exe -X utf8 -P ref_python.py
gh release download v1.23.2 -R microsoft/onnxruntime -p onnxruntime-win-x64-1.23.2.zip   (unzipped; lib/onnxruntime.dll copied beside exe)
cargo build --release -j 4 --manifest-path rust-harness/Cargo.toml
ORT_DYLIB_PATH=<abs>\rust-harness\target\release\onnxruntime.dll
rust-harness/target/release/s0-4-harness.exe C:\Users\Admin\.polaris\models\gliner2.5-decide-onnx fp32 4 golden_inputs.jsonl golden_rust_fp32.jsonl bench_rust_fp32.json normalize
rust-harness/target/release/s0-4-harness.exe ... fp16 4 golden_inputs.jsonl golden_rust_fp16.jsonl bench_rust_fp16.json normalize
.venv/Scripts/python.exe -X utf8 -P compare.py golden_rust_fp32.jsonl     (and golden_rust_fp16.jsonl, golden_rust_fp32_raw.jsonl)
```
`dump_py_ids.py` / `rust-harness/src/bin/dump_ids.rs` dump Python vs Rust input_ids (used to find the trailing-period difference; artifacts in `artifacts/py_ids.json`, `rs_ids.json`).

## RAW parity output (fp32 ONNX, `normalize`) — `artifacts/compare_fp32.txt`
```
ALL                                                     top1 260/260 = 100.00%  max|dp|=0.000004
choice                                                  top1 60/60 = 100.00%  max|dp|=0.000004
multi3/Does this task require changing more than one file? top1 30/30 = 100.00%  max|dp|=0.000001
multi3/intent                                           top1 30/30 = 100.00%  max|dp|=0.000001
multi3/task_complexity                                  top1 30/30 = 100.00%  max|dp|=0.000000
noul                                                    top1 60/60 = 100.00%  max|dp|=0.000001
score                                                   top1 50/50 = 100.00%  max|dp|=0.000000
item-level (all tasks of the item agree): 200/200
mismatches (id, task, py_top, rust_top, py_margin, max|dp|): []
smallest python top1-top2 margins: [(0.00025, 'g164'), (0.00032, 'g139'), (0.00032, 'g146'), (0.00096, 'g136'), (0.00103, 'g143')]
```
Without the trailing-period rule (`artifacts/compare_fp32_raw.txt`):
```
ALL                                                     top1 244/260 = 93.85%  max|dp|=0.182236
choice                                                  top1 60/60 = 100.00%  max|dp|=0.182236
multi3/intent                                           top1 26/30 = 86.67%  max|dp|=0.088206
score                                                   top1 38/50 = 76.00%  max|dp|=0.018237
```
fp16 ONNX (keep_io fp32), `artifacts/compare_fp16.txt`: `ALL top1 260/260 = 100.00%  max|dp|=0.001005`, 200/200 items.
The golden set includes near-ties (smallest Python margin 0.00025) that still agree at fp32, so the 100 % is not just easy cases; however all 260 agree and the distribution of score labels in the set is narrow (see Open issues).

## RAW latency / memory (CPU, 4 threads, concurrent load on the machine)
Rust fp32 (`bench_rust_fp32.json`; 3 warm-up calls, then 40 single-task calls / 30 three-task calls):
```
"load_s": 12.6971525, "rss_after_load_MB": 2113, "peak_rss_sampled_MB": 2409,
"latency_1task": {"n": 40, "p50_ms": 363.3617, "p95_ms": 481.2181, "min_ms": 290.2404, "max_ms": 536.3508},
"latency_3task": {"n": 30, "p50_ms": 668.1487999999999, "p95_ms": 757.9903999999999, "min_ms": 558.6611, "max_ms": 801.7923000000001}
```
An earlier identical fp32 run (raw text): load 19.63 s, 1-task p50 405 ms / p95 511 ms, 3-task p50 771 ms / p95 810 ms, peak 2408 MB — run-to-run noise ~10 %.
External check (PowerShell polling `PeakWorkingSet64` every 100 ms of a full fp32 run): `PeakWorkingSet64 polled max = 2,524,184,576 bytes = 2,407 MiB`.

Rust fp16 (`bench_rust_fp16.json`): `load_s 11.63, rss_after_load_MB 3001, peak_rss_sampled_MB 3035, 1-task p50 891.7 / p95 1031.9 ms, 3-task p50 1333.2 / p95 1479.9 ms` — larger and slower than fp32 on this CPU (fp16 weights are up-cast; no native fp16 CPU kernels). Do not use fp16 on CPU.

Python `gliner2` (`py_bench.json`, torch eager, 4 threads, fp32):
```
"load_s": 23.585863399999653, "rss_after_load_GB": 2.2563705444335938, "peak_wset_GB": 4.068641662597656,
"latency_1task": {"n": 40, "p50_ms": 1071.0148500002106, "p95_ms": 1270.3230000006442, ...},
"latency_3task": {"n": 30, "p50_ms": 1111.2575000006473, "p95_ms": 1445.5997999975807, ...}
```
So Rust/ORT is ~2.9x faster than torch eager for 1 task and ~1.7x for 3 tasks on this machine. The 3-task call is ~1.8x the 1-task time in Rust, i.e. multi-schema batching saves little vs. separate calls (each task adds its own prompt tokens to the sequence). Vendor's "~167 ms" (48-core) and the chatlog "<15 ms" are not reached: 363 ms p50 here. Plan R5 stands (latency budgets must be sized for ~0.4-0.8 s per call on CPU).
Python-sidecar fallback characterized as well: 2.26 GB steady RSS, 4.07 GB peak at load, 1.07 s p50 — i.e. the fallback is worse on every axis, and not needed.

## Open issues
1. **CUDA EP not measured (UNVERIFIED).** Installed: CUDA toolkit v13.3 only; no `cudart64_12*` and no `cudnn64_9*` found (searched `C:\Program Files\NVIDIA GPU Computing Toolkit\CUDA` and `...\NVIDIA\CUDNN`). Per the brief, nothing was installed. The fp16-iobinding fragments for CUDA were exported (and then deleted for disk space); re-export takes ~6 min and ~8 GB RAM. 4 GB VRAM fits the 0.9 GB fp16 encoder (LIKELY, unmeasured).
2. **RSS margin** (above): 2.35 GiB peak vs 2.5 GiB limit. A lower-memory option to investigate (UNVERIFIED): int8 dynamic quantisation of the encoder, or memory-mapped external data.
3. **Rust-side text rule** (trailing period) is a hidden contract between `gliner2` Python and `gliner2-rs`; add it to `polaris-system-one-engine` plus a golden-parity regression test in CI (P4-X-1). Other Python-2.1.0 behaviours (e.g. `max_len` truncation, long-text chunking) were not exercised: the golden texts are short (<= ~100 tokens). UNVERIFIED for long inputs.
4. **Model quality on Noul/Score prompts is weak (VERIFIED, separate from parity):** vs. my synthetic "intended" labels Python gets Choice 59/60, Noul 28/60, Score 8/50; predicted Score labels collapse to {0,1,2,10} (13/13/4/20). The "yes/no question as task name" and "0..10 ordinal" mappings in PLAN §6.2 need calibration/fixtures before being trusted; the model card only demonstrates 0..5 scores. (Intended labels are my own heuristics, not ground truth.)
5. Export is memory hungry (8.4 GB peak RSS, failed twice under memory pressure). P4-X-1 xtask should require >= 10 GB free or export fragments separately; the legacy TorchScript exporter is deprecated in torch 2.9+ (`DeprecationWarning` in export log) — pin the torch version used.
6. The engine chose `ExecutionMode::IoBinding` while running on CPU; harmless here but unexplained (LIKELY an `Auto` heuristic of 0.9.6). Not investigated.
7. Disk: spike dir holds `.venv` (torch) and `rust-harness/target` (both git-ignored by repo `.gitignore` lines 31/37, as are `artifacts/`). `research/gliner2-rs-src` (clone, `.git` removed) is NOT ignored and shows as untracked — delete it when done; the export script's provenance is the commit hash above. HF cache (`~/.polaris/models/hf`, 1.9 GB) and the fp32 ONNX (1.8 GiB) are kept; the uv cache was pruned/cleaned.
8. Nothing was committed; no repo file outside the spike dir was modified by this work (other `M` entries in `git status` belong to other jobs).

## Files
`export.log`, `golden_inputs.jsonl`, `golden_python.jsonl`, `golden_rust_fp32.jsonl`, `golden_rust_fp16.jsonl`, `golden_rust_fp32_raw.jsonl`, `py_bench.json`, `bench_rust_fp32.json`, `bench_rust_fp16.json`, `bench_rust_fp32_raw.json`, `compare.py`, `artifacts/*`, scripts `gen_golden.py`, `ref_python.py`, `run_export.py`, `dump_py_ids.py`, `rust-harness/`.
