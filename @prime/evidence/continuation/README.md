# Non-UI phase continuation evidence

Snapshot: 2026-10-24. Phase 2 paused; UI deferred. These are scoped checks, not whole-phase acceptance. The frozen wire inventory now checks all previous tags plus the additive frame/approval fields, enum default and optional receipt presence; no old field assertion was removed.

| Command / condition | Raw evidence |
|---|---|
| `cargo check --offline --workspace` | [workspace-check.txt](workspace-check.txt) |
| `cargo test --offline -p polaris-config -p polaris-security-secrets -p polaris-nebius-cloud-token-factory -p polaris-rpc -p polaris-proto --no-fail-fast` | [contracts.txt](contracts.txt) |
| Frozen wire inventory, including every prior tag, new frame/approval tags, unspecified enum default and optional receipt presence | [wire-schema.txt](wire-schema.txt) |
| Windows `cargo test --offline -p polaris-rpc` | [rpc-windows.txt](rpc-windows.txt) |
| WSL/Linux `cargo test --offline -p polaris-rpc --test transport_contracts`; strict owned-package Clippy | [rpc-linux.txt](rpc-linux.txt) |
| `cargo clippy --offline -p polaris-rpc --all-targets --no-deps -- -D warnings` | [rpc-clippy.txt](rpc-clippy.txt) |
| RPC zero-cursor/session replay regression before fix | [rpc-replay-red.txt](rpc-replay-red.txt) |
| Frame contracts plus actual CAS/EventLog reopen | [frames.txt](frames.txt) |
| Strict owned-package frame Clippy | [frames-clippy.txt](frames-clippy.txt) |
| Config-to-SDK secret references before/after implementation | [references-red.txt](references-red.txt), [references-green.txt](references-green.txt) |
| SDK secret-source and content-store dispatch, scope of full SDK failure | [sdk-contracts.txt](sdk-contracts.txt), [sdk-schema-conflict.txt](sdk-schema-conflict.txt) |
| Scoped vector contracts, strict Clippy and performance limits | [vector.txt](vector.txt), [vector-clippy.txt](vector-clippy.txt), [vector-performance.txt](vector-performance.txt) |
| Strict Clippy for the eight changed SDK/config/provider/secrets/proto/RPC/frames/vector packages (`--all-targets --no-deps -- -D warnings`) | [clippy.txt](clippy.txt) |
| Member/layer/generated-document/format consistency | [consistency.txt](consistency.txt) |

Credentials in fixtures are synthetic. TLS material and bearer tokens are not printed. The live
Windows credential-manager experiment used one random test-owned account and removed it afterward.
No live Token Factory/Contree/search service acceptance is implied by local HTTP fixtures.

The historical/current SDK schema test contradiction is explicitly retained as a RED gate; no
assertion was removed to make it green. Vector performance is stated with dimensions and profile.

VERIFIED snapshot: 134 passing cases across the five-package config/secrets/provider/proto/RPC run (three opt-in tests ignored); 22 frame contracts; 19 vector contracts (one performance test ignored in the fast suite); 14 focused SDK contracts; eight Linux RPC checks. These counts overlap other listed focused runs and must not be added as unique system coverage. The separate SDK schema test remains RED.

Vector performance file contains explicitly labeled observed tool-output excerpts, not a reconstructed full raw stdout log. At 100k rows x 768 dimensions, debug measured 506.315803 seconds and release 568.1559863 seconds. Both miss the target; development jobs were concurrent, so an isolated calibration is still open.

The final witness policy check is RED: one existing skills unit-test fixture calls `ApprovalWitness::new` outside SDK/verifier (`approval_cache.rs:197`). `consistency.txt` retains the failure. Member/layer/format/document checks pass; the witness failure is not counted as passing consistency.
