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

