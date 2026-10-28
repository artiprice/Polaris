# E42 model lifecycle and native idle eviction evidence

VERIFIED scoped checkpoint captured before subsequent context/operator/release-tool implementation.
Full-system and complete-phase acceptance remain open.

| Command group | Exit | Raw output |
|---|---:|---|
| `members` | 0 | [members.txt](members.txt) |
| `layers` | 0 | [layers.txt](layers.txt) |
| `witness` | 0 | [witness.txt](witness.txt) |
| `workspace-check` | 0 | [workspace-check.txt](workspace-check.txt) |
| `scoped-tests` | 0 | [scoped-tests.txt](scoped-tests.txt) |
| `safety-clippy` | 0 | [safety-clippy.txt](safety-clippy.txt) |
| `scoped-format` | 0 | [scoped-format.txt](scoped-format.txt) |
| `supervised-real-worker` | 0 | [supervised-real-worker.txt](supervised-real-worker.txt) |
| `wsl-worker-models` | 101 | [wsl-worker-models.txt](wsl-worker-models.txt) |
| `sdk-full-tests` | 101 | [sdk-full-tests.txt](sdk-full-tests.txt) |
| `advisories` | 1 | [advisories.txt](advisories.txt) |
| `vet` | 4294967295 | [vet.txt](vet.txt) |
| `idle-default-real-model` | 0 | [idle-default-green.txt](idle-default-green.txt) |
| `child-eviction-reload` | 0 | [child-eviction-reload.txt](child-eviction-reload.txt) |
| `models-linux` | 0 | [models-linux.txt](models-linux.txt) |

The scoped five-package suite reports **160 passed, 0 failed, 6 ignored**. Three-package all-target
Clippy denies warnings, unwrap/expect/panic, numeric casts, indexing and unchecked arithmetic;
it is not full-workspace strict Clippy. The four installed-artifact cases above each use real CPU/fp32
GLiNER/ORT data; explicit and idle child cases both make a second real inference after unloading.
This is lifecycle coverage, not calibrated model parity, broad accuracy or an RSS benchmark.

The [native idle RED](review/worker/idle-default-red.txt) compiled and failed: after 46 idle seconds
explicit eviction still found one model. [GREEN](idle-default-green.txt) finds zero. New idle-policy
and client method API failures are compile evidence, not behavioral RED. The idle helper and CLI
tests were authored with their implementations and have post-implementation GREEN coverage.
The 20 model catalog/slot/manager tests are verification of existing behavior, not invented historical
test-first evidence. Agent Clippy runs used warnings only; the final root safety gate is stronger.
Its initial fixture arithmetic/clone diagnostics are retained; no assertion or gate was weakened.

The owner measures idle time after completed work; cancelled queued requests do not renew it.
Destruction stays on the native owner thread and completes before explicit eviction acknowledges.
A busy model is never preempted. A successful call does not establish RSS reduction, OS termination
or heavy-build admission exclusion. The generic ModelLoader requires each real implementation to
prevent handles escaping a lease and to own cancellation-safe cleanup; fixture tests cannot prove
those backend-specific requirements.

The standalone Linux model contracts pass **20 tests**. The broader offline Linux worker build
failed before compilation because aead 0.6.1 is not cached; worker portability remains UNVERIFIED.
SDK tests retain two schema-boundary failures. Cached advisories and cargo-vet retain failed release
gates with exact current output above, without exemptions. No dependency was added by E42.

[Review log hashes](retained-review-logs.json) identify original redirected output. Installed model
hashes in [the copied E41 observation](local-model-artifacts-e41.json) keep their earlier provenance;
they are not relabeled as an E42 hash observation. Current model paths are explicit non-secret
environment overrides in commands.json. Later source changes are not certified by this checkpoint.

Production model-slot assembly, resource/pressure policy, checkpoint-bound cache and batching,
PII-NER/CUDA, OCR/nano, full parity/calibration, prolonged stress/Loom/formal and release gates remain
unfinished. P1 historical acceptance, P2 pause and UI deferral remain in force.
