VERIFIED owned-code NER implementation evidence

Definition of done for this owned slice: a separately configured local GLiNER2 PII engine returns bounded raw UTF-8 byte spans through an authenticated worker RPC; the kernel client verifies range/confidence bounds; the vault combines valid NER spans with DFA/entropy masking, preserves reversible markers, and degrades deterministically on detector failure/timeout; Decide and PII engines have separate identities and are never resident simultaneously. The cache identity also includes an OS-random per-worker incarnation so identical loaded model generation/digest cannot be reused across process restarts.

Commands and observed results:
- `cargo test -p polaris-worker -p polaris-system-one-engine -p polaris-system-one-client -p polaris-proto -p polaris-security-pii --tests` — exit 0; full output retained in `focused-tests.txt`. This included 5 vault PII tests, the authenticated client PII span and timeout path, worker no-model/cap/incarnation tests, engine domain-separation/raw-byte span fixture, and wire schema contracts. The local model tests are ignored.
- `cargo clippy -p polaris-sdk -p polaris-security-pii -p polaris-system-one-engine -p polaris-system-one-client -p polaris-proto -p polaris-worker --all-targets --no-deps -- -D warnings` — exit 0; output retained in `scoped-clippy.txt`.
- `python -I @prime/tools/workspace.py check-layers` — `check-layers: 93 packages, 280 edges -> OK`; output retained in `workspace-layers.txt`.
- `cargo fmt -p polaris-sdk -p polaris-security-pii -p polaris-system-one-engine -p polaris-system-one-client -p polaris-proto -p polaris-worker` — exit 0.
- `cargo test -p polaris-system-one-engine --test real_pii_model -- --ignored` — NOT RUN. Live companion-model artifacts were not provided in this verification; `real_pii_model_returns_bounded_raw_utf8_spans` remains ignored. No actual PII inference or quality result is claimed.
- `@prime/tools/runtime/llama-b11804/llama-server.exe`, PID 15100, was verified under the owned workspace runtime path and stopped after parent confirmed local acceptance had ended; exact metadata is retained in `llama-owned-process-teardown.txt`.

API/behavior notes:
- `PiiEntitySpan` carries only half-open UTF-8 byte offsets and confidence; the RPC has no returned text/label field.
- The worker separately lazy-loads `PiiEngineConfig`, uses raw text with no Decide period normalization, pins CPU/fp32/standard execution, uses all seven pinned privacy groups, and caps input at `polaris_proto::limits::MAX_PII_INPUT_BYTES` (16384 bytes). It uses the pinned crate's long-input chunking path.
- The worker evicts the other native model before loading one, uses the same thread-affine owner, and gives PII its own idle deadline and health digest/generation.
- `PiiVault::mask_with_ner` checks the vault input limit before dispatch, drops detector work at the configured deadline, falls back to DFA/entropy for unavailable/timeout/malformed ranges, and preserves actual coverage in `SanitizeReport`.
- Model outputs are synthetic in the fast vault and client fixture tests. Only the ignored real-artifact test can establish live inference behavior.

Remaining acceptance gaps:
- Actual companion ONNX files and ORT execution are not verified in this run.
- No semantic recall/precision or latency claim is made; complete DLP coverage is not claimed.
- Root owns the final `KernelPromptAssembler::with_semantic_privacy` integration and broad project acceptance; those are outside this evidence slice.
