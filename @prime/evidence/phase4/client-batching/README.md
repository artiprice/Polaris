# P4 System-One client batching and cache

## Implemented

- Added one lazy dispatcher task per `S1Client`, with a 3 ms collection window, 8-item maximum, 32 queued requests, and exact protobuf encoded-size checks against the 1 MiB worker message cap.
- Preserved authenticated worker handshake and each caller deadline. Each decision deadline includes its bounded identity lookup, queue delay, and decision RPC; canceled callers have independent oneshot receivers.
- Added an additive `HealthResponse` loaded-model identity pair. Empty digest plus generation zero means identity unavailable (including old workers); malformed or non-ready health results cannot authorize a cache lookup.
- Engine identity hashes the fixed ONNX fragment/tokenizer manifest, ONNX Runtime library bytes, normalization mode, and intra-op thread count. Hashing streams through a fixed buffer. The engine compares pre-load and post-load digests and refuses unstable inputs.
- Worker publishes identity only while the native model is loaded, increments its generation on each load in that worker process, and clears identity on explicit/idle eviction and shutdown.
- Cache keys bind that identity/generation to the complete encoded DecisionRequest (schema, already-masked text, and context). Hits also compare original request bytes. The cache is limited to 256 entries and 4 MiB combined serialized request and decision bytes; identity change or unavailable identity clears old entries.
- Cache insertion requires ready authenticated identity probes both immediately before and after the decision batch, with exact identity equality. This prevents a response that crosses an eviction/reload from being labeled as the later generation.

## Evidence

- `focused-tests.txt`: `cargo test -p polaris-system-one-client -p polaris-system-one-engine -p polaris-worker`; exit 0. Client 21 unit + 4 integration tests, engine 8 unit tests, worker 12 library + 3 binary + integration tests passed. Real model tests remain ignored because local GLiNER/ORT paths are unavailable.
- `scoped-clippy.txt`: `cargo clippy -p polaris-system-one-client -p polaris-system-one-engine -p polaris-worker --all-targets --no-deps -- -D warnings`; exit 0.
- `race-regression-red.txt`: pre-fix race test observed one RPC (`[1]`) instead of two (`[1, 1]`), proving the old post-RPC-only identity binding could cache across a generation change. The fixed test passes in `focused-tests.txt`.
- Descriptors: `specs/descriptors/system_one_client_batching.json` and `specs/descriptors/system_one_checkpoint_bound_cache.json`.

## Limits

- No real GLiNER inference or loaded-model digest was exercised in this focused run. Identity hashing and worker identity publication are covered with deterministic local unit/fixture behavior; actual model artifacts remain unavailable to these checks.
- Legacy workers omit the additive health fields and automatically disable caching. Protobuf fields are additive; the worker protocol major version was not changed.