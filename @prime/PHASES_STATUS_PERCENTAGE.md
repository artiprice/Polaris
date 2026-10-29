# Phase completion percentages

> **E51/E52 implementation is ongoing and UNVERIFIED:** the non-UI task, host-control, private checkpoint,
> calibrated-identity, model-pause, math-dispatch and prompt-fitting additions are not reflected in this dated estimate.
> Verification and percentage reassessment are deferred until implementation is complete, as requested.

> Snapshot: 2026-10-27T21:07:24.670085+00:00 · Scoped E50 update; other estimates retained.
> **LIKELY — every percentage below is an engineering estimate, not a measured proof of correctness.**
> **VERIFIED — mechanical inventory:** 93 workspace packages; 1350 first-party source/config/spec files; 816 Rust files opened and hashed.

> **Scope of this refresh:** E50 reassesses provider/model/math/Contree, worker/PII/S1/client/router/calibration, limiter and P3-X-1/P4-X-1/2 code/evidence. Other estimates retain prior bases. Security-rule/watchdog changes are still accumulating and their estimates remain pending owner verification. No fresh all-source acceptance audit or phase exit is claimed. [Evidence](evidence/phase3-20261027-local/README.md).

## How to read the percentages

- **Code %** estimates the owning phase’s intended behavior present in actual source, including necessary integration. Documentation, empty exports and compiling skeletons do not earn implementation credit. Source line count is not the denominator.
- **Verified %** estimates the same requirement scope backed by relevant retained native, integration, workload, fuzz or formal evidence. It is capped at Code %. A test count alone is not production acceptance. Historical evidence is distinguished from a fresh final-tree run.
- Percentages for individual work items use coarse five-point increments; uncertainty is roughly ±10–20 points for partial later-phase modules. Aggregate percentages are rounded arithmetic means, not effort/time forecasts.
- Phase summaries equally weight their assigned crate rows and named phase-task rows, **excluding rows marked UI deferred**. P0 also includes its 13 de-risking spikes. Diagnostic extension rows `P2-EXT-*` make cross-phase tool/composition gaps explicit. Milestones and per-PC inventory are separate views and are not counted twice.
- P1 rows score the accepted offline/non-UI P1 contract. P2 writes/shell, P3 model execution and P4 inference are scored in their own phase even when their files live in a P1-owned crate. A facade score covers its phase-local exports only; it does not certify its later-phase children.
- UI stays visible below but is excluded from the current non-UI phase aggregates. Missing accounts/keys remain unfinished; they are not removed from the denominator. A chosen fallback can complete a **de-risking decision**, while the production implementation still has its own later-phase score.
- **100% is not a whole-system guarantee.** A phase cannot exit merely because an average is high; its mandatory acceptance gate must pass independently.

## Phase overview — current non-UI scope

| Phase | Code % | Verified % | Exit-gate assessment |
|---|---:|---:|---|
| P0 — Foundations and de-risking | 78% | **72%** | OPEN — audits/access/manual spike conditions remain. |
| P1 — Offline kernel core | 100% | **95%** | Historical non-UI PASS (2026-10-22); M1/UI deferred; final expanded-tree gates not re-certified. |
| P2 — Execution plane | 87% | **75%** | PAUSED by user — prior live evidence retained; composition/final gates remain open. |
| P3 — S2 loop, providers, models and frames | 62% | **44%** | OPEN — complete phase acceptance not demonstrated. |
| P4 — System One, inference and safety | 65% | **38%** | OPEN — complete phase acceptance not demonstrated. |
| P5 — Knowledge, search, vectors and ingest | 41% | **30%** | OPEN — complete phase acceptance not demonstrated. |
| P6 — Nebius depth and remote execution | 23% | **9%** | OPEN — complete phase acceptance not demonstrated. |
| P7 — Advanced agentics, research and extensions | 50% | **35%** | OPEN — complete phase acceptance not demonstrated. |
| P8 — DevEnv and Needle | 46% | **30%** | OPEN — complete phase acceptance not demonstrated. |
| P9 — Daemon, RPC and interfaces | 59% | **45%** | OPEN — complete phase acceptance not demonstrated. |
| P10 — Hardening and submission | 34% | **19%** | OPEN — complete phase acceptance not demonstrated. |

**Use the bold Verified % as the conservative progress indicator.** No percentage converts the current system into a production-ready release.

## Current facts that limit readiness

- **VERIFIED:** the existing approval-cache fixture now obtains witnesses through the actual LocalDirectiveVerifier. All original cache-isolation/expiry assertions remain in verifier integration tests; `check-witness` reports zero violations. [Current gate evidence](evidence/continuation-20261026/README.md).
- **VERIFIED:** non-UI P3/P5/P9 continuation adds actual secret custody, bounded frame execution/journaling, scoped SQLite/HNSW storage and authenticated native/network RPC. Current commands, limitations and failures are recorded in [E32](PHASES_STATUS.md#e32--non-ui-p3p5p9-continuation-2026-10-24). Full phase acceptance remains open.
- **VERIFIED:** the full current SDK suite reports 56 passed and zero failed after the user-approved schema-maximum test correction. Minimum/future/round-trip/property checks remain; E41/E42 failure logs are historical. [Current raw output](evidence/continuation-20261026-e43/sdk-full-tests.txt).
- **VERIFIED:** P1’s scoped 2026-10-22 record contains 1,009 passing Windows tests, Linux checks and separate workload/fuzz/formal evidence. That is a historical scoped acceptance, not a fresh all-phase result: [P1 record](P1_LOGIC_STATUS.md), [raw excerpt](evidence/p1/verification.txt).
- **VERIFIED:** current P2 evidence includes an immutable image, actual 3 ordinary + 2 independent sandbox tests, signed review/proof, 100 create/exec/destroy cycles on the 6-GiB VM, real gateway interruption recovery and zero final resources: [P2 checkpoint](P2_LOGIC_STATUS.md).
- **VERIFIED:** corrected 6-GiB S0-7: initial apply 6,333ms, incremental 5,180ms, fresh remote no-op round trip 109ms; real 10k checkout overflow/rescan passed in 325.85s. [S0-7 raw](tmp/p2/sync-s0-7-live-6g-roundtrip.txt), [checkout raw](tmp/p2/checkout-flood-real-6g.txt).
- **VERIFIED:** new builtin proposal/shell library paths have focused tests, but the current kernel still uses P1 tools and has no working general `ProposeDiff` dispatch. Existing-file replacement is implemented; create-new `write_file` is missing. Library schemas alone do not close this integration gap.
- **VERIFIED failures:** the cached advisory check still reports six unmaintained crates, and cargo-vet reports 804 unvetted dependencies. These release gates remain failed without exemptions. [Current dependency evidence](evidence/continuation-20261026-e43/README.md).
- **VERIFIED:** strict workspace Clippy failed; focused condition/conversion fixes were checked afterwards, but there is no final full clean result yet. [full raw](tmp/p2/current-workspace-clippy.txt), [focused raw](tmp/p2/clippy-sync-ratelimit2.txt).
- **VERIFIED failure / NOT VERIFIED property:** the full arbitrary-32-byte entropy Kani attempt reached its 1,500-second harness timeout: `VERIFICATION:- FAILED`, `CBMC timed out`, 0 successful harnesses. It peaked at 2GiB RAM without a cgroup OOM kill. No final whole-crate automata Miri verdict is captured in this snapshot; P2 is paused. Neither is counted as a successful proof. [Kani raw](tmp/p2/automata-kani-entropy-scoped-20261023-02.txt).
- **VERIFIED source / NOT VERIFIED execution:** the new resident-pressure scenario compiles and passes focused Clippy but has not run against the real gateway. The generic hardened image context passes locked metadata; its changed full Docker build and production blueprint apply remain unverified.
- **VERIFIED:** credentials are missing from the doctor’s presence-only report; the report contains no secret payloads. [doctor JSON](evidence/p2/doctor-headless-current.json).

## Every phase and subphase

### P0 — Foundations and de-risking

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-tests`](../tests/src) | 40% | 30% | Actual cross-crate approval/session tests exist; all-system/heavy matrix and later phases incomplete. |
| [`xtask`](../xtask/src) | 100% | 95% | P0 member/layer/witness checks and negative cases exist; broader P10 security/release gates scored separately. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P0-SKEL-1` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-2` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-3` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-4` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-5` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-6` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-7` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-8` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-SKEL-9` | 100% | 95% | Existing skeleton/tooling/document contract checked historically; current graph has expanded. |
| `P0-ACC-1` | 0% | 0% | Account/key/quota acceptance is not available; no credential payload inspected. |
| `P0-ACC-2` | 0% | 0% | Account/key/quota acceptance is not available; no credential payload inspected. |
| `P0-ACC-3` | 0% | 0% | Account/key/quota acceptance is not available; no credential payload inspected. |
| `P0-ACC-4` | 0% | 0% | Account/key/quota acceptance is not available; no credential payload inspected. |
| `P0-ENV-1` | 100% | 90% | Reference environment/build tooling exists; historical native/WSL evidence, not every fresh-machine path. |
| `P0-ENV-2` | 100% | 90% | Reference environment/build tooling exists; historical native/WSL evidence, not every fresh-machine path. |
| `P0-ENV-3` | 100% | 90% | Reference environment/build tooling exists; historical native/WSL evidence, not every fresh-machine path. |
| `P0-ENV-4` | 100% | 90% | Reference environment/build tooling exists; historical native/WSL evidence, not every fresh-machine path. |
| `P0-ENV-5` | 100% | 90% | Reference environment/build tooling exists; historical native/WSL evidence, not every fresh-machine path. |
| `P0-ENV-6` | 80% | 60% | Local SSE endpoint works historically; dependable tool calling not established (recorded flaky/absent calls). |
| `P0-ENV-7` | 45% | 20% | Policies/tools exist; current deny/vet fail and Slint licence decision remains open. |
| `P0-XT-1` | 100% | 95% | Rust member/layer checks and planted-negative tests have native/WSL evidence. |
| `S0-1` | 100% | 100% | Real Windows SDK + WSL mTLS lifecycle demonstrated. |
| `S0-2` | 100% | 100% | Actual nested-KVM VM boot demonstrated; no production browser loop implied. |
| `S0-3` | 100% | 100% | De-risking decision complete: unsupported custom NemoClaw path rejected; native applier selected. Production applier remains P2. |
| `S0-4` | 85% | 75% | Conditional 260/260 parity with punctuation rule; CPU latency/RSS measured; CUDA/fp16 acceptance incomplete. |
| `S0-5` | 0% | 0% | Token Factory capability probe blocked on access. |
| `S0-6` | 100% | 100% | Needle engine/embeddings spike demonstrated; production adapter remains P8. |
| `S0-7` | 100% | 95% | Real corrected 6-GiB sync/no-op/10k overflow evidence; larger P2 adversarial matrix stays open. |
| `S0-8` | 90% | 80% | IPC spike measured; private per-user root security condition must remain enforced. |
| `S0-9` | 100% | 100% | De-risking decision complete: LanceDB budget failed; measured usearch/SQLite fallback selected. |
| `S0-10` | 30% | 10% | ConPTY probe authored; manual key acceptance still pending (UI deferred). **UI deferred; excluded from aggregate.** |
| `S0-11` | 100% | 100% | De-risking decision complete: unavailable MXC rejected; WSL tiers chosen. |
| `S0-12` | 90% | 80% | Guest/image/handshake/screenshot spike exists; default-policy Xvfb limitation retained. |
| `S0-13` | 0% | 0% | Contree authentication/branching spike blocked on access. |

### P1 — Offline kernel core

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-cli`](../apps/polaris-cli/src) | 100% | 95% | P1 headless CLI/main/doctor/RTK complete; TUI/dump rendering excluded here and scored under P1-X-2. |
| [`polaris-sdk`](../crates/polaris-sdk/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-proto`](../crates/polaris-proto/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-config`](../crates/polaris-config/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-log`](../crates/polaris-log/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-telemetry`](../crates/polaris-telemetry/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-code-intel`](../crates/polaris-code-intel/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-storage-cas`](../crates/polaris-storage/crates/cas/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-storage-eventlog`](../crates/polaris-storage/crates/eventlog/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-storage-registry`](../crates/polaris-storage/crates/registry/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-storage-cache`](../crates/polaris-storage/crates/cache/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-storage`](../crates/polaris-storage/src) | 100% | 95% | P1 facade exports checked; semantic vectors are P5, sync is P2. |
| [`polaris-kernel-bus`](../crates/polaris-kernel/crates/bus/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-kernel-directives`](../crates/polaris-kernel/crates/directives/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-kernel-supervisor`](../crates/polaris-kernel/crates/supervisor/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-kernel-reaper`](../crates/polaris-kernel/crates/reaper/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-tools-builtin`](../crates/polaris-tools/crates/builtin/src) | 100% | 95% | P1 read/pure/CAS/recall tools complete; P2 writes and async sandbox shell are a separate extension below. |
| [`polaris-tools-rtk`](../crates/polaris-tools/crates/rtk/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |
| [`polaris-tools`](../crates/polaris-tools/src) | 100% | 95% | P1 facade/catalog checked; math P3 and P2 mutable/shell services retain separate gates. |
| [`polaris-kernel`](../crates/polaris-kernel/src) | 100% | 95% | P1 local stores/session/directive composition complete; P2 proposal and P3 model composition scored separately. |
| [`polaris-ui-model`](../crates/polaris-ui/crates/model/src) | 65% | 55% | Substantial reducer/composer/completion logic and 111-test historical evidence; HUD/settings/fixtures/integration incomplete. **UI deferred; excluded from aggregate.** |
| [`polaris-ui-cli`](../crates/polaris-ui/crates/cli/src) | 0% | 0% | Documentation-only renderer; user-deferred. **UI deferred; excluded from aggregate.** |
| [`polaris-ui`](../crates/polaris-ui/src) | 0% | 0% | Documentation-only UI facade; user-deferred. **UI deferred; excluded from aggregate.** |
| [`polaris-spike-bus`](../tests/spike/bus/src) | 100% | 95% | P1 contract has native/WSL evidence in E30; current expanded-tree final regression remains open. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P1-X-1` | 100% | 95% | Actual LocalBackends/session typestate and signed directive composition; E30. |
| `P1-X-2` | 0% | 0% | TUI shell/composer/dump acceptance is user-deferred. **UI deferred; excluded from aggregate.** |
| `P1-X-3` | 100% | 95% | AWS-LC provider is installed/tested in actual main; E30. |

### P2 — Execution plane

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-storage-sync`](../crates/polaris-storage/crates/sync/src) | 90% | 80% | Bounded conditional sync, deny rules and real large/10k sync pass; broader scale/tearing/cancellation matrix open. |
| [`polaris-security-automata`](../crates/polaris-security/crates/automata/src) | 90% | 70% | Real scanners/redactor/entropy and fuzz/performance evidence; full entropy Kani and whole-crate Miri lack a final pass. |
| [`polaris-platform-wsl`](../crates/polaris-platform/crates/wsl/src) | 95% | 90% | Real WSL argv/lifecycle/KeepAlive/doctor tests; complete hostile configuration and fresh-host matrix open. |
| [`polaris-platform-process`](../crates/polaris-platform/crates/process/src) | 90% | 80% | Owned processes, PTY/watch overflow plus native stress; hostile batch/crash/scale matrix incomplete. |
| [`polaris-platform`](../crates/polaris-platform/src) | 100% | 85% | P2-local facade exports exist; this does not certify later-phase children or full agent composition. |
| [`polaris-vcs`](../crates/polaris-vcs/src) | 80% | 65% | Real gix/git, CAS undo, crash journal and worktrees; concurrent/adversarial/disk-full coverage incomplete. |
| [`polaris-openshell-client`](../crates/polaris-openshell/crates/client/src) | 90% | 80% | Real mTLS lifecycle, transfer, quotas and recovery evidence; wider reconnect/partition/client-concurrency matrix open. |
| [`polaris-openshell-policy`](../crates/polaris-openshell/crates/policy/src) | 90% | 80% | Pinned schema, least-privilege roles/provider policy and tests; complete live role/driver matrix incomplete. |
| [`polaris-openshell-nemoclaw`](../crates/polaris-openshell/crates/nemoclaw/src) | 80% | 60% | Driver binding/onboarding/context fixes and focused tests exist; changed image recipe build and integrated apply remain unverified. |
| [`polaris-openshell`](../crates/polaris-openshell/src) | 100% | 85% | P2-local facade exports exist; this does not certify later-phase children or full agent composition. |
| [`polaris-security`](../crates/polaris-security/src) | 100% | 85% | P2-local facade exports exist; this does not certify later-phase children or full agent composition. |
| [`polaris-runtime-leases`](../crates/polaris-runtime/crates/leases/src) | 90% | 80% | Conflicting-request FIFO, OCC, allocation, Loom and current ASan runs; final Kani and broader policy edge cases open. |
| [`polaris-runtime-guards`](../crates/polaris-runtime/crates/guards/src) | 85% | 75% | Blast/AST/proof evidence and real M2 gate; full classifier/adversarial matrix remains incomplete. |
| [`polaris-runtime`](../crates/polaris-runtime/src) | 100% | 85% | P2-local facade exports exist; this does not certify later-phase children or full agent composition. |
| [`polaris-spike-sandbox`](../tests/spike/sandbox/src) | 90% | 80% | 100 lifecycle cycles, checkout overflow, gateway kill, ENOSPC/OOM pass; new resident-pressure scenario compiled but not run. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P2-X-1` | 100% | 95% | Immutable final image, toolchain/guest handshake and real execution evidence; changed generic builder has its own open test. |
| `P2-X-2` | 100% | 100% | Doctor report: 26 items/actions, ready=true, required_missing=0; credential values omitted. |
| `P2-EXT-TOOLS` | 50% | 20% | P3 adds create-new/OCC/hybrid mutation service APIs. Model-visible proposal/shell/blueprint integration remains incomplete; paused P2 scores are retained. |
| `P2-EXT-INTEGRATION` | 40% | 25% | Live execution proof/recovery works; general write/shell/blueprint production composition remains incomplete. |

### P3 — S2 loop, providers, models and frames

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-ratelimit`](../crates/polaris-ratelimit/src) | 65% | 25% | E50 fixes weighted burst over-admission; preserved refund acceptance remains RED, including independently valid non-tail refund cases. No production refund completion is claimed. |
| [`polaris-security-capability`](../crates/polaris-security/crates/capability/src) | 85% | 75% | E49 adds separately scoped host cancellation authority alongside real mutation/read authorization; full revocation/security/load audit remains open. |
| [`polaris-security-secrets`](../crates/polaris-security/crates/secrets/src) | 85% | 75% | Real OS/environment SDK source and bounded zeroizing AEAD vault; Windows synthetic keyring roundtrip and contracts pass. Linux credential backend, fuzz/formal and full composition remain open; E32. |
| [`polaris-nebius-cloud-token-factory`](../crates/polaris-nebius-cloud/crates/token-factory/src) | 85% | 65% | E50 expands exact tool-call/usage handling and exercised synthetic provider contracts; keyed provider acceptance remains unavailable. |
| [`polaris-nebius-cloud-sandboxes`](../crates/polaris-nebius-cloud/crates/sandboxes/src) | 75% | 50% | E50 resolves trusted exact catalog repo/commit identities and validates near-miss results; actual credentialed Contree acceptance remains unavailable. |
| [`polaris-nebius-cloud`](../crates/polaris-nebius-cloud/src) | 60% | 35% | Nebius facade now exports the implemented AI Cloud surface. Provider transport and live remote execution remain absent; E41. |
| [`polaris-models`](../crates/polaris-models/src) | 80% | 60% | E50 adds exact loopback local-model ownership/profile validation and actual pinned runtime counting/chat evidence; full registry/remote-provider matrices remain open. |
| [`polaris-tools-math`](../crates/polaris-tools/crates/math/src) | 85% | 60% | E50 adds bounded compound exponent handling and numeric edge cases with focused native/Linux evidence; complete workload/formal acceptance remains open. |
| [`polaris-runtime-frames`](../crates/polaris-runtime/crates/frames/src) | 90% | 75% | E49 adds actual sealed step/ancestry admission, owned model tool dispatch and durable scoped cancellation; live/full stress/formal acceptance remains open. |
| [`polaris-runtime-context`](../crates/polaris-runtime/crates/context/src) | 85% | 60% | Production frame assembly now masks before provider counting, preserves complete groups and exposes verified archives only under actual CAS Read authority. Local HTTP/CAS fixtures and domain regression pass; live full-context lifecycle remains open; E47. |
| [`polaris-kernel-operators`](../crates/polaris-kernel/crates/operators/src) | 70% | 30% | RateLimitOp uses the real dual limiter, bounded cancellation-aware admission and noncloneable permits with dispatch/usage reconciliation; six cases and a compile-fail contract pass. Complete production pipeline composition remains open; E43. |
| [`polaris-kernel-verifier`](../crates/polaris-kernel/crates/verifier/src) | 75% | 50% | E49 wires real mutation evidence and latest current-tree ordinary/independent proof validation; S1 calibration and full hybrid matrices remain open. |
| [`polaris-spike-ratelimit`](../tests/spike/ratelimit/src) | 35% | 25% | E49 implements seven real limiter workloads with measured reports; full A–D repetition/load/stress matrix and unused refund acceptance remain open. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P3-X-1` | 60% | 40% | E50 actual read-only local Qwen and scripted edit/test pipelines are retained; combined live model edit/test/proof remains RED and keyed acceptance unavailable. |
| `P3-X-2` | 0% | 0% | Keyed Nemotron loop unavailable; Token Factory access missing. |
| `P3-X-3` | 0% | 0% | The stateless typed adapter exists, but deployed sidecar/SWE acceptance has not run. Contree access is unavailable. |
| `P3-X-4` | 0% | 0% | Live SSE fixture recording unavailable; existing fixtures are offline/synthetic. |
| `P3-X-5` | 85% | 75% | E49 adds model-visible durable ask/answer, scoped cancellation, sibling/requester and restart/race evidence; full stress/UI acceptance remains open. |

### P4 — System One, inference and safety

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-worker`](../apps/polaris-worker/src) | 80% | 65% | E50 actual authenticated PII/Decide reload and warm/post-eviction decision bursts pass; full ingest/nano/pressure/crash matrices remain open. |
| [`polaris-system-one-engine`](../crates/polaris-system-one/crates/engine/src) | 75% | 55% | E50 includes actual pinned Decide and PII CPU inference; fresh export/Python parity, fallback sidecar, precision/CUDA/resource matrices remain open. |
| [`polaris-system-one-client`](../crates/polaris-system-one/crates/client/src) | 85% | 65% | E50 bounded batching, exact cache/single-flight, model generation/incarnation fences and actual worker IPC are checked; full workload/formal acceptance remains open. |
| [`polaris-security-pii`](../crates/polaris-security/crates/pii/src) | 85% | 60% | E50 adds actual PII spans, bounded degradation, sealed reversible snapshots and semantic prompt masking; corpus, performance, fuzz and complete host key lifecycle remain open. |
| [`polaris-security-rules`](../crates/polaris-security/crates/rules/src) | 0% | 0% | Documentation-only security rule packs. |
| [`polaris-system-one-schemas`](../crates/polaris-system-one/crates/schemas/src) | 80% | 45% | E50 implements bounded binary calibration metrics and measured-candidate output; held-out real labels and production threshold evidence remain open. |
| [`polaris-system-one-router`](../crates/polaris-system-one/crates/router/src) | 75% | 50% | E50 includes combined task routing, exact environment identity short-circuits, bounded unique candidate sets and 16 focused tests; actual app triage wiring, calibration, injection/load matrices and catalogs remain open. |
| [`polaris-system-one-verifier`](../crates/polaris-system-one/crates/verifier/src) | 65% | 20% | Pre/postflight, merge and safety logic; actual S1 inference integration not verified. |
| [`polaris-system-one-watchdog`](../crates/polaris-system-one/crates/watchdog/src) | 55% | 15% | Loop/context/drift monitor logic; live supervision/calibration and validation open. |
| [`polaris-system-one`](../crates/polaris-system-one/src) | 65% | 40% | E50 checks consumer fail policy, bounded admission, circuit and host epoch fencing; real host/consumer composition is being implemented and full phase acceptance remains open. |
| [`polaris-spike-s1`](../tests/spike/s1/src) | 50% | 30% | E50 implements owned bounded burst dispatch, answer integrity and actual repeated-input CPU worker bursts; full A-D fault/load/stampede/policy matrix remains open. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P4-X-1` | 70% | 25% | E50 pinned guarded exporter/golden tooling and actual existing CPU artifacts are checked; fresh export/Python goldens/parity remain open. |
| `P4-X-2` | 55% | 20% | E50 bounded binary calibration and measured-candidate tooling are checked; real labelled held-out production threshold evidence remains open. |
| `P4-X-3` | 0% | 0% | MVP demo/video not recorded; UI also deferred. **UI deferred; excluded from aggregate.** |

### P5 — Knowledge, search, vectors and ingest

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-storage-vector`](../crates/polaris-storage/crates/vector/src) | 85% | 60% | Real scoped usearch/SQLite store with immutable embedding identity, atomic mutations, exact pre-limit payload filtering and cache recovery. Focused contracts pass; measured 100k x 768 debug/release throughput misses the target. Broader crash/recall and optional LanceDB gates remain open; E32. |
| [`polaris-ingest`](../crates/polaris-ingest/src) | 65% | 40% | Concrete text/Markdown/HTML/CSV/TSV/OOXML/text-layer PDF parsers, bounded preflight and available provenance; actual worker CAS flow passes. OCR, broader malformed-format matrix, preemptive resource isolation and complete knowledge/PII integration remain open; E40. |
| [`polaris-search-providers`](../crates/polaris-search/crates/providers/src) | 75% | 55% | Concrete redacting Tavily/SearXNG, DNS-pinned SSRF policy and bounded fetch/charset/DOM conversion; 18 local contracts pass. Live service/container, broader DNS/fuzz/load and audit gates remain open; E39. |
| [`polaris-search-sift`](../crates/polaris-search/crates/sift/src) | 85% | 60% | Bounded source race, pools, cancellation/drain, deduplicated S1 budget, SDK web bridge and hash-verified durable drop manifests. Local recovery/race contracts pass; live production composition, calibration and wider workloads remain open; E39. |
| [`polaris-search`](../crates/polaris-search/src) | 100% | 80% | Both real children exported with provider-to-race/sift flow documented and doctest checked. This facade score does not certify production kernel retrieval composition or live services; E39. |
| [`polaris-nvidia-docs`](../crates/polaris-nvidia-docs/src) | 60% | 45% | Profiles, CUDA advisor, llms.txt/HTML ingestion, BM25 index and fetcher; `$nvidia(...)`, `/nv-profile`, `/nv-verify` wired in every frontend with sample-repo evidence (E54). Docs sync/seed, vector search and tree-sitter rule packs remain. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P5-X-1` | 0% | 0% | SearXNG adapter and local protocol fixtures exist; the named live container/engine acceptance remains unperformed; E39. |
| `P5-X-2` | 0% | 0% | Tavily adapter and local protocol fixtures exist; the named keyed live acceptance remains unavailable; E39. |
| `P5-X-3` | 0% | 0% | No production NVIDIA sync/index/provenance seed run. |
| `P5-X-4` | 0% | 0% | No masked live embedding/vector-store run; access missing. |

### P6 — Nebius depth and remote execution

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-nebius-cloud-data-lab`](../crates/polaris-nebius-cloud/crates/data-lab/src) | 55% | 15% | Consent/proof/masking/export logic exists; remote API and fine-tune path absent/unverified. |
| [`polaris-nebius-cloud-ai-cloud`](../crates/polaris-nebius-cloud/crates/ai-cloud/src) | 25% | 20% | Checked budget reservations, terminal TTL, strict mTLS/IP ingress checks and owner-label selection. Provider transport and live cloud integration remain absent; E41. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P6-X-1` | 0% | 0% | No SWE resolver/branch tree transcript; Contree missing. |
| `P6-X-2` | 0% | 0% | No E4 remote gateway transcript; AI Cloud quota/access missing. |
| `P6-X-3` | 35% | 10% | Local export logic partly exists; no real Data Lab API/fine-tune trigger acceptance. |

### P7 — Advanced agentics, research and extensions

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-guest`](../apps/polaris-guest/src) | 40% | 25% | Real confined sync helpers and Linux tests; full structured exec/browser guest service missing. |
| [`polaris-ext-mcp`](../crates/polaris-ext/crates/mcp/src) | 75% | 50% | Real RMCP stdio/HTTP client, bounded framing, schema-aware normalization, caller authorization and lifecycle checks. Local protocol fixtures pass; live sandbox-backed MCP, production registry/frame assembly and wider lifecycle/adversarial gates remain open; E39. |
| [`polaris-ext-skills`](../crates/polaris-ext/crates/skills/src) | 80% | 75% | Confined discovery/selection/cache retained; authentic LocalDirectiveVerifier-issued cache witnesses pass and check-witness is green. Skill-specific approval, runtime enablement/injection and full trust integration remain open; E39. |
| [`polaris-ext`](../crates/polaris-ext/src) | 75% | 55% | Skills and concrete MCP client exported with shared trust documentation; full capability-bound kernel extension assembly remains open; E39. |
| [`polaris-browser`](../crates/polaris-browser/src) | 45% | 35% | Host side: a11y outline, DCT pHash visual diff and verified sandbox artifact session with tests. Guest-side Chromium driver and the microVM loop wait for the sandbox image (P2-X-1). |
| [`polaris-hacker`](../crates/polaris-hacker/src) | 40% | 30% | Bounded metadata-only AST/DFA observations, deterministic boundary corpus and cancellation-owned SDK sandbox stress. Seeded/local lifecycle tests pass; live sanitizers, race/CUDA/performance analysis and runtime command assembly remain open; E40. |
| [`polaris-runtime-dag`](../crates/polaris-runtime/crates/dag/src) | 70% | 30% | Task-ID-correlated completions, explicit interruptions, dependency skipping and cancellation/drop contracts now have post-implementation evidence. Full-scheduler join interruption, actual panic, snapshots, race/pool stress and production integration remain open; E44. |
| [`polaris-runtime-research`](../crates/polaris-runtime/crates/research/src) | 80% | 60% | Strict program parser, locked durable checkpoint/CAS evidence, path-attenuated tools, VCS receipts and bounded runner. Twenty real local-process iterations plus crash/reopen pass; live sandbox/model composition, cross-process contention, broader crash windows and adversarial critique remain open; E39. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P7-X-1` | 70% | 50% | Twenty capability-checked local process/Git/CAS iterations, all 60 payloads reopened and one abrupt-exit recovery pass. Actual sandbox/model research composition and broader crash matrix remain open; E39. |
| `P7-X-2` | 90% | 80% | `/hack` composed and wired in every frontend; seeded race/blocking/CUDA acceptance 5/5 with 0 false positives on the synthetic repo, calibrated on this repository (E55). Sandbox sanitizer workloads remain open. |
| `P7-X-3` | 0% | 0% | No composed microVM browser/a11y/pHash loop; individual spikes are separate. |

### P8 — DevEnv and Needle

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-devenv-nix`](../crates/polaris-devenv/crates/nix/src) | 65% | 35% | Bounded parse/sanitize/cache/prompt and backend seam; actual Nix/store/policy integration absent. |
| [`polaris-devenv-docker`](../crates/polaris-devenv/crates/docker/src) | 80% | 70% | 12-test suite, actual UUID recovery/BuildKit cache and image exec pass; Podman/latency/full gate incomplete. |
| [`polaris-devenv-cicd`](../crates/polaris-devenv/crates/cicd/src) | 80% | 70% | Bounded CI caps, owned cancellation cleanup (E41); cache mounts for backends that apply them and the cached Rust pipeline, run live in Docker with persistent cache volumes (E56). Dagger remains open. |
| [`polaris-devenv`](../crates/polaris-devenv/src) | 60% | 50% | Child exports; composed as `/ci` and `/ci-run` in every frontend (E56). Nix environment composition remains. |
| [`polaris-system-one-nano`](../crates/polaris-system-one/crates/nano/src) | 0% | 0% | Documentation-only Needle production adapter; earlier spike is not runtime integration. |

### P9 — Daemon, RPC and interfaces

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-desktop`](../apps/polaris-desktop/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-web`](../apps/polaris-web/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-daemon`](../apps/polaris-daemon/src) | 60% | 35% | Real headless CLI composes local kernel and authenticated RPC listener with bounded configuration and shutdown. P1 profile exposes authenticated health/events; mutation dispatch and terminal attachment require a fuller kernel profile. Live remote deployment and broader lifecycle gates remain open; E39. |
| [`polaris-rpc`](../crates/polaris-rpc/src) | 85% | 70% | Real pipe/UDS/TCP+mTLS servers and credential-bound SDK client; Windows/Linux network, receipt, replay and shutdown checks pass. Production daemon/full-kernel assembly remains open; E32. |
| [`polaris-ui-gui`](../crates/polaris-ui/crates/gui/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-ui-web`](../crates/polaris-ui/crates/web/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-spike-web`](../tests/spike/web/src) | 0% | 0% | Documentation-only backend connection/load spike. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P9-X-1` | 0% | 0% | Welcome-page visual acceptance is user-deferred. **UI deferred; excluded from aggregate.** |
| `P9-X-2` | 95% | 85% | Real Windows pipe ownership/100 connections, Linux private UDS, mTLS trust failures, token/rights/revocation and bounded shutdown checks pass (E32); desktop and web attach to `polarisd` over the named pipe with daemon-issued credentials, attach security tests pass (E53). Independent security audit and remote mTLS deployment remain open. |

### P10 — Hardening and submission

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P10-1` | 30% | 20% | Many P1/P2 suites exist; complete system mutation/formal/load/security matrix not delivered. |
| `P10-2` | 35% | 15% | Locked/offline full Cargo-graph SPDX inventory, lockfile digest, deterministic bytes, official schema and negative checks now have evidence. This is not a licence/security audit; current deny/vet and full unsafe/licence review remain open; E43. |
| `P10-3` | 35% | 25% | Actual P1/P2 budgets recorded; all-phase latency/RSS/throughput table incomplete. |
| `P10-4` | 20% | 0% | Dist profile/build settings exist; no verified full submission/installer artifacts and hashes. |
| `P10-5` | 0% | 0% | Demo/video and judge fallback acceptance absent; UI deferred and keys missing. **UI deferred; excluded from aggregate.** |
| `P10-6` | 50% | 35% | Real Linux check legs exist; full current portability matrix/macOS adaptation not delivered. |

## Milestones — separate acceptance view

| Milestone | Code % (estimate) | Verified % (estimate) | Acceptance boundary |
|---|---:|---:|---|
| M0 — skeleton/tooling | 100% | 95% | Historical skeleton gate passed; current expanded graph has unresolved security checks. |
| M1 — sessions/directives **from TUI** | 75% | 65% | Offline kernel/headless session path exists; TUI acceptance deferred. |
| M2 — first sandboxed test **from TUI** | 85% | 80% | Headless replacement acceptance passed; original TUI milestone is deferred, full P2 still open. |
| M3 — composed local S2 loop | 35% | 10% | Building blocks exist; no complete plan/edit/test/proof transcript. |
| M4 — Nemotron + Contree loop | 15% | 0% | Some provider source exists; Contree and live keyed integration unavailable. |
| M5 — MVP demo | 10% | 0% | Reusable foundations/spikes exist; demo and S1 runtime missing. |
| M6 — submission build + video | 10% | 0% | Build configuration exists; complete artifacts/video not delivered. |

## Per-PC setup — observed values, not phase averages

These are the retained doctor observations on this reference PC. A successful installation probe does not prove that the related product feature is implemented. `OBSERVED`/`UNKNOWN` is not a verified pass.

| Item | Owning phase | Required for current doctor | Observed status | Verified setup % |
|---|---|---|---|---:|
| `win-msvc` | P0 | yes | PASS | 100% |
| `win-git` | P0 | yes | PASS | 100% |
| `win-rust` | P0 | yes | PASS | 100% |
| `win-python` | P0 | yes | PASS | 100% |
| `win-cmake-nasm` | P0 | yes | PASS | 100% |
| `win-protoc` | P0 | no | PASS | 100% |
| `win-longpaths` | P0 | yes | PASS | 100% |
| `win-cargo-tools` | P0 | yes | PASS | 100% |
| `win-uv` | P0 | yes | PASS | 100% |
| `win-ollama` | P0 | no | PASS | 100% |
| `win-keyring` | P3 | no | OBSERVED | 0% |
| `win-models` | P4 | no | PASS | 100% |
| `win-cuda` | P4 | no | UNKNOWN | 0% |
| `win-pdfium` | P5 | no | MISSING | 0% |
| `win-wasm32` | P9 | no | FAIL | 0% |
| `wsl-distro` | P0 | yes | PASS | 100% |
| `wsl-docker` | P0 | yes | PASS | 100% |
| `wsl-openshell` | P0 | yes | PASS | 100% |
| `wsl-keepalive` | P0 | no | FAIL | 0% |
| `wsl-rust` | P0 | yes | PASS | 100% |
| `wsl-kani` | P1 | no | PASS | 100% |
| `wsl-kvm` | P7 | no | PASS | 100% |
| `wsl-sandbox-image` | P2 | no | PASS | 100% |
| `wsl-searxng` | P5 | no | FAIL | 0% |
| `wsl-nix` | P8 | no | FAIL | 0% |
| `contree-mcp` | P3 | no | MISSING | 0% |

## Analysis coverage and consistency

**VERIFIED:** the mechanical inventory opened and hashed every included first-party file and assigned workspace files to the deepest owning package. This scoped refresh reassesses the package/task rows listed above against their recorded E39–E44 evidence; other requirement estimates retain the earlier audit basis. This is not a line-by-line security proof or a new audit of every package.

Excluded from implementation credit: third-party `vendor/`, generated/build output, `target/`, dependency-probe resolution copies, `@prime/tmp/`, node_modules, fuzz corpora and binaries. Test/fixture source is inventoried but its mere presence does not count as a passing run. Existing PLAN checkbox state and status labels were not treated as sole evidence.

Inventory: [source-inventory.json](evidence/progress/source-inventory.json). Source fingerprint SHA256: `ae60f28160d5b25ff5630c04b8aba0e72eacdce5fca4ce42213a4ee00d7ea439`. The working tree is changing during non-UI all-phase continuation; P2 is paused. This snapshot is an evidence checkpoint, not phase acceptance.

Important status reconciliation: `PHASES_STATUS.md` still has old P2 text such as “doc-only sandbox spike” or “live acceptance pending”; current code and newer raw P2 logs supersede those descriptions. Conversely, its P1 PASS labels certify the P1 slice, not later extensions in those crates. This report does not rewrite historical evidence or mark any open phase PASS.

Sources: [prompt.md](prompt.md), [PLAN.md](PLAN.md), [PHASES.md](PHASES.md), [PHASES_STATUS.md](PHASES_STATUS.md), [workspace ownership](tools/workspace.toml), [P1 acceptance](P1_LOGIC_STATUS.md), [P2 checkpoint](P2_LOGIC_STATUS.md).

## Highest-impact unfinished work

1. Compose P3 into an actual local plan/edit/test/proof run using the real provider, tool, context and journal adapters; close transcript/usage and cross-backend acceptance gaps.
2. Complete later non-UI worker/inference, retrieval/ingest/provider, research/MCP/browser/hacker and daemon compositions; close representative performance and dependency/security gates.
3. Validate keyed Token Factory, Contree, search and cloud integrations when access exists; local fixtures do not certify those services.
4. Complete calibrated S1 production assembly, checkpoint-aware cache/batching, PII-NER and resource/admission policy. The concrete worker/engine/client and real lifecycle evidence do not close these remaining P4 requirements.
5. P2 work is explicitly paused by the user. Its retained write/shell/blueprint composition, formal and adversarial gaps remain open for a later resumption.

This continuation changes no UI implementation. UI estimates above retain the earlier audit basis; consult E33–E38 for newer UI evidence.
