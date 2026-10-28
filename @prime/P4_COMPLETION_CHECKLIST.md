# Phase 4 completion checklist — requirements retained

VERIFIED: this ledger retains each P4 owner's PLAN TODO and test matrix. An unchecked item means its complete requirement is not certified; it is not a claim that no implementation exists.

Current partial implementation and scoped evidence: [E50](evidence/phase3-20261027-local/README.md). All original requirements remain in scope. UI is deferred; non-UI demo transcripts remain required.

## polaris-worker

2026-10-26 §E42: native owner drops idle models after a validated 1ms..1day interval (default 45s); explicit eviction acknowledges only after destruction. Installed-model RED/GREEN and authenticated child explicit/idle eviction plus real reload pass. This does not pause heavy-build admission, preempt inference, impose RSS limits, prove RSS reduction or complete nano/OCR/pressure policy.

- [ ] Write JSON descriptors (prompt.md §6.3) for: mode dispatch, named-pipe server with token auth, health/memory reporting, eviction policy, ingest isolation limits.
- [ ] Replace the doc-only lib skeleton with `src/main.rs` + bin target `polaris-worker`.
- [ ] Reuse the pipe acceptor from `polaris-rpc` or define it in `polaris-proto`-level helper; avoid duplicating the pre-created-next-instance logic (open: which crate owns the shared pipe helper, since the worker does not depend on `polaris-rpc`).
- [ ] Authenticate the supervisor: one-time token via inherited handle; reject any other peer; no secrets on argv.
- [ ] `--mode s1`: load engine lazily on first `Decide`, batch requests, return probabilities; honour per-request deadline so kernel fail-open/closed policy is deterministic.
- [ ] Eviction: idle TTL, memory-pressure signal from the supervisor, `Evict { force }` before cargo/nvcc builds; unload really frees RSS (measure).
- [ ] `--mode ingest`: wrap each document in a limit set (max bytes, max pages, wall-clock timeout); child-of-worker or in-process catch only for task panics; crashes are expected and recovered by the supervisor.
- [ ] `--mode nano`: supervise the Needle engine (`--serve`) or Python package; keep disabled until spike S0-6.
- [ ] Heartbeat/health to the supervisor; exit code taxonomy (clean, auth failure, model load failure, OOM).
- [ ] Accept shutdown (graceful) and exit when parent pipe closes (no orphan workers).
- [ ] BLOCKED-ON-ACCESS (Hugging Face model download and the S0-4 ONNX export): real model inference; before that, run engine tests on tiny synthetic ONNX graphs.
- [ ] BLOCKED-ON-ACCESS (CUDA EP on GTX 1650): GPU path measured only on the dev machine; CPU path first.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | mode parsing; eviction policy table (idle, pressure, force); exit-code mapping |
| contract | `WorkerService` descriptor snapshot; `Health` reports mode and RSS |
| smoke | spawn each mode, `Health` OK within a deadline, `Shutdown` exits 0 (nano marked SKIPPED until S0-6) |
| security | wrong token refused; token absent from process command line; second client refused |
| fuzz | WSL2 nightly: ingest parsers on mutated docx/xlsx/pdf/pptx/html (crash must not escape the worker) |
| stress | crash loop: kill the worker 50 times under load, supervisor restarts within intensity limits, circuit breaker opens, DLQ receives work; memory pressure forces eviction |
| load | `Decide` batch throughput and latency with hdrhistogram; S1 batch sizes 1/8/32 (targets to measure; vendor CPU latency is about 167 ms on 48 cores, so the chatlog's under-15 ms is NOT a target) |
| spike | `polaris-spike-s1` bursts decisions and kills the worker mid-burst |
| fixtures | tiny synthetic ONNX models and labelled Decide cases; PII samples generated at test time |

## polaris-system-one-engine

E50: separate raw-byte PII span engine and model/runtime digests implemented; installed Decide inference checked. Live PII quality, full parity/calibration and CUDA gates remain open.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: model loading + ORT discovery, multi-task classify, entity extraction, parity gate, sidecar fallback.
- [ ] Spike S0-4 step 1: export Decide to ONNX fragments (encoder, heads) from the Python checkpoint; record exporter script, tool versions and output blake3 in `ModelManifest`.
- [ ] Spike S0-4 step 2: build the Python reference run (uv is missing on this machine; install it in P0) and produce `reference.jsonl` for the calibration fixtures.
- [ ] Implement ORT loading: `ORT_DYLIB_PATH`, DLL beside the exe (never System32), reject ORT < 1.23 with `OrtVersionTooOld`.
- [ ] Implement CPU path via `gliner2-rs` classification with probabilities; confirm the crate really accepts span checkpoints and our export (API to confirm at implementation).
- [ ] Implement `parity_check` and enforce top-1 >= 99% on the fixture set; below that, `EngineBackend` selects the sidecar.
- [ ] Measure fp32 / fp16 / int8-dynamic on CPU and CUDA EP (GTX 1650, sm_75): p50/p95 for batch 1/8/32 at 128/256/512 tokens; record in the manifest and in `PHASES_STATUS.md`.
- [ ] Implement the PII-NER path using the published ONNX of `gliner2-privacy-filter-PII-multi` (BLOCKED-ON-ACCESS: Hugging Face download reachability and licence check for both checkpoints; licences are UNVERIFIED).
- [ ] Implement `resident_bytes` accounting and `unload` that releases the ORT session and arena; test RSS returns near baseline.
- [ ] Implement the supervised Python sidecar (JSON frames; restart policy delegated to the kernel supervisor) as the fallback backend.
- [ ] Add batch-composition invariance checks (result for an item must not depend on its neighbours in the batch; padding mask correctness).
- [ ] Wire the worker-side gRPC adapter in `apps/polaris-worker` (not in this crate); engine exposes only plain Rust calls.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | label-to-prompt formatting; truncation flag set when input exceeds `max_seq_len`; manifest hash mismatch -> `ModelFileMissing`; ORT version gate |
| proptest | per-task probabilities within [0,1] and (single-label) sum to about 1; batch-composition invariance within tolerance; unicode/empty/very long text never panics |
| golden-parity | native vs Python `gliner2` on the calibration fixtures: top-1 >= 99% (gate), `max_abs_prob_diff` recorded; separate run per precision/EP |
| spike | S0-4: export feasibility, parity, CPU vs CUDA EP latency and RSS |
| load | sustained batches (1/8/32) for 10 min: throughput, p99, RSS slope |
| stress | load/unload x 100 loop (RSS leak check); ORT DLL missing; OOM at large batch returns `OutOfMemory`, not abort |
| fuzz | tokenizer + text preprocessing with arbitrary bytes-as-UTF-8 and lone surrogate edge cases (WSL2 nightly) |
| security | prompt-injection corpus: long padding to push the real question out of the window (truncation must keep the instruction side), confusable unicode in labels |
| fixtures | `tests/fixtures/s1/{decide,pii}/*.jsonl` with provenance + licence; hostile inputs generated at test time |

## polaris-system-one-client

E50: bounded cross-call batching, exact request/checkpoint/generation/incarnation cache and single-flight implemented with cancellation/race fixtures; authenticated real Decide worker checks pass; complete workload/formal gates remain open.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: request batcher, cache key + strict integrity, deadline/fail-mode resolution, named-pipe transport.
- [ ] Define `worker.proto` `DecideBatch` messages with `polaris-proto` owners; include `protocol_version` and `checkpoint_blake3` in replies.
- [ ] Implement the batcher: time-window + size coalescing, per-request response routing, bounded queue with `QueueFull` backpressure.
- [ ] Implement cache key construction and single-flight; strict-integrity blake3 check on hit for safety/PII/postflight schemas.
- [ ] Implement `Outcome` resolution so each `Consumer` maps to its `FailMode` exactly as the facade matrix states; one place, table-driven.
- [ ] Implement `NamedPipeTransport`: custom tonic connector, pre-created next pipe instance on the server side (worker), token auth metadata; document the DACL concern (default allows other local users).
- [ ] Add circuit-breaker cooperation: after N consecutive transport failures return `FailedOpen/Closed` immediately (no queueing) until the supervisor reports the worker healthy.
- [ ] Add cache stats + purge wired to `/cache stats|purge` via telemetry events.
- [ ] Add a non-stub in-process transport backed by recorded real engine answers (fixtures labelled synthetic until re-recorded) for kernel tests.
- [ ] Load-test coalescing under 50-subagent bursts (spike `polaris-spike-s1`); tune window and max batch from S0-4 numbers.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | policy resolution table for every `Consumer`; timeouts not cached; schema version in key invalidates old entries |
| proptest | every request receives exactly its own answer under random interleavings; cache key stable under input normalization, different under any schema/version/checkpoint change |
| loom | batcher state machine (enqueue/flush/shutdown) modelled without tokio: no lost or duplicated request |
| contract | `worker.proto` request/reply against a fake worker over a real named pipe; protocol_version mismatch rejected |
| load | cached decisions per second and p99; mixed hit/miss at 1k, 10k concurrent callers; record with hdrhistogram |
| scalability | 1 -> 32 frames each submitting watchdog/triage questions |
| stress | worker killed mid-batch: all in-flight requests resolve to the policy outcome within deadline; worker crash loop; queue saturation |
| spike | `polaris-spike-s1` burst: 50 concurrent subagent questions; compare batching on/off |
| security | cache-poison attempt via crafted colliding inputs on a strict schema (must miss); pipe connection without token rejected |

## polaris-security-pii

E50: reversible NER spans, deterministic degradation, sealed marker-map snapshots and kernel private CAS/recovered-key integration checked; actual PII companion inference, corpus/performance/fuzz acceptance remains open.

- [ ] Write JSON descriptors (prompt.md §6.3) for: mask, unmask-at-boundary, placeholder-injection defence, NER degradation, sealed export/import.
- [ ] Define placeholder format with a per-session random tag so attacker-authored text such as `<API_KEY_1>` cannot be unmasked into a secret (placeholder-injection defence).
- [ ] Implement `mask` using `DfaScanner` spans + entropy candidates; replace by single left-to-right pass over a pre-collected match list (not repeated `str::replace`, which the chatlog used and which is quadratic and order-dependent).
- [ ] Make masking stable and idempotent: the same value gets the same token within a session; already-masked text is unchanged.
- [ ] Implement `mask_with_ner`: batch via `DecisionEngine`, apply a timeout, mark `degraded = true` on failure; ensure the degraded path never skips the DFA/entropy layer (fail closed).
- [ ] Implement `unmask_for_sandbox` through a boundary witness type; it must refuse to unmask any text bound for a model or log.
- [ ] Implement `SanitizeReport` counts for UI messages ("sanitized N items").
- [ ] Implement sealed export/import via the `Sealer` trait for `/session resume`.
- [ ] Add maximum vault capacity and eviction policy (error, never silent drop).
- [ ] Windows: UTF-8 and UTF-16 inputs (PowerShell output), path-like identifiers with backslashes, and CRLF boundaries in span replacement.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | the chatlog example (IP, `sk-...` key, email) masked and unmasked correctly; same value -> same token; counts reported |
| proptest | `unmask(mask(x)) == x` for any text when unmasking is allowed; masked output contains none of the extracted values; masking is idempotent |
| fuzz | `fuzz_pii_masker` (arbitrary UTF-8 and bytes), `fuzz_pii_unmask_tokens` (tokens adversarially placed) |
| security | PII-egress capture test: fake secrets/PII generated at test time are absent from the serialized request sent to a wiremock Token Factory; placeholder-injection corpus; fail-closed when NER returns an error |
| load | mask 1 MB of log text < 50 ms with DFA+entropy only (target, to be measured) |
| stress | 100k distinct values hit the capacity limit with a typed error |
| fixtures | fake PII/keys generated at test time; NER outputs recorded as **synthetic** until a real model is available |

## polaris-security-rules

Versioned AST/DFA rule packs, bounded diff/command audits, explicit partial-language coverage and fail-closed canonical S1 arbitration implemented; flow-sensitive UAF/symlink and registry-backed supply-chain coverage remain open. Focused raw test and strict Clippy evidence is being captured.

- [ ] Write JSON descriptors (prompt.md §6.3) for: pack parsing/validation, AST-pattern evaluation on diffs, DFA-pattern evaluation, command audit, S1 arbitration with fail-closed timeout.
- [ ] Confirm what `ast-grep-core` provides for YAML rule configs (the `ast-grep-config` / `ast-grep-language` crates are not in `workspace.toml`); if core alone is insufficient, implement our own rule schema in `serde-saphyr` mapped to core patterns, and tell the toml owner.
- [ ] Define the pack schema and version it; write the first packs: Rust unsafe/race, C/C++ UAF and double free, CUDA (`__syncthreads()` in divergent branch, unsynchronised shared memory, OOB thread index), command injection (`std::process::Command`/`os.system` with tainted args), path traversal and symlink races, hard-coded credentials, typosquatting heuristics.
- [ ] Build the CWE table with explicit ids (for example 362, 78, 22, 59, 416, 415, 798, 787, 125); mark unmapped supply-chain heuristics as "no CWE assigned" rather than inventing one.
- [ ] Implement `audit_diff` restricted to changed hunks (not whole files) using code-intel spans.
- [ ] Implement `audit_command` using the automata scanner, shared with blast radius; return structured findings, not booleans.
- [ ] Implement `arbitrate`: S1 may downgrade a *disputed* finding only when confidence exceeds a configured bar; timeout or error keeps the finding (fail closed).
- [ ] Add a pack regression corpus: each rule has at least one true-positive and one true-negative snippet.
- [ ] Windows: rule paths use forward-slash normalised relative paths; `cmd.exe` / PowerShell dangerous patterns included alongside POSIX ones.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases / properties (budgets are targets) |
|---|---|
| unit | each rule: TP and TN snippet; CWE/severity present; pack with unknown CWE rejected |
| proptest | auditing an unchanged file yields no new findings; findings are within diff hunks and spans are in bounds |
| fuzz | `fuzz_rules_pack_yaml` (arbitrary YAML), `fuzz_rules_audit_command` (arbitrary command strings) |
| security | known-bad corpus (command injection, traversal, hard-coded key, CUDA race) all detected; known-benign corpus has zero findings (false-positive budget recorded, not assumed); S1 timeout never removes a finding |
| load | 1,000-line diff audited < 100 ms with all default packs (target) |
| scalability | pack count 10 / 100 / 500 rules vs audit latency |
| fixtures | per-rule snippets with provenance; malicious-diff corpora generated at test time |

## polaris-system-one-schemas

E50: bounded exact-identity binary calibration metrics and xtask measured-candidate reports implemented; production thresholds remain uncalibrated pending real labelled held-out evidence.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: schema TOML parsing, threshold validation, threshold application, calibration staleness check, builtin coverage.
- [ ] Define the TOML grammar and a `schemas/*.toml` directory embedded with `include_str!`; one file per schema id.
- [ ] Implement `Prob`-validated thresholds with ordering invariants and exhaustive error variants.
- [ ] Implement `apply_threshold` for Choice, Score (expected vs argmax rule per schema) and Noul.
- [ ] Ship all builtin schemas as `uncalibrated` with conservative defaults; safety-relevant ones set `integrity = "strict"` and `on_timeout = "closed"`.
- [ ] Implement `SchemaCatalog` and unit-test that every `Consumer` has exactly one default fail policy.
- [ ] Implement the staleness check (`calibrated` requires matching `fixture_blake3` / `checkpoint_blake3`); xtask supplies the hashes.
- [ ] Add the calibration runner as an `xtask` subcommand contract (inputs: fixtures + engine output JSONL; outputs: TOML patch) without embedding inference here.
- [ ] Build the first labelled fixture sets for effort (50+ tasks), safety.command (benign vs destructive), tool relevance (MCP tool lists), chunk relevance; record provenance and licence.
- [ ] Re-run calibration after S0-4 once a trustworthy engine exists (BLOCKED-ON-ACCESS: S0-4 export result, HF checkpoint access).

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | duplicate ids rejected; malformed TOML gives a typed error; every builtin loads and validates |
| proptest | threshold monotonicity: raising `accept` never increases the number of Accept verdicts; verdicts total over any valid probability vector |
| kani | `apply_threshold` on bounded vectors: no panic, no out-of-range index, no NaN reaching a comparison |
| golden | insta snapshot of the loaded builtin set (guards accidental threshold edits) |
| contract | every `Consumer` variant has a schema with a fail policy; safety/PII/postflight/arbiter are `closed` + `strict` |
| fixtures | labelled calibration sets (JSONL, provenance + licence); injection negatives generated at test time |
| security | schema TOML from a repo (`.polaris/`) cannot lower a built-in `closed` policy to `open` (override only tightens) |

## polaris-system-one-router

E50: combined intent/effort/parallel/vision request, Medium constrained fallback, bounded input and hard directives checked; labelled ranking/calibration, environments and complete workload matrices remain open.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: `route`, `prune_tools`, `rank_files`, `route_sandbox`, `rank_env_images`.
- [ ] Implement deterministic fallbacks first (lexical scoring, effort default, conservative sandbox tier) so fail-open behaviour exists before S1 does.
- [ ] Implement `route`: Choice over {trivial-direct, S2} + Score effort 1-5 + Noul parallelizable/needs-vision; unify into one multi-task request.
- [ ] Implement `prune_tools` as two-stage: lexical prefilter to a bounded candidate set, then S1 Score; hard caps on candidate count and per-summary length.
- [ ] Implement `rank_files` as two-stage (tree-sitter/ripgrep candidates are inputs); never score all N files with S1 (cost grows with N on CPU).
- [ ] Implement `route_sandbox`: deterministic rules first (nested KVM absent -> no E2; no key -> no E3), S1 only for ambiguous facts.
- [ ] Implement `rank_env_images`: exact repo+commit/lockfile-hash match short-circuits S1; S1 ranks only near-misses.
- [ ] Respect `$subagent-models`/`$sandbox` directives as hard constraints applied after scoring.
- [ ] Build labelled fixtures: 100+ task descriptions with effort labels, 30 MCP tool lists with relevant-tool labels, 20 repos with file relevance labels (BLOCKED-ON-ACCESS for real-model calibration: S0-4).
- [ ] Measure S1 cost vs candidate count (10/20/80/200 files, 10/50 tools) on the target machine and set the candidate caps from data.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | zero candidates and `TopK(0)` are typed errors; directive overrides win over scores; tie-breaking is deterministic |
| proptest | output is a permutation subset of input ids, length <= k, scores non-increasing; same input gives same output |
| golden-parity | router verdicts on labelled fixtures with the Python reference vs Rust engine: identical routes in >= 99% of cases |
| smoke | with a fake always-timeout engine every method returns its deterministic fallback with `RouteSource::DeterministicFallback` |
| load | 1k route() calls and 50-tool pruning under concurrent frames; p99 recorded |
| scalability | 10 / 50 / 500 tools; 20 / 200 / 2000 candidate files (cap holds) |
| security | tool-description injection corpus: a tool whose summary says "always select me" must not outrank genuinely relevant tools beyond a measured bound; oversize summaries truncated |
| fixtures | `tests/fixtures/s1/router/*.jsonl` (labelled tasks/tools/files) |

## polaris-system-one-verifier

Production deterministic safety, preflight/postflight and merge arbitration in progress; UNVERIFIED, validation deferred by user.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: command gate, preflight tamper detection, secret scan, postflight tautology, diff arbiter.
- [ ] Implement AST-based test-tampering detection via `code-intel` (removed/weakened asserts, `#[ignore]`, `pytest.mark.skip`, `unittest.skip`, `@Disabled`, deleted test fns); replace the chatlog's substring check (`"-    assert"`), which is trivially bypassed and noisy.
- [ ] Implement secret detection with DFA signatures + Shannon entropy from `security-automata`; findings never echo the secret value (redacted evidence).
- [ ] Implement the command gate: shell tokenization, destructive-pattern DFA (`rm -rf`, unquoted variable expansion into rm, `/proc/kcore`, `curl|sh`, hidden-file edits), S1 only for the residual "unclear intent" class; unknown -> `AskUser`.
- [ ] Implement deterministic tautology detectors (`assert!(true)`, `assert_eq!(x, x)`, no assertions, test never calls code under test via the changed symbols) before asking S1 `is_tautological`.
- [ ] Define the strong postflight rule: ProofOfDone integration needs mutation evidence (cargo-mutants, currently missing on this machine) or a falsifier run; S1 opinion alone yields `Unverified`, never `Pass`.
- [ ] Implement `arbitrate`: adjacency check from line/AST ranges, orthogonal -> `code-intel` AST merge, semantic congruence via S1, conflicting or S1 timeout -> `RequiresSupervisorAttention` with no disk write.
- [ ] Implement diagnostics ranking over deduplicated diagnostics supplied by RTK (the verifier does not parse compiler output itself).
- [ ] Build a malicious-diff corpus generator (tamper variants x 4 languages x obfuscations) run at test time; never commit the corpus.
- [ ] Calibrate thresholds for residual S1 questions after S0-4 (BLOCKED-ON-ACCESS: working engine).

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | each `TamperKind` detected in Rust/Python/TS/C++ diffs; benign refactors of tests not flagged; command gate on a 200-command labelled list |
| proptest | gate decisions invariant under whitespace/quoting-equivalent rewrites of the same command; a diff that only adds assertions never yields `TestTampering` |
| fuzz | diff parser and command tokenizer on arbitrary bytes (WSL2 nightly) |
| golden-parity | residual S1 questions: Python reference vs Rust engine agreement >= 99% on the fixture set |
| smoke | end-to-end: tampering diff blocked before sandbox; green run with `assert!(true)` test yields `Unverified`/`Fail` |
| stress | 10k-line diffs, 1k-file diffs: bounded time, no panic; S1 timeout storms degrade to deterministic path |
| security | obfuscated tampering corpus (assert moved into comment, `cfg(any())`, macro-wrapped skip); secret-in-diff corpus built at test time; evidence redaction check |
| fixtures | labelled diffs and commands with provenance; fake keys generated at test time |

## polaris-system-one-watchdog

Production bounded loop detection, advisory role/context guards and semantic drift inspection in progress; UNVERIFIED, validation deferred by user.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: loop detection, role-table check, goal-drift question, fatigue measure, `inspect` ordering.
- [ ] Implement deterministic checks first and in this order: role table (a table lookup), exact-repeat fingerprint window, near-duplicate via normalized token shingles (Jaccard), token-pressure ratio.
- [ ] Implement the goal-drift Noul question ("does this step advance the root goal") via S1 with a calibrated band; drift in the Abstain band does nothing.
- [ ] Replace the chatlog's "semantic vectors" with shingle-based similarity now; add Needle-3 embeddings as an optional refinement after S0-6 (GLiNER2 is not an embedder).
- [ ] Replace "reasoning entropy" (needs logprobs, not exposed) with measurable repetition rate and token pressure; record both in the verdict.
- [ ] Implement disproven-hypothesis marking: a rejected patch variant fingerprint is added to the frame's negative-constraint list and returned in `Loop`.
- [ ] Implement the fail-open path: S1 timeout yields `Unchecked` while deterministic verdicts (loop, role, pressure) still apply.
- [ ] Make the 5-10 step thresholds (window size, shingle size, similarity cutoff, pressure ratio) config data in `schemas`, not constants.
- [ ] Build trajectory fixtures: recorded or hand-built looping and non-looping step sequences (synthetic, labelled as such).
- [ ] Measure the per-step overhead (deterministic path vs with S1) on the target machine.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit | role table denies out-of-role tool; window of N identical steps -> `Loop`; pressure above ratio -> `ContextFatigue` |
| proptest | loop detector never flags strictly distinct steps (distinct fingerprints and low shingle overlap); permuting unrelated frames does not change a frame's verdict |
| golden | labelled trajectories: loop precision/recall recorded (deterministic path needs no model) |
| smoke | S1 down: deterministic verdicts still produced, drift reported as `Unchecked` |
| load | 1k frames x 15 steps/s of fingerprint checks: bounded time per call |
| scalability | window sizes 5/50/500; 1 -> 32 frames |
| security | injected "ignore your role" text in tool output must not change the role table; oversize thoughts bounded before hashing |
| fixtures | `tests/fixtures/s1/watchdog/*.jsonl` trajectories (synthetic, labelled) |

## polaris-system-one

E50: explicit per-consumer failure policy, bounded admission, circuit and host epoch fencing checked; whole P4 parity/calibration/MVP acceptance remains open.

- [ ] Write JSON descriptors (prompt.md section 6.3) for: kernel-side re-export set (no `engine`/`nano`), matrix-vs-schemas consistency check.
- [ ] Keep the kernel-side-only dependency set (no `engine`/`nano` edges) and re-check `cargo tree -p polaris-kernel -i ort` is empty once real deps land.
- [ ] Add a doc-test or integration test asserting every `Consumer` variant has a row in the matrix and a `fail` policy in the builtin schemas.
- [ ] Publish the calibration procedure and the memory budget as crate-level `//!` docs and keep them in sync with `schemas` and `engine` after S0-4.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| unit / doc | doc-tests compile the typed API; matrix consistency test (11 rows vs `Consumer` enum) |
| contract | facade re-exports exactly the seven children (`check-layers` + `check-members`) |
| spike | S0-4 fills the latency and memory columns above |

## polaris-spike-s1

E50: real-engine burst dispatch, bounded owned tasks, sanitized per-request outcomes and observed latency quantiles implemented; real crash/load matrices remain open.

- [ ] Write JSON descriptors (prompt.md §6.3) for: decision burst driver, fault plan, fail-open/closed assertions, stampede check.
- [ ] Replace the doc-only lib skeleton with `tests/decisions.rs`.
- [ ] Build a synthetic ONNX model fixture so the burst can run before the real Decide export exists (S0-4); label results SYNTHETIC-MODEL.
- [ ] Scenario A: 500 decisions in 1 s, assert batching reduces model invocations and all callers get answers or typed timeouts.
- [ ] Scenario B: kill the worker mid-burst; assert supervisor restart within intensity limits, circuit breaker behaviour, and per-consumer fail-open/closed outcomes.
- [ ] Scenario C: stampede of 100 identical inputs; the model runs once and the rest hit the cache (xxh3 key).
- [ ] Scenario D: timeout injection per consumer; SafetyGate/PiiMask return the DFA path, others proceed.
- [ ] Re-run Scenarios A and B with the real model after S0-4; record latency on CPU and on the GTX 1650 with conditions.
- [ ] BLOCKED-ON-ACCESS (S0-4 ONNX export + Hugging Face download): the real-model run.

Required matrix (retained verbatim; no full-matrix PASS claim):
| Category | Cases |
|---|---|
| spike | scenarios A to D |
| stress | worker crash loop (kill every 2 s for 2 min); memory pressure forcing eviction mid-burst |
| security | fail-closed consumers never return "allow" on timeout |
| proptest | for any fault timing, every request resolves exactly once (answer, typed timeout, or typed error) |

## Named phase gates

- [ ] P4-X-1 — pinned export outputs and Python goldens, recorded hashes and current Rust parity.
- [ ] P4-X-2 — real labelled fixture precision/recall and held-out production threshold evidence.
- [ ] P4-X-3 — complete non-UI MVP transcript; UI/video work remains explicitly deferred.
- [ ] P4 exit — S1 parity and measured latency, all consumer failure modes, complete MVP evidence.
