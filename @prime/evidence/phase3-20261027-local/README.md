# E50 — Local providers, System One and privacy integration

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

This evidence directory is still accumulating; no immutable final-tree seal is claimed.

| Command | Exit | Raw output |
|---|---:|---|
| `receipt-red` | 101 | [01-receipt-red.txt](01-receipt-red.txt) |
| `receipt-green` | 0 | [02-receipt-green.txt](02-receipt-green.txt) |
| `receipt-library-safety` | 0 | [03-receipt-library-safety.txt](03-receipt-library-safety.txt) |
| `limiter-workload-refund` | 101 | [04-limiter-workload-refund.txt](04-limiter-workload-refund.txt) |
| `limiter-workload-dispatched` | 0 | [05-limiter-workload-dispatched.txt](05-limiter-workload-dispatched.txt) |
| `configured-tool-compile` | 101 | [06-configured-tool-compile.txt](06-configured-tool-compile.txt) |
| `configured-tool-live` | 101 | [07-configured-tool-live.txt](07-configured-tool-live.txt) |
| `configured-tool-owned-wsl` | 101 | [08-configured-tool-owned-wsl.txt](08-configured-tool-owned-wsl.txt) |
| `configured-tool-sync-diagnostic` | 101 | [09-configured-tool-sync-diagnostic.txt](09-configured-tool-sync-diagnostic.txt) |
| `local-model-kernel-live` | 0 | [10-local-model-kernel-live.txt](10-local-model-kernel-live.txt) |
| `local-model-retained-transcript` | 0 | [11-local-model-retained-transcript.txt](11-local-model-retained-transcript.txt) |
| `human-answer-ref-red` | 101 | [12-human-answer-ref-red.txt](12-human-answer-ref-red.txt) |
| `human-answer-ref-green` | 0 | [13-human-answer-ref-green.txt](13-human-answer-ref-green.txt) |
| `human-authority-expiry-red` | 101 | [14-human-authority-expiry-red.txt](14-human-authority-expiry-red.txt) |
| `human-authority-expiry-green` | 0 | [15-human-authority-expiry-green.txt](15-human-authority-expiry-green.txt) |
| `kernel-current-regression` | 0 | [16-kernel-current-regression.txt](16-kernel-current-regression.txt) |
| `configured-real-model-compile` | 0 | [17-configured-real-model-compile.txt](17-configured-real-model-compile.txt) |
| `configured-tool-current-image` | 0 | [18-configured-tool-current-image.txt](18-configured-tool-current-image.txt) |
| `local-model-edit-sandbox-live` | 101 | [19-local-model-edit-sandbox-live.txt](19-local-model-edit-sandbox-live.txt) |
| `current-kernel-test-safety` | 101 | [19-current-kernel-test-safety.txt](19-current-kernel-test-safety.txt) |
| `local-model-edit-proof-gated` | 101 | [21-local-model-edit-proof-gated.txt](21-local-model-edit-proof-gated.txt) |
| `local-model-edit-proof-diagnostic` | 101 | [22-local-model-edit-proof-diagnostic.txt](22-local-model-edit-proof-diagnostic.txt) |
| `p4-domain-baseline` | 0 | [23-p4-domain-baseline.txt](23-p4-domain-baseline.txt) |
| `p4-calibration-api-red` | 101 | [24-p4-calibration-api-red.txt](24-p4-calibration-api-red.txt) |
| `p4-calibration-green` | 0 | [25-p4-calibration-green.txt](25-p4-calibration-green.txt) |
| `p4-calibration-safety` | 0 | [26-p4-calibration-safety.txt](26-p4-calibration-safety.txt) |
| `live-owned-model-loader` | 101 | [27-live-owned-model-loader.txt](27-live-owned-model-loader.txt) |
| `p4-calibration-runner-api-red` | 101 | [28-p4-calibration-runner-api-red.txt](28-p4-calibration-runner-api-red.txt) |
| `p4-calibration-runner-green` | 101 | [29-p4-calibration-runner-green.txt](29-p4-calibration-runner-green.txt) |
| `p4-calibration-runner-fixed` | 0 | [30-p4-calibration-runner-fixed.txt](30-p4-calibration-runner-fixed.txt) |
| `p4-calibration-cli` | 0 | [31-p4-calibration-cli.txt](31-p4-calibration-cli.txt) |
| `live-owned-model-loader-fixed` | 101 | [32-live-owned-model-loader-fixed.txt](32-live-owned-model-loader-fixed.txt) |
| `live-owned-model-loader-current` | 0 | [33-live-owned-model-loader-current.txt](33-live-owned-model-loader-current.txt) |
| `pii-snapshot-api-red` | 101 | [34-pii-snapshot-api-red.txt](34-pii-snapshot-api-red.txt) |
| `pii-snapshot-green` | 0 | [35-pii-snapshot-green.txt](35-pii-snapshot-green.txt) |
| `phase34-scoped-code-safety` | 0 | [36-phase34-scoped-code-safety.txt](36-phase34-scoped-code-safety.txt) |
| `seal-key-recovery-api-red` | 101 | [37-seal-key-recovery-api-red.txt](37-seal-key-recovery-api-red.txt) |
| `seal-key-recovery-green` | 0 | [38-seal-key-recovery-green.txt](38-seal-key-recovery-green.txt) |
| `pii-snapshot-cas-api-red` | 101 | [39-pii-snapshot-cas-api-red.txt](39-pii-snapshot-cas-api-red.txt) |
| `pii-snapshot-cas-green` | 101 | [40-pii-snapshot-cas-green.txt](40-pii-snapshot-cas-green.txt) |
| `pii-snapshot-cas-current` | 0 | [41-pii-snapshot-cas-current.txt](41-pii-snapshot-cas-current.txt) |
| `pii-snapshot-reopened-key` | 0 | [42-pii-snapshot-reopened-key.txt](42-pii-snapshot-reopened-key.txt) |
| `phase34-current-compile` | 0 | [43-phase34-current-compile.txt](43-phase34-current-compile.txt) |
| `prompt-privacy-restore-api-red` | 101 | [44-prompt-privacy-restore-api-red.txt](44-prompt-privacy-restore-api-red.txt) |
| `prompt-privacy-restored` | 101 | [45-prompt-privacy-restored.txt](45-prompt-privacy-restored.txt) |
| `prompt-privacy-current` | 101 | [46-prompt-privacy-current.txt](46-prompt-privacy-current.txt) |
| `prompt-privacy-restored-matching-capacity` | 0 | [47-prompt-privacy-restored-matching-capacity.txt](47-prompt-privacy-restored-matching-capacity.txt) |
| `model-probe-timeout-red` | 101 | [48-model-probe-timeout-red.txt](48-model-probe-timeout-red.txt) |
| `model-probe-timeout-green` | 101 | [49-model-probe-timeout-green.txt](49-model-probe-timeout-green.txt) |
| `consumer-policy-api-red` | 101 | [50-consumer-policy-api-red.txt](50-consumer-policy-api-red.txt) |
| `consumer-policy-current` | 0 | [51-consumer-policy-current.txt](51-consumer-policy-current.txt) |
| `semantic-prompt-api-red` | 101 | [52-semantic-prompt-api-red.txt](52-semantic-prompt-api-red.txt) |
| `semantic-prompt-current` | 0 | [53-semantic-prompt-current.txt](53-semantic-prompt-current.txt) |
| `model-probe-current` | 101 | [54-model-probe-current.txt](54-model-probe-current.txt) |
| `model-probe-independent-deadlines` | 0 | [55-model-probe-independent-deadlines.txt](55-model-probe-independent-deadlines.txt) |
| `task-routing-red` | 101 | [56-task-routing-red.txt](56-task-routing-red.txt) |
| `task-routing-combined` | 101 | [57-task-routing-combined.txt](57-task-routing-combined.txt) |
| `task-routing-valid-distribution` | 0 | [58-task-routing-valid-distribution.txt](58-task-routing-valid-distribution.txt) |
| `phase34-native-complete-selection-interrupted` | UNKNOWN | [59-phase34-native-complete-selection.txt](59-phase34-native-complete-selection.txt) |
| `phase34-native-complete-selection` | 101 | [60-phase34-native-complete-selection.txt](60-phase34-native-complete-selection.txt) |
| `root-policy-routing-regression` | 0 | [61-root-policy-routing-regression.txt](61-root-policy-routing-regression.txt) |
| `weighted-burst-red` | 101 | [62-weighted-burst-red.txt](62-weighted-burst-red.txt) |
| `weighted-burst-current` | 0 | [63-weighted-burst-current.txt](63-weighted-burst-current.txt) |
| `limiter-weighted-and-refund-current` | 101 | [64-limiter-weighted-and-refund-current.txt](64-limiter-weighted-and-refund-current.txt) |
| `installed-decide-real-inference` | 0 | [65-installed-decide-real-inference.txt](65-installed-decide-real-inference.txt) |
| `installed-s1-process-real-inference` | 0 | [66-installed-s1-process-real-inference.txt](66-installed-s1-process-real-inference.txt) |
| `installed-s1-supervised-real-inference` | 0 | [67-installed-s1-supervised-real-inference.txt](67-installed-s1-supervised-real-inference.txt) |
| `pinned-pii-download-interrupted` | UNKNOWN | [68-pinned-pii-download.txt](68-pinned-pii-download.txt) |
| `pinned-pii-reviewed-resume` | 1 | [69-pinned-pii-reviewed-resume.txt](69-pinned-pii-reviewed-resume.txt) |
| `pinned-pii-fresh-after-rejected-partial` | 0 | [70-pinned-pii-fresh-after-rejected-partial.txt](70-pinned-pii-fresh-after-rejected-partial.txt) |
| `export-readiness-current` | 3 | [71-export-readiness-current.txt](71-export-readiness-current.txt) |
| `workspace-check` | 101 | [quality/01-workspace-check.txt](quality/01-workspace-check.txt) |
| `members` | 0 | [quality/02-members.txt](quality/02-members.txt) |
| `layers` | 0 | [quality/03-layers.txt](quality/03-layers.txt) |
| `witness` | 0 | [quality/04-witness.txt](quality/04-witness.txt) |
| `render-docs` | 0 | [quality/05-render-docs.txt](quality/05-render-docs.txt) |
| `check-docs` | 0 | [quality/06-check-docs.txt](quality/06-check-docs.txt) |
| `generator-preservation` | 0 | [quality/07-generator-preservation.txt](quality/07-generator-preservation.txt) |
| `p4-linux-current` | 101 | [gates/01-p4-linux-current.txt](gates/01-p4-linux-current.txt) |
| `phase34-linux-current` | 0 | [gates/02-phase34-linux-current.txt](gates/02-phase34-linux-current.txt) |
| `root-production-safety` | 101 | [gates/03-root-production-safety.txt](gates/03-root-production-safety.txt) |
| `root-production-safety-current` | 101 | [gates/04-root-production-safety-current.txt](gates/04-root-production-safety-current.txt) |
| `workspace-check-rpc-fields` | 0 | [gates/05-workspace-check-rpc-fields.txt](gates/05-workspace-check-rpc-fields.txt) |
| `root-production-safety-after-key-fix` | 0 | [gates/06-root-production-safety-after-key-fix.txt](gates/06-root-production-safety-after-key-fix.txt) |
| `root-test-target-safety` | 101 | [gates/07-root-test-target-safety.txt](gates/07-root-test-target-safety.txt) |
| `root-test-target-safety-current` | 0 | [gates/08-root-test-target-safety-current.txt](gates/08-root-test-target-safety-current.txt) |
| `phase34-linux-latest` | 0 | [gates/09-phase34-linux-latest.txt](gates/09-phase34-linux-latest.txt) |
| `s1-burst-api-red` | 101 | [gates/09-s1-burst-api-red.txt](gates/09-s1-burst-api-red.txt) |

Additional focused source-owned evidence: [client batching](../phase4/client-batching/), [single-flight](../phase4/client-singleflight/), [PII span integration](../phase4/pii-ner/), [actual pinned PII/worker bursts and supervisor recovery](../phase4/s1-burst/README.md), [router environments](../phase4/router-environments/), [refund RED evidence](../phase4/ratelimit-refunds/03-design-blocker.md), [export pipeline](../phase4/p4-x1-export/).
