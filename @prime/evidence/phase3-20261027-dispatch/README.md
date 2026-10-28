# E49 — Frame dispatch, cancellation and current-tree proof

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

| Command | Exit | Raw output |
|---|---:|---|
| `kernel-frame-regression` | 0 | [01-kernel-frame-regression.txt](01-kernel-frame-regression.txt) |
| `scoped-safety` | 101 | [02-scoped-safety.txt](02-scoped-safety.txt) |
| `scoped-safety-fixed` | 101 | [03-scoped-safety-fixed.txt](03-scoped-safety-fixed.txt) |
| `scoped-root-safety` | 101 | [04-scoped-root-safety.txt](04-scoped-root-safety.txt) |
| `integration-current` | 101 | [05-integration-current.txt](05-integration-current.txt) |
| `integration-updated` | 101 | [06-integration-updated.txt](06-integration-updated.txt) |
| `integration-with-ancestry` | 101 | [07-integration-with-ancestry.txt](07-integration-with-ancestry.txt) |
| `regression-native` | 101 | [08-regression-native.txt](08-regression-native.txt) |
| `scoped-final-safety` | 101 | [09-scoped-final-safety.txt](09-scoped-final-safety.txt) |
| `regression-current` | 101 | [10-regression-current.txt](10-regression-current.txt) |
| `regression-final` | 0 | [11-regression-final.txt](11-regression-final.txt) |
| `regression-shared-owners` | 0 | [12-regression-shared-owners.txt](12-regression-shared-owners.txt) |
| `library-safety-current` | 0 | [13-library-safety-current.txt](13-library-safety-current.txt) |
| `workspace-generator` | 0 | [14-workspace-generator.txt](14-workspace-generator.txt) |
| `test-target-safety` | 101 | [15-test-target-safety.txt](15-test-target-safety.txt) |
| `test-target-safety` | 101 | [16-test-target-safety.txt](16-test-target-safety.txt) |
| `test-target-safety` | 0 | [17-test-target-safety.txt](17-test-target-safety.txt) |
| `corrected-test-safety-regression` | 0 | [18-corrected-test-safety-regression.txt](18-corrected-test-safety-regression.txt) |
| `workspace-check` | 0 | [quality/01-workspace-check.txt](quality/01-workspace-check.txt) |
| `members` | 0 | [quality/02-members.txt](quality/02-members.txt) |
| `layers` | 0 | [quality/03-layers.txt](quality/03-layers.txt) |
| `witness` | 0 | [quality/04-witness.txt](quality/04-witness.txt) |
| `render-docs` | 0 | [quality/05-render-docs.txt](quality/05-render-docs.txt) |
| `check-docs` | 0 | [quality/06-check-docs.txt](quality/06-check-docs.txt) |
| `generator-preservation` | 0 | [quality/07-generator-preservation.txt](quality/07-generator-preservation.txt) |
| `linux-domain` | 101 | [gates/01-linux-domain.txt](gates/01-linux-domain.txt) |
| `linux-domain-current` | 0 | [gates/02-linux-domain-current.txt](gates/02-linux-domain-current.txt) |
