# P4 GLiNER export pipeline

`gliner_export.py` wraps the exact S0-4 exporter and Python reference tools. It pins the Decide checkpoint revision, exporter bytes, golden generator/reference bytes, and records Python dependency versions and SHA-256 for all actual input/output files. The upstream exporter copy is unchanged from `dariofinardi/gliner2-rs` revision `7626d237d8c58f0fcfadd094e8360e7d5ae9c8ee`; its Apache-2.0 license is in `upstream/LICENSE`.

The tool has no model-download fallback. The pinned checkpoint must already be present in the local Hugging Face cache. `validate` is metadata-only and checks that proposed output targets are new. Both export and Python golden inference refuse to run below 10 GiB currently available RAM; this threshold exceeds the S0-4 measured 8.38 GB export peak and the 4.07 GB reference peak. Export and goldens are written to fresh staging directories and promoted only after all required files and deterministic row/category constraints pass.

From the repository root, run with the S0-4 environment:

```powershell
$python = '@prime/spikes/s0-4-gliner/.venv/Scripts/python.exe'
& $python -X utf8 -P @prime/tools/system-one/gliner_export.py validate
& $python -X utf8 -P @prime/tools/system-one/gliner_export.py export "$env:USERPROFILE/.polaris/models/gliner2.5-decide-onnx-p4"
& $python -X utf8 -P @prime/tools/system-one/gliner_export.py goldens crates/polaris-system-one/crates/engine/tests/fixtures/s1/decide
```

`validate` exits 3 when inputs are ready but current free RAM is below the export threshold. Do not run the export while that state persists or while other memory-intensive jobs are active. Golden fixtures are deterministically generated synthetic inputs; the `intended` labels are metadata, not calibrated ground truth. `golden_python.jsonl` contains outputs from actual inference by the pinned Python model.

Focused metadata/contract tests:

```powershell
python -I @prime/tools/system-one/test_gliner_export.py -v
```

This pipeline does not claim the native Rust parity gate is satisfied; compare generated Python references to actual Rust outputs using the S0-4 comparator after export/build, and retain per-precision parity results.


## int8 evaluation (2026-10-28)

`quantize_int8.py` derives encoder-only variants from a complete fp32 export (heads stay fp32;
gliner2-rs `Precision::Fp32` file names, real precision in `manifest.json`). Measured with the S0-4
Rust harness (4 threads, i7-10850H, ONNX Runtime 1.23.2 unless noted) on the 200-row golden set:

| Variant | top-1 vs Python | max \|dp\| | RSS after load | p50 1-task |
|---|---|---|---|---|
| fp32 (reference export) | 260/260 = 100% | 0.000004 | 2114 MB | 239 ms |
| dynamic int8, per-tensor | 201/260 = 77.3% | 0.709 | 1256 MB | 199 ms |
| dynamic int8, per-channel | 210/260 = 80.8% | 0.679 | 1257 MB | 203 ms |
| dynamic int8, FFN only, per-channel | 204/260 = 78.5% | 0.687 | 1542 MB | 351 ms |
| 8-bit block-32 weight-only (MatMulNBits) | 260/260 = 100% | 0.0196 | 1282 MB | 1753 ms |
| same, `--accuracy-level 4` | 260/260 = 100% | 0.0196 | 1283 MB | 2149 ms |
| same, ONNX Runtime 1.31 DLL | 260/260 = 100% | 0.0196 | 1285 MB | 1515 ms |

Dynamic activation quantization breaks the parity gate (score/intent labels have tiny Python
margins). Block-wise 8-bit weights keep exact parity and cut memory 39 %, but the CPU MatMulNBits
8-bit path is 6–9× slower than fp32. No variant passes both the ≥ 99 % parity gate and the latency
budget yet; fp32 remains the only fully passing Decide artifact.
