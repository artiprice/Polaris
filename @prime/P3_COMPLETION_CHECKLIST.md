# Phase 3 completion checklist — evidence audit

## E50 current ledger

VERIFIED scoped implementation/evidence checkpoint; Phase 3 and Phase 4 remain IN PROGRESS.
UI is deferred and P2 remains paused except the sandbox image repair required for P3 tests.

Production additions include: same-owner pinned local llama.cpp counting/chat and an owned-process
loader; exact test-receipt review/frame/session lookup; human-answer reference access and authority
expiry checks; compatible Contree environment resolution; math expression guards; S1 batching,
checkpoint/generation/incarnation-bound exact cache and bounded single-flight; separate raw-byte
PII span engine/RPC/client/vault integration; exact environment ranking and bounded unique tool/file
candidates; owned S1 burst dispatch with answer-integrity validation; encrypted private CAS marker snapshots with explicit
host seal-key recovery; semantic prompt masking before tokenization; combined task routing; shared
consumer fail-open/closed policy with bounded admission and worker epoch fencing; binary calibration
metrics and a measured-candidate xtask; guarded pinned export/golden tooling. These additions do not
certify real-model quality, production thresholds, owner matrices or phase exit.

The selected 27-package native run recorded 620 passed / 2 failed / 16 ignored before the final
weighted-admission fix and single-flight follow-up. Both failures were refund acceptance cases.
The subsequent complete limiter/spike run recorded 16 passed / 3 failed / 0 ignored before new
independent refund regressions. The current refund suite records 7 passed / 7 failed: safe non-tail
refund capacity remains unrecovered, and three preserved fixtures have conflicting contracts.
All original refund assertions remain. Whole-phase tests are RED; no selected-green claim replaces it.
Weighted burst tests first failed twice, then passed twice after admission began checking the full
new weighted debt rather than the old unit-cost threshold.

Actual retained runtime observations include the local Qwen read-only tool/continuation transcript,
exact count/chat prompt-usage parity, owned loader shutdown, and the real OpenShell test pipeline
with fixed synthetic S2/S1 fixtures. The combined actual model/edit/test attempt remains RED on
invalid model arguments, denied before mutation. The rebuilt P3 image is immutable
sha256:b7a2dc2a53a71c5695242dd890d2705aa5d0e3b45dafcd885ae6a3ea2bbba4b2;
the paused P2 image/tag was not replaced. Direct Decide, actual authenticated Windows worker,
idle eviction/reload and supervised-worker inference pass using rehashed existing local artifacts.
The original four real-model cases do not establish 200-row parity or labelled calibration.
All nine companion PII artifacts are now pinned by exact revision/size/SHA256. An interrupted
encoder partial failed whole-file verification, was retained separately and never used; a fresh
full fetch passed every hash. Actual PII CPU inference passes a planted raw-UTF8 span fixture.
Actual authenticated PII/Decide/PII reload, idle eviction and warm/post-eviction 50-request bursts
pass (3 cases). Each repeated-input burst answered 50/50 at peak 8 owned tasks; this exercises
cache/single-flight, not 50 distinct native model invocations or the full throughput matrix.
Two exact owned worker kills exposed a fatal-versus-transient retirement bug. After correction,
actual replacement/authenticated readiness/final retirement pass; the supervisor suite records
50 passed. These two kills do not complete the full crash-loop/intensity/workload matrix.

A later scoped Linux run recorded 164 passed / 0 failed / 2 ignored for kernel prompt/model/privacy,
client/PII/facade/router/schemas/secrets/proto, before the newest rules/router/watchdog changes.
Workspace all-target compilation previously passed after propagating additive health fields to RPC;
new security-rule/watchdog integration is accumulating and final-tree compilation is not certified.
Member/layer/witness, generated docs and generator preservation checks pass. Five selected production
libraries and four selected kernel test targets pass the retained unsafe/panic/cast/index/arithmetic
lint subset. This does not certify strict whole-workspace lint or a fresh final Linux/full formal run.

Still open: correct bounded out-of-order refund accounting and conflicting late-refund assumptions;
actual combined local edit/test/proof acceptance with calibrated S1; Token Factory and Contree keyed
acceptance (BLOCKED-ON-ACCESS, user confirmed unavailable); full PII corpus/quality/resource results;
full S1 parity, held-out calibration and MVP recording; watchdog/context/role/candidate integrations
and complete owner load, scalability, stress, fuzz, Miri, Kani, security and release matrices.
Export inputs are present, but the 10 GiB available-RAM precondition was not satisfied in the last
probe; no RAM guard or assertion was weakened. Snapshot key provisioning/rotation and rollback
protection are host lifecycle obligations; the implemented key recovery never silently replaces keys.

[Retained E50 evidence](evidence/phase3-20261027-local/README.md).

## E49 current ledger

VERIFIED scoped E49 checkpoint. Phase 3 remains IN PROGRESS; P2 is paused and UI deferred.

The model-facing kernel runner now dispatches real frame-bound write/edit/patch, file revisions,
durable ask_user and explicitly host-approved sandbox test requests. Exact sealed step/frame ancestry,
shared bounded cancellation registries, durable subtree receipts, independent runner cancellation,
shutdown ownership and complete-set admission protect these operations. SDK event schema 6 and
proto tags 28–30 preserve older records and the existing compact-event size bound. Host CancelFrame
uses a separate L4 right; the model does not receive host approval authority.

The production frame verifier validates exact recorded tool-result envelopes, mutation receipts,
approvals and the current source manifest. Completion requires the latest successful ordinary and
independent test verdict bound to the exact session/frame/review/source tree. New schema-2 proof
reports supply those bindings; older unbound proof reports require a fresh test run. These checks
do not constitute live S1 calibration or a full live model/edit/test/proof acceptance transcript.

Selected native regression: 655 passed, 0 failed, 3 ignored. Linux SDK/proto/capability/frames/
Code Intel/verifier/spike domain: 238 passed, 0 failed, 0 ignored. Kernel Linux is not covered.
Seven production libraries pass the recorded unsafe/panic/cast/index/arithmetic lint subset;
focused new kernel test targets pass the same subset. Workspace all-target compilation and
member/layer/witness/generated-doc/generator-preservation gates pass. Code Intel's 38 production
arithmetic violations reported in E48 are fixed; existing example/test and dependency lint debt
is not thereby closed. Exact commands, failed attempts and raw outputs remain below.

Seven limiter spike cases now exercise actual limiter instances: staged priority, shedding,
changing Retry-After floors, reservation/overrun, hostile typed durations, owned waiter cancellation
and real-clock admissions. They do not prove concurrent priority scheduling, fixed-floor anti-herd
spread, the 100-repetition A–D matrix, 10-minute load, fairness or formal/stress acceptance.
New polaris-ratelimit reservation_refund acceptance tests remain RED (four failures, three passes
in the retained investigation). They are excluded from the selected 655-test command. This is
not a green whole-workspace test claim. No refund assertion has been weakened.

The pinned local llama.cpp/Qwen runtime has separately produced three actual structured-tool-call
probes with exact count/chat prompt-token parity. See the tracked runtime provenance manifest.
This observation is not yet an integrated production provider/loader or M3 transcript. Downloaded
binary/weight artifacts are ignored and excluded from first-party inventory. Token Factory and
Contree keyed acceptance remains BLOCKED-ON-ACCESS. Full owner matrices, calibrated S1, real
M3/M4, new Miri/Kani, strict workspace lint and dependency audit/vet gates remain open.

Behavioral REDs include duplicate cross-service answers and stale/current-proof handling; missing
API compilation failures are distinguished in retained logs. Some additional tests followed
implementation, so no universal test-first claim is made. No source lint suppressions were added.

[Evidence and exact command scope](evidence/phase3-20261027-dispatch/README.md).

The original audit below is historical; this current ledger supersedes its source-absence findings. All original owner requirements and test matrices remain in scope.

## Historical initial audit

Snapshot: 2026-10-27. Scope: all P3 crate tasks and P3-X-1..5; M3/M4 prerequisites; UI is deferred, while P3-X-5's non-UI protocol remains included. This audit checks current source/evidence references and local model availability; it does not certify phase acceptance or run model inference.

Status terms: **EVIDENCE-PROVED (SCOPED)** means a cited run verifies that bounded behavior only; **INCOMPLETE** means an acceptance condition or listed owner requirement remains open; **UNVERIFIED** means the necessary evidence was not found/produced; **BLOCKED-ON-ACCESS** means an external credential/service is required and unavailable. A source file, descriptor, estimate, historical status label, or synthetic fixture alone is not proof.

## M3/M4 entry and exit requirements

| Requirement | Current evidence | Finding |
|---|---|---|
| P3 entry requires P2 exit and P0-ENV-6 local endpoint readiness ([PHASES.md §P3](PHASES.md#p3--system-two-walking-skeleton--token-factory-contree); milestones in [PHASES.md §2.1](PHASES.md#21-milestones)). | P2 is user-paused; [P2_LOGIC_STATUS.md](P2_LOGIC_STATUS.md) retains open P2 scope. Historical P0-ENV-6 is `PASS (endpoint)` in [PHASES_STATUS.md §E10](PHASES_STATUS.md#e10--environment-verification-wsl2-openshell-linux-leg-local-llm-2026-10-20-2205), but tool calls appeared in only 1/4 historical trials; the two E10 transcript runs had none. This session: `ollama list` → `nemotron-mini:latest ed76ab18784f 2.7 GB`; GET `/api/version` → `{"version":"0.40.11"}`; GET `/v1/models` → one model, `nemotron-mini:latest`. | Endpoint/model availability is verified; P2 exit and dependable tool-capable model are not. M3 entry prerequisite not fully demonstrated. |
| M3 / P3-X-1: actual local loop transcript `plan → edit → sandbox test → proof-of-done`. | [E47](evidence/continuation-20261027-e47/README.md) documents production prompt assembly and frame execution through real adapters, but the HTTP server's token IDs/count and verifier fingerprint are synthetic. [PHASES_STATUS.md](PHASES_STATUS.md) says live tokenizer/provider/edit/test/proof remains open. | **INCOMPLETE.** No actual model-generated plan, authorized edit, sandbox test, proof transcript. This audit intentionally issued no inference requests. |
| Exact full-chat token counting for the same wire request as chat dispatch. | [E47](evidence/continuation-20261027-e47/README.md) and [provider `VllmChatClient`](../crates/polaris-nebius-cloud/crates/token-factory/src/client.rs) use same-owner chat/tokenizer and `/tokenize`; fixture validates shared projection. Installed local provider is Ollama 0.40.11. `llama-server`, `llama-cli`, `vllm`, and `uv` were not found by `Get-Command`. No matching Hugging Face tokenizer cache was observed; listed cache entries are unrelated (Qwen2.5 GGUF, Needle, GPT-2, etc.). | **UNVERIFIED / local readiness gap.** Ollama's OpenAI-compatible endpoint was found, but this runtime/code path has no verified exact chat-tokenizer contract. No byte/text heuristic may substitute. vLLM documents a `/tokenize` API; no local vLLM binary was found. See [vLLM API docs](https://docs.vllm.ai/en/v0.6.5/serving/openai_compatible_server.html). |
| M4 / P3-X-2: same loop on Nebius Nemotron 3, effort tiers, transcript and usage. | Workspace decision D9 says no API keys; user says credentials are unavailable. P3 status marks P3-X-2 BLOCKED-ON-ACCESS. | **BLOCKED-ON-ACCESS.** Do not count missing credentials out of scope. |
| M4 / P3-X-3: Contree sidecar plus E3 SWE-bench run and transcript. | [PHASES.md per-PC setup](PHASES.md#01-per-pc-setup) says `contree-mcp` is not installed and needs access; [PLAN §13 sandboxes](PLAN.md#polaris-nebius-cloud-sandboxes--token-factory-sandboxes-contree-through-the-contree-mcp-sidecar-run-sync-images-directory_state_id-branching-swe-env-resolver) is still a doc-only adapter. User says credentials unavailable. | **BLOCKED-ON-ACCESS; INCOMPLETE.** No adapter or transcript. |
| M4 / P3-X-4: re-record SSE fixtures live; provider contract suite against real service. | Existing SSE tests/fixtures are synthetic; PHASES status marks live recording BLOCKED-ON-ACCESS. | **BLOCKED-ON-ACCESS.** Synthetic fixtures remain useful local evidence but do not meet this criterion. |
| P3-X-5 non-UI compatibility: durable `ask_user`, atomic answer, requester continuation, sibling progress, per-frame cancel, `None`/required validation, stale/duplicate/race/restart evidence. | [UI_PLAN §3.4](UI_PLAN.md#34-human-askanswer-protocol-and-popup-p3-not-an-m1-dependency) explicitly says pure questionnaire state only; no human-request SDK/kernel/frame backend or durable transitions. `CommandBody` has no answer-request command; `CancelFrame` exists in proto/SDK but targeted kernel dispatch does not. A descriptor exists at `specs/descriptors/human_request_backend.json`, which is not an implementation. | **INCOMPLETE.** This backend/protocol stays in denominator despite UI deferral. P3-X-5 also depends on P2 exit. |

## P3 named tasks

| Task | Required acceptance | Evidence checked | Finding |
|---|---|---|---|
| P3-X-1 | Local real-server plan/edit/sandbox-test/proof transcript. | E45–E47 scoped synthetic-provider / real-driver evidence; current Ollama metadata above; no transcript found in cited evidence. | **INCOMPLETE; actual tokenizer route UNVERIFIED.** |
| P3-X-2 | Nemotron 3 effort-tier loop, transcript + usage. | PHASES_STATUS lists missing key. | **BLOCKED-ON-ACCESS.** |
| P3-X-3 | Contree sidecar and E3 SWE-bench transcript. | Adapter skeleton; Contree absent. | **BLOCKED-ON-ACCESS and implementation incomplete.** |
| P3-X-4 | Live SSE re-recording and real-service contract suite. | Synthetic fixtures only; key absent. | **BLOCKED-ON-ACCESS.** |
| P3-X-5 | Backend human request protocol and targeted frame cancellation, per above. | Only fixture/descriptors and protocol enums; no service/dispatch. | **INCOMPLETE; not UI-deferred.** |

## P3 owner-crate audit

The `PLAN.md` links identify the full owner specifications and their TODO/test matrices. The concise “open requirements” below preserve the unresolved TODOs; checked boxes and status percentages in planning files are not treated as proof. Evidence references are the latest scoped records cited by `PHASES_STATUS.md` / `PHASES_STATUS_PERCENTAGE.md`, rather than a fresh acceptance run.

| Owner / full specification | Evidence-backed scope | Open requirements (source TODO references) |
|---|---|---|
| `polaris-ratelimit` — [PLAN §13](PLAN.md#polaris-ratelimit--gcra-rate-limiting-rpm-plus-token-weighted-tpm-priority-shedding-retry-after-15-minute-window-tracking) | Current percentage status says RPM/TPM/GCRA reservation/reconciliation source exists; complete behavioral/load verification missing. No current P3 limiter evidence record located. | **INCOMPLETE/UNVERIFIED:** descriptors; governor-vs-GCRA refund criterion bench; fake `Clock`; lock-free RPM+TPM GCRA; RAII permit refund/overshoot; configurable priority shedding; `Retry-After`/`x-ratelimit-*` penalty; cancellation-safe acquire; 15-minute window and `/quota`; jittered backoff; property/load matrix. TODO lines 1401–1410; tests lines 46–60. Header semantics remain **BLOCKED-ON-ACCESS/UNVERIFIED** under keyed S0-5. |
| `polaris-security-capability` — [PLAN §13](PLAN.md#polaris-security-capability--biscuit-capability-tokens-session-authority-per-frame-attenuation-authorization) | E45 scoped tests verify workspace scope, frame-root/token binding, monotone attenuation and real per-resource kernel read authorization. | **INCOMPLETE:** mutation issuer/fact construction and hybrid-verifier integration, global revocation and security audit; close dependency-audit debt. E45 expressly bounds its evidence to read-only. See PLAN TODO line 2056 and PHASES_STATUS P3 owner row. |
| `polaris-security-secrets` — [PLAN §13](PLAN.md#polaris-security-secrets--secret-vault-keyring-storage-secrecyzeroize-handling-chacha20-poly1305-ram-sealing) | E32 reports bounded zeroizing AEAD vault, SDK secret source, identifier/storage bounds and isolated real Windows keyring store/load/delete; synthetic keyring roundtrip/contracts are retained. | **INCOMPLETE/UNVERIFIED:** descriptor coverage; Linux credential backend; full keyring/env composition; `VirtualLock` decision/measurement or explicit no-lock policy; canary/log/heap hygiene; fuzz/formal and full integration. PLAN TODO lines 2195–2203. Keyring credential availability remains unavailable per user. |
| `polaris-nebius-cloud-token-factory` — [PLAN §13](PLAN.md#polaris-nebius-cloud-token-factory--openai-compatible-token-factory-client-sse-chat-streaming-tool-calls-embeddings-model-list-usage-accounting) | E47 scoped loopback fixtures verify same chat projection and bounded vLLM `/tokenize` response parsing; existing SSE/provider source and synthetic contracts are documented. | **INCOMPLETE/UNVERIFIED:** remaining descriptors and SSE ADR; live local full loop; keyed Token Factory SSE/usage/model/rate-limit/reasoning probe and live fixture re-recording; TLS app-level `ring` gate. Open P3-X-2/4 remain access-blocked. PLAN TODO lines 4645–4655; test matrix lines 49–61. Do not transfer vLLM `/tokenize` support to Ollama/Nebius without a live deployment check. |
| `polaris-nebius-cloud-sandboxes` — [PLAN §13](PLAN.md#polaris-nebius-cloud-sandboxes--token-factory-sandboxes-contree-through-the-contree-mcp-sidecar-run-sync-images-directory_state_id-branching-swe-env-resolver) | No production adapter; PHASES_STATUS identifies documentation-only skeleton. | **INCOMPLETE + BLOCKED-ON-ACCESS:** sidecar setup/auth; pinned inventory, loopback/port protection, `SandboxBackend` command/read/sync/long-operation mapping and cancellation, branch budget/StateId, SWE resolver, secret-safe registry auth, fake MCP suite then live suite, Windows child supervision and stale-process detection. PLAN TODO lines 4734–4744; S0-13 is access-blocked. |
| `polaris-nebius-cloud` facade — [PLAN §13](PLAN.md#polaris-nebius-cloud--facade-over-the-nebius-integrations-token-factory-sandboxes-data-lab-ai-cloud) | E41 documents facade exports for implemented AI Cloud surface; this does not prove P3 provider composition. | **INCOMPLETE:** compile doctest re-exporting all four children; requirement mapping with evidence ids; per-child access inventory; fallback story. PLAN TODO lines 4602–4605. |
| `polaris-models` — [PLAN §13](PLAN.md#polaris-models--model-registry-effort-tiers-subagent-models-mapping-and-local-model-lifecycle) | E42 verifies 20 catalog/lease/single-flight/cancel/drain/idle/reload/pressure contracts on Windows/Linux using test-only loaders. | **INCOMPLETE:** production loader ownership and composition, descriptor coverage, live catalog provenance, effort/subagent mapping, custom local endpoint config, cascade plan, price/list hooks; owner status says stress/formal acceptance open. PLAN TODO lines 3643–3652. |
| `polaris-tools-math` — [PLAN §13](PLAN.md#polaris-tools-math--deterministic-math-tools-exact-decimal-calculator-interpolation-statistics-gpu-occupancy-and-bandwidth) | Existing source has focused math contracts; current status says 25 contracts cover expression/interpolation/statistics/sourced GPU specs, with full audit open. | **INCOMPLETE/UNVERIFIED:** exact decimal evaluator choice and overflow/limit coverage; rational-fraction scope; finite interpolation conversion and sorted knots; percentile definition/NaN/empty; cited GPU data provenance; occupancy and unit-correct bandwidth; schema snapshots and full fuzz/performance gates. PLAN TODO lines 3827–3835. |
| `polaris-runtime-frames` — [PLAN §13](PLAN.md#polaris-runtime-frames--agents-as-context-frames-over-one-s2-client-and-one-s1-engine) | E46 verifies bounded durable frame journaling over actual kernel; E47 verifies sealed root/role/model/effort/token binding in scoped synthetic HTTP composition. | **INCOMPLETE:** live S2+tool+verifier step and complete plan/edit/test/proof transcript; production loader/limiter/frame assembly and formal acceptance. P3-X-5 needs durable human service and targeted cancellation in this owner. PLAN TODO lines 3134–3141 (some scope later covered by E45–E47; live gates remain). |
| `polaris-runtime-context` — [PLAN §13](PLAN.md#polaris-runtime-context--lossless-context-virtualization-stable-prefix-scratchpad-ref-folding) | E47 verifies shared tokenizer-independent window validation, masking before provider count, complete-group retention, bounded verified archive storage, exact count via fixture and CAS Read-gated archive visibility. | **INCOMPLETE:** full context lifecycle/live tokenizer, full prefix stability/expand/scratchpad/ref-resolver matrix and cross-platform properties. No live model-backed count is proved. PLAN TODO lines 3209–3218; E47 states live tokenizer remains open. |
| `polaris-kernel-operators` — [PLAN §13](PLAN.md#polaris-kernel-operators--functional-operator-pipeline-pipe-fork_branch-fold-assert_guard-and-the-kernel-side-operators) | E43 verifies real limiter admission/permit/usage reconciliation in scoped RateLimitOp tests and compile-fail contract. | **INCOMPLETE:** complete production operator pipeline and generic combinator/fail policy/PII/S1/directive wiring; drop-at-await cancellation matrix. PLAN TODO lines 2937–2944. `ReflectOp` is a P7 item, retained in its source section but not a P3 exit criterion. |
| `polaris-kernel-verifier` — [PLAN §13](PLAN.md#polaris-kernel-verifier--hybridmutationverifier-biscuit-datalog-authorization-ast-metric-calculus-s1-semantic-congruence) | Scoped authority/terminal-witness contracts exist; PHASES_STATUS states full hybrid mutation acceptance is open and incomplete unsafe-analysis coverage conservatively asks for human approval. | **INCOMPLETE:** actual token fact/Clock construction; Biscuit layer; code-intel threshold layer; S1 preflight/congruence and timeout policy; private `ApprovedMutation` witness wired to durable event append; structured corrective rejections; calibration and Windows canonical resource path checks. PLAN TODO lines 3024–3032. |
| `polaris-spike-ratelimit` — [PLAN §13](PLAN.md#polaris-spike-ratelimit--burst-tests-for-the-rate-limiter-50-sub-agents-at-once-429-storms-retry-after-floods) | PHASES_STATUS labels the crate NOT STARTED/documentation-only. | **INCOMPLETE:** no descriptors or workload; build A-D deterministic/load scenarios (50-priority burst, 429 storm/backoff, TPM overshoot/reconcile, hostile header flood), collect p50/p99/shedding/leaked tasks, freeze measured criteria; live header reconciliation is keyed. PLAN TODO lines 7775–7783. |

## Owner test-matrix categories

These are the full matrix categories named in each P3 owner section of `PLAN.md` §13. Numeric values there are targets, not evidence. A cited focused suite can satisfy only the cases it actually exercises; no category is promoted to phase acceptance from source presence or percentages.

| Owner | Required matrix categories | Evidence disposition |
|---|---|---|
| `polaris-ratelimit` ([PLAN 1414–1424](PLAN.md#polaris-ratelimit--gcra-rate-limiting-rpm-plus-token-weighted-tpm-priority-shedding-retry-after-15-minute-window-tracking)) | unit; proptest; Loom if CAS loop; Kani; load; 429 spike; 10k-cancellation stress; 1–32-frame fairness/scalability; synthetic fixtures then live headers. | **UNVERIFIED/INCOMPLETE:** current phase status explicitly says complete behavioral/load verification missing; spike crate not started. |
| `polaris-security-capability` ([PLAN 2060–2067](PLAN.md#polaris-security-capability--biscuit-capability-tokens-session-authority-per-frame-attenuation-authorization)) | unit; attenuation-chain proptest; arbitrary-token fuzz; widening/replay security corpus; 1/4/16-block load; generated golden tokens. | **EVIDENCE-PROVED (SCOPED):** E45 covers workspace scope, frame-root/token binding, attenuation and readonly per-resource kernel authorization. **INCOMPLETE:** fuzz/load/security audit and full mutation pipeline are not certified by E45. |
| `polaris-security-secrets` ([PLAN 2207–2215](PLAN.md#polaris-security-secrets--secret-vault-keyring-storage-secrecyzeroize-handling-chacha20-poly1305-ram-sealing)) | unit; seal/open and nonce proptest; blob fuzz; canary/log/key separation/zeroize security; real Windows keyring (explicit skip if unavailable); load; generated fake-key fixtures. | **EVIDENCE-PROVED (SCOPED):** E32 Windows synthetic-keyring store/load/delete and vault contracts. **INCOMPLETE:** Linux backend, broader privacy/fuzz/formal and full composition. |
| `polaris-kernel-operators` ([PLAN 2948–2956](PLAN.md#polaris-kernel-operators--functional-operator-pipeline-pipe-fork_branch-fold-assert_guard-and-the-kernel-side-operators)) | unit; pure-pipeline proptest; PII fail-closed security; 10k-cancellation permit/deadlock stress; overhead load; fake end-to-end smoke; generated PII fixtures. | **EVIDENCE-PROVED (SCOPED):** E43 RateLimitOp admission/reconciliation cases and compile-fail check. **INCOMPLETE:** whole pipeline, PII path and cancellation stress. |
| `polaris-kernel-verifier` ([PLAN 3035–3044](PLAN.md#polaris-kernel-verifier--hybridmutationverifier-biscuit-datalog-authorization-ast-metric-calculus-s1-semantic-congruence)) | unit; layer-order/attenuation proptest; arbitrary-token fuzz; metric Kani; malicious-diff and TTL security corpus; 50k/s load; slow-S1 concurrent timeout stress; generated malicious diffs/fake keys and synthetic congruence fixtures. | **EVIDENCE-PROVED (SCOPED):** narrow authority/terminal and issued-witness tests only. **INCOMPLETE:** current status says full hybrid mutation route open; no broad matrix evidence found. |
| `polaris-runtime-frames` ([PLAN 3144–3152](PLAN.md#polaris-runtime-frames--agents-as-context-frames-over-one-s2-client-and-one-s1-engine)) | unit + compile-fail typestate; scope/quota proptest; quota Kani; 1–32-frame scalability; 1k cancellation stress; child-escalation security; role/tool fixtures. | **EVIDENCE-PROVED (SCOPED):** E46 durable journal and E47 binding/driver tests with synthetic provider fixtures. **INCOMPLETE:** live model loop, broader formal/concurrency matrix and human request protocol. |
| `polaris-runtime-context` ([PLAN 3221–3232](PLAN.md#polaris-runtime-context--lossless-context-virtualization-stable-prefix-scratchpad-ref-folding)) | unit; byte-exact/determinism/overflow proptest; ref/fold fuzz; fold bounds Kani; pure fold/ref Miri; throughput/allocation load; 150k-token/10-minute stress; 1k/10k/100k scalability; reference-forgery/PII security; cargo/CRLF/invalid-UTF8 fixtures. | **EVIDENCE-PROVED (SCOPED):** E47 window/archive/CAS and HTTP assembly fixture checks. **INCOMPLETE:** live tokenizer/lifecycle and full property/fuzz/formal/load matrix. |
| `polaris-models` ([PLAN 3655–3663](PLAN.md#polaris-models--model-registry-effort-tiers-subagent-models-mapping-and-local-model-lifecycle)) | unit; model mapping/lease proptest; acquire/evict Loom; fake-loader/catalog smoke; 5-minute pressure stress; custom-entry secret security; dated `models_info.json` and malformed synthetic fixtures. | **EVIDENCE-PROVED (SCOPED):** E42 20 lifecycle/catalog contracts on two OSes using test-only loaders. **INCOMPLETE:** production loader ownership and stress/formal/root-assembly gates. |
| `polaris-tools-math` ([PLAN 3838–3847](PLAN.md#polaris-tools-math--deterministic-math-tools-exact-decimal-calculator-interpolation-statistics-gpu-occupancy-and-bandwidth)) | exact arithmetic/interpolation/statistics/occupancy unit; mathematical proptest; calc and TOML fuzz; occupancy Kani; whole-crate Miri; expression-bomb security; 1M/s load; sourced real GPU fixtures. | **EVIDENCE-PROVED (SCOPED):** status cites 25 focused contracts. **INCOMPLETE:** full owner audit and fuzz/formal/performance/provenance gates. |
| `polaris-nebius-cloud-token-factory` ([PLAN 4659–4671](PLAN.md#polaris-nebius-cloud-token-factory--openai-compatible-token-factory-client-sse-chat-streaming-tool-calls-embeddings-model-list-usage-accounting)) | SSE/request/error unit; chunk-boundary UTF-8 proptest + differential oracle; parser/delta fuzz; frame-size Kani + parser Miri; synthetic/live/local provider contract; allocation/load; 32-stream scalability; 429/reset/slow-loris/10-MB stress; secret/redirect security; S0-5 live spike; provenance-labelled fixtures. | **EVIDENCE-PROVED (SCOPED):** synthetic SSE/provider tests and E47 loopback `/tokenize` HTTP security/parity tests. **BLOCKED-ON-ACCESS:** live Token Factory contract/spike. **INCOMPLETE:** local model loop and remaining matrix. |
| `polaris-nebius-cloud-sandboxes` ([PLAN 4748–4760](PLAN.md#polaris-nebius-cloud-sandboxes--token-factory-sandboxes-contree-through-the-contree-mcp-sidecar-run-sync-images-directory_state_id-branching-swe-env-resolver)) | unit; environment/path proptest; MCP decoder fuzz; fake/live MCP contract; keyed sidecar smoke; keyed 5×20 branch load; 1–8 fan-out scalability; sidecar/process/network stress; path/port/token security; S0-13 spike; provenance fixtures. | **INCOMPLETE/BLOCKED-ON-ACCESS:** adapter not implemented and no sidecar/access. Synthetic fake-server suite is itself still a TODO. |
| `polaris-spike-ratelimit` ([PLAN 7786–7791](PLAN.md#polaris-spike-ratelimit--burst-tests-for-the-rate-limiter-50-sub-agents-at-once-429-storms-retry-after-floods)) | 100 fake-clock repetitions of scenarios A–D; 10 req/s 10-minute real-clock load; window-rollover/cancellation stress; admitted-count proptest. | **INCOMPLETE:** documentation-only/no workload implementation or result. |
| `polaris-nebius-cloud` facade | No dedicated test matrix specified in its owner section; plan requires child re-export compile doctest and linked requirement/access/fallback docs. | **INCOMPLETE:** facade evidence does not establish required four-child demonstration or P3 provider composition. |

## E48 current implementation delta

The initial audit is historical. [E48](evidence/phase3-20261027/README.md) adds SDK/proto human contracts, a dedicated L4 host verifier, durable kernel request/answer/interruption and replay, actual bus answer dispatch, frame-bound mutation with real parser/capability/lease/hybrid-verifier/CAS ownership, and the stateless typed Contree adapter. S1 decisions and Contree responses in the new fixtures are synthetic. Model tool dispatch, actual frame-tree targeted cancellation, live M3/M4 and the remaining owner matrices remain incomplete.

## Highest-priority implementation holes

1. **Unblock truthful local token counting for M3.** Pair the actual model and server with an exact tokenizer for the same fully rendered chat request. Current local facts prove Ollama/model availability only; `VllmChatClient` is an explicit vLLM `/tokenize` adapter, and no vLLM/llama server is installed. Keep byte heuristics out of the production route.
2. **Complete the local live loop.** Compose a real model's tool call through authorization, edit proposal/application, P2 sandbox execution, verification and proof; capture the required transcript. E47's real driver/CAS/event bus with synthetic model outputs is not this acceptance. P2 is paused and its exit remains an entry dependency.
3. **Finish P3-X-5 runtime composition.** Core SDK/proto contracts, atomic durable request/answer/interruption/replay and signed kernel bus answers exist. Add model-visible ask_user dispatch, actual frame-tree cancellation, sibling progress and requester continuation evidence. UI stays deferred.
4. **Complete Contree deployment and acceptance.** The stateless typed adapter and synthetic contracts exist. Deployed sidecar wiring, SWE environment resolution and required live operation/fan-out/security acceptance remain open. Access is unavailable.
5. **Close rate-limit and hybrid mutation gaps.** The limiter's performance/burst acceptance is absent; operator and verifier pieces do not form the production approval pipeline.

## Audit boundaries

This checklist does not compute or endorse percentages. It does not mark P3 complete. It excludes visual UI implementation tasks because the user deferred UI, but includes P3-X-5's backend protocol compatibility. Keyed/live M4 work remains in the denominator and is marked BLOCKED-ON-ACCESS. See the companion [audit report](tmp/20261027-p3-audit/README.md) for raw local availability commands and limitations.
