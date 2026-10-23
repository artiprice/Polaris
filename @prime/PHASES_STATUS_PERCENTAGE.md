# Phase completion percentages

> Snapshot: 2026-10-23T15:02:11.174982+00:00 · Working tree, including uncommitted implementation.
> **LIKELY — every percentage below is an engineering estimate, not a measured proof of correctness.**
> **VERIFIED — scope inventory:** 93 workspace packages; 950 first-party source/config/spec files; 585 Rust files read by the inventory pass.

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
| P2 — Execution plane | 87% | **75%** | OPEN — substantial live evidence; write/shell/blueprint composition and final gates incomplete. |
| P3 — S2 loop, providers, models and frames | 36% | **17%** | OPEN — complete phase acceptance not demonstrated. |
| P4 — System One, inference and safety | 28% | **10%** | OPEN — complete phase acceptance not demonstrated. |
| P5 — Knowledge, search, vectors and ingest | 8% | **3%** | OPEN — complete phase acceptance not demonstrated. |
| P6 — Nebius depth and remote execution | 18% | **5%** | OPEN — complete phase acceptance not demonstrated. |
| P7 — Advanced agentics, research and extensions | 23% | **15%** | OPEN — complete phase acceptance not demonstrated. |
| P8 — DevEnv and Needle | 44% | **26%** | OPEN — complete phase acceptance not demonstrated. |
| P9 — Daemon, RPC and interfaces | 20% | **8%** | OPEN — complete phase acceptance not demonstrated. |
| P10 — Hardening and submission | 33% | **18%** | OPEN — complete phase acceptance not demonstrated. |

**Use the bold Verified % as the conservative progress indicator.** No percentage converts the current system into a production-ready release.

## Current facts that limit readiness

- **VERIFIED:** P1’s scoped 2026-10-22 record contains 1,009 passing Windows tests, Linux checks and separate workload/fuzz/formal evidence. That is a historical scoped acceptance, not a fresh all-phase result: [P1 record](P1_LOGIC_STATUS.md), [raw excerpt](evidence/p1/verification.txt).
- **VERIFIED:** current P2 evidence includes an immutable image, actual 3 ordinary + 2 independent sandbox tests, signed review/proof, 100 create/exec/destroy cycles on the 6-GiB VM, real gateway interruption recovery and zero final resources: [P2 checkpoint](P2_LOGIC_STATUS.md).
- **VERIFIED:** corrected 6-GiB S0-7: initial apply 6,333ms, incremental 5,180ms, fresh remote no-op round trip 109ms; real 10k checkout overflow/rescan passed in 325.85s. [S0-7 raw](tmp/p2/sync-s0-7-live-6g-roundtrip.txt), [checkout raw](tmp/p2/checkout-flood-real-6g.txt).
- **VERIFIED:** new builtin proposal/shell library paths have focused tests, but the current kernel still uses P1 tools and has no working general `ProposeDiff` dispatch. Existing-file replacement is implemented; create-new `write_file` is missing. Library schemas alone do not close this integration gap.
- **VERIFIED:** current dependency gate failed on one vulnerability and four unmaintained findings. `ctutils` was updated 0.4.3 → 0.4.4; a clean post-update deny result is **not yet established**. `cargo vet --locked` reports **584 unvetted dependencies**. No new exemption was added. [deny raw](tmp/p2/current-dependency-deny.txt), [vet raw](tmp/p2/current-dependency-vet.txt), [update raw](tmp/p2/ctutils-security-update.txt).
- **VERIFIED:** strict workspace Clippy failed; focused condition/conversion fixes were checked afterwards, but there is no final full clean result yet. [full raw](tmp/p2/current-workspace-clippy.txt), [focused raw](tmp/p2/clippy-sync-ratelimit2.txt).
- **VERIFIED failure / NOT VERIFIED property:** the full arbitrary-32-byte entropy Kani attempt reached its 1,500-second harness timeout: `VERIFICATION:- FAILED`, `CBMC timed out`, 0 successful harnesses. It peaked at 2GiB RAM without a cgroup OOM kill. Whole-crate automata Miri is running and has no final verdict. Neither is counted as a successful proof. [Kani raw](tmp/p2/automata-kani-entropy-scoped-20261023-02.txt).
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
| `P2-EXT-TOOLS` | 50% | 20% | New proposal/shell registry routes and focused tests; create-new write and kernel async/hybrid proposal wiring missing. |
| `P2-EXT-INTEGRATION` | 40% | 25% | Live execution proof/recovery works; general write/shell/blueprint production composition remains incomplete. |

### P3 — S2 loop, providers, models and frames

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-ratelimit`](../crates/polaris-ratelimit/src) | 65% | 20% | Dual RPM/TPM/GCRA reservation/reconciliation source; complete behavioral/load verification missing. |
| [`polaris-security-capability`](../crates/polaris-security/crates/capability/src) | 85% | 75% | Real signed Biscuit scopes/attenuation/expiry plus tests/fuzz/proofs; full mutation assembly/security audit open. |
| [`polaris-security-secrets`](../crates/polaris-security/crates/secrets/src) | 0% | 0% | Documentation-only secret vault; no production credential custody implementation. |
| [`polaris-nebius-cloud-token-factory`](../crates/polaris-nebius-cloud/crates/token-factory/src) | 65% | 30% | Actual HTTP/SSE/tools/models/embeddings source and offline tests; live provider, throughput and full loop absent. |
| [`polaris-nebius-cloud-sandboxes`](../crates/polaris-nebius-cloud/crates/sandboxes/src) | 0% | 0% | Documentation-only Contree adapter; access also missing. |
| [`polaris-nebius-cloud`](../crates/polaris-nebius-cloud/src) | 0% | 0% | Documentation-only facade; child implementations do not implement its own exports. |
| [`polaris-models`](../crates/polaris-models/src) | 65% | 25% | Catalog/effort/cancellable lifecycle source; real model assembly/eviction and validation incomplete. |
| [`polaris-tools-math`](../crates/polaris-tools/crates/math/src) | 80% | 55% | Decimal/interpolation/statistics/GPU logic with 25 contracts; full requirements/performance/fuzz audit incomplete. |
| [`polaris-runtime-frames`](../crates/polaris-runtime/crates/frames/src) | 65% | 20% | Frame typestate/quota/scope/streamed-step logic; working composed S2 loop not demonstrated. |
| [`polaris-runtime-context`](../crates/polaris-runtime/crates/context/src) | 65% | 20% | Folding/assembly/scratchpad/prefix logic; end-to-end context lifecycle not verified. |
| [`polaris-kernel-operators`](../crates/polaris-kernel/crates/operators/src) | 65% | 20% | Typed operators/retry/cancel policies exist; production agent-loop composition and validation open. |
| [`polaris-kernel-verifier`](../crates/polaris-kernel/crates/verifier/src) | 60% | 20% | Hybrid mutation/evidence logic exists; only narrower authority/terminal cases verified, full mutation route open. |
| [`polaris-spike-ratelimit`](../tests/spike/ratelimit/src) | 0% | 0% | Documented workload only; actual phase spike not implemented. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P3-X-1` | 0% | 0% | No composed local plan/edit/sandbox-test/proof transcript. |
| `P3-X-2` | 0% | 0% | Keyed Nemotron loop unavailable; Token Factory access missing. |
| `P3-X-3` | 0% | 0% | Contree sidecar/SWE task unavailable and adapter remains skeleton. |
| `P3-X-4` | 0% | 0% | Live SSE fixture recording unavailable; existing fixtures are offline/synthetic. |

### P4 — System One, inference and safety

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-worker`](../apps/polaris-worker/src) | 0% | 0% | Documentation-only supervised inference worker. |
| [`polaris-system-one-engine`](../crates/polaris-system-one/crates/engine/src) | 0% | 0% | Documentation-only inference engine; spike results are not this adapter. |
| [`polaris-system-one-client`](../crates/polaris-system-one/crates/client/src) | 0% | 0% | Documentation-only worker client. |
| [`polaris-security-pii`](../crates/polaris-security/crates/pii/src) | 55% | 20% | Bounded reversible vault/sealing logic; NER and sealed persistence/runtime integration incomplete. |
| [`polaris-security-rules`](../crates/polaris-security/crates/rules/src) | 0% | 0% | Documentation-only security rule packs. |
| [`polaris-system-one-schemas`](../crates/polaris-system-one/crates/schemas/src) | 60% | 20% | Schema/catalog/threshold validation exists; full calibration and consumers incomplete. |
| [`polaris-system-one-router`](../crates/polaris-system-one/crates/router/src) | 65% | 20% | Routing/effort/tool/file/sandbox ranking source; engine/calibrated runtime validation absent. |
| [`polaris-system-one-verifier`](../crates/polaris-system-one/crates/verifier/src) | 65% | 20% | Pre/postflight, merge and safety logic; actual S1 inference integration not verified. |
| [`polaris-system-one-watchdog`](../crates/polaris-system-one/crates/watchdog/src) | 55% | 15% | Loop/context/drift monitor logic; live supervision/calibration and validation open. |
| [`polaris-system-one`](../crates/polaris-system-one/src) | 45% | 15% | Exports exist; complete worker/inference/calibration composition missing. |
| [`polaris-spike-s1`](../tests/spike/s1/src) | 0% | 0% | Documented synthetic workload only; real phase harness absent. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P4-X-1` | 25% | 15% | Spike export artifacts exist, but reproducible production export/golden pipeline not delivered. |
| `P4-X-2` | 0% | 0% | No validated precision/recall calibration table for the production routes. |
| `P4-X-3` | 0% | 0% | MVP demo/video not recorded; UI also deferred. **UI deferred; excluded from aggregate.** |

### P5 — Knowledge, search, vectors and ingest

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-storage-vector`](../crates/polaris-storage/crates/vector/src) | 0% | 0% | Documentation-only VectorStore implementation. |
| [`polaris-ingest`](../crates/polaris-ingest/src) | 0% | 0% | Documentation-only document/OCR ingestion. |
| [`polaris-search-providers`](../crates/polaris-search/crates/providers/src) | 0% | 0% | Documentation-only live search providers. |
| [`polaris-search-sift`](../crates/polaris-search/crates/sift/src) | 60% | 20% | Provenance-preserving sift/fallback logic; integrated source races and validation incomplete. |
| [`polaris-search`](../crates/polaris-search/src) | 25% | 10% | Partial exports; providers and composed retrieval path absent. |
| [`polaris-nvidia-docs`](../crates/polaris-nvidia-docs/src) | 0% | 0% | Documentation-only NVIDIA ingestion/index/adviser. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P5-X-1` | 0% | 0% | SearXNG provider/container acceptance unavailable. |
| `P5-X-2` | 0% | 0% | Tavily provider not implemented/live; credential missing. |
| `P5-X-3` | 0% | 0% | No production NVIDIA sync/index/provenance seed run. |
| `P5-X-4` | 0% | 0% | No masked live embedding/vector-store run; access missing. |

### P6 — Nebius depth and remote execution

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-nebius-cloud-data-lab`](../crates/polaris-nebius-cloud/crates/data-lab/src) | 55% | 15% | Consent/proof/masking/export logic exists; remote API and fine-tune path absent/unverified. |
| [`polaris-nebius-cloud-ai-cloud`](../crates/polaris-nebius-cloud/crates/ai-cloud/src) | 0% | 0% | Documentation-only AI Cloud/E4 adapter. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P6-X-1` | 0% | 0% | No SWE resolver/branch tree transcript; Contree missing. |
| `P6-X-2` | 0% | 0% | No E4 remote gateway transcript; AI Cloud quota/access missing. |
| `P6-X-3` | 35% | 10% | Local export logic partly exists; no real Data Lab API/fine-tune trigger acceptance. |

### P7 — Advanced agentics, research and extensions

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-guest`](../apps/polaris-guest/src) | 40% | 25% | Real confined sync helpers and Linux tests; full structured exec/browser guest service missing. |
| [`polaris-ext-mcp`](../crates/polaris-ext/crates/mcp/src) | 0% | 0% | Documentation-only MCP client/registry integration. |
| [`polaris-ext-skills`](../crates/polaris-ext/crates/skills/src) | 80% | 75% | Confined discovery/selection/approval cache, Windows/Linux/redb evidence; runtime enablement/injection incomplete. |
| [`polaris-ext`](../crates/polaris-ext/src) | 25% | 10% | Skills exports/catalog exist; MCP and complete extension runtime absent. |
| [`polaris-browser`](../crates/polaris-browser/src) | 0% | 0% | Documentation-only browser/a11y/image adapter. |
| [`polaris-hacker`](../crates/polaris-hacker/src) | 0% | 0% | Documentation-only zero-token scanner/fuzzer/stress pipeline. |
| [`polaris-runtime-dag`](../crates/polaris-runtime/crates/dag/src) | 65% | 15% | Bounded scheduler/pool/race/cancel source; composed concurrent runtime/model checks incomplete. |
| [`polaris-runtime-research`](../crates/polaris-runtime/crates/research/src) | 40% | 35% | Deterministic core/checkpoint codec and tests; research runner/store/parser and 20-iteration resume absent. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P7-X-1` | 0% | 0% | No 20-iteration research run and kill/resume acceptance. |
| `P7-X-2` | 0% | 0% | No seeded-bug hacker pipeline run. |
| `P7-X-3` | 0% | 0% | No composed microVM browser/a11y/pHash loop; individual spikes are separate. |

### P8 — DevEnv and Needle

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-devenv-nix`](../crates/polaris-devenv/crates/nix/src) | 65% | 35% | Bounded parse/sanitize/cache/prompt and backend seam; actual Nix/store/policy integration absent. |
| [`polaris-devenv-docker`](../crates/polaris-devenv/crates/docker/src) | 80% | 70% | 12-test suite, actual UUID recovery/BuildKit cache and image exec pass; Podman/latency/full gate incomplete. |
| [`polaris-devenv-cicd`](../crates/polaris-devenv/crates/cicd/src) | 50% | 15% | Typed pipeline/matrix/sandbox engine; Dagger and complete execution validation incomplete. |
| [`polaris-devenv`](../crates/polaris-devenv/src) | 25% | 10% | Child exports exist; unified environment/CI command composition missing. |
| [`polaris-system-one-nano`](../crates/polaris-system-one/crates/nano/src) | 0% | 0% | Documentation-only Needle production adapter; earlier spike is not runtime integration. |

### P9 — Daemon, RPC and interfaces

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|
| [`polaris-desktop`](../apps/polaris-desktop/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-web`](../apps/polaris-web/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-daemon`](../apps/polaris-daemon/src) | 0% | 0% | Documentation-only daemon; no authenticated running listener. |
| [`polaris-rpc`](../crates/polaris-rpc/src) | 50% | 20% | Request auth/service adapter source; named-pipe/mTLS listener assembly and live verification absent. |
| [`polaris-ui-gui`](../crates/polaris-ui/crates/gui/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-ui-web`](../crates/polaris-ui/crates/web/src) | 0% | 0% | Documentation-only interface; user-deferred UI. **UI deferred; excluded from aggregate.** |
| [`polaris-spike-web`](../tests/spike/web/src) | 0% | 0% | Documentation-only backend connection/load spike. |

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P9-X-1` | 0% | 0% | Welcome-page visual acceptance is user-deferred. **UI deferred; excluded from aggregate.** |
| `P9-X-2` | 30% | 10% | Auth helpers exist; no complete named-pipe/mTLS running transport acceptance. |

### P10 — Hardening and submission

| Crate / subphase | Code % | Verified % | What exists / what remains |
|---|---:|---:|---|

| Named task / subphase | Code % | Verified % | Remaining scope / evidence |
|---|---:|---:|---|
| `P10-1` | 30% | 20% | Many P1/P2 suites exist; complete system mutation/formal/load/security matrix not delivered. |
| `P10-2` | 30% | 10% | Threat/policy tooling exists; current deny/vet fail, full audit/unsafe/licence review incomplete. |
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

**VERIFIED:** the inventory pass opened and hashed every included first-party file, assigned each workspace file to its deepest package, and compared all 93 package entries with the phase table and PLAN responsibilities/TODO sections. Requirement-based source/API review and retained-evidence review supplied the estimates. This is a progress audit; it is **not a line-by-line security proof of every file**.

Excluded from implementation credit: third-party `vendor/`, generated/build output, `target/`, dependency-probe resolution copies, `@prime/tmp/`, node_modules, fuzz corpora and binaries. Test/fixture source is inventoried but its mere presence does not count as a passing run. Existing PLAN checkbox state and status labels were not treated as sole evidence.

Inventory: [source-inventory.json](evidence/progress/source-inventory.json). Source fingerprint SHA256: `d03dcfc169984a10092ff72f6b2d05794024efd3e528802e994adde21c6f393d`. The working tree is changing during ongoing P2 implementation; this snapshot should be refreshed after the pending integration/final checks.

Important status reconciliation: `PHASES_STATUS.md` still has old P2 text such as “doc-only sandbox spike” or “live acceptance pending”; current code and newer raw P2 logs supersede those descriptions. Conversely, its P1 PASS labels certify the P1 slice, not later extensions in those crates. This report does not rewrite historical evidence or mark any open phase PASS.

Sources: [prompt.md](prompt.md), [PLAN.md](PLAN.md), [PHASES.md](PHASES.md), [PHASES_STATUS.md](PHASES_STATUS.md), [workspace ownership](tools/workspace.toml), [P1 acceptance](P1_LOGIC_STATUS.md), [P2 checkpoint](P2_LOGIC_STATUS.md).

## Highest-impact unfinished work

1. Finish P2 production proposal/write/sandbox-shell and blueprint composition, including new-file edits and the intended hybrid authorization path.
2. Close P2 formal/adversarial evidence and run the complete current Windows/Linux verification matrix; resolve dependency/security failures without weakening gates.
3. Compose P3 into an actual local plan → edit → sandbox test → proof loop; then validate keyed services when access exists.
4. Implement the S1 worker/engine/client and calibrated runtime path before treating the P4 policy/router code as a working System One.
5. Complete the absent provider/vector/ingest/MCP/browser/hacker/daemon adapters and their real integration before claiming later phases production-ready.

No UI work is authorized by this report; UI remains deferred.
