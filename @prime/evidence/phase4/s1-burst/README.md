# S1 owned workload and actual local worker observations

VERIFIED scoped evidence; Phase 3/4 exit remains open. `commands.json` preserves exact commands, terminal exits and raw output paths, including RED attempts.

- `05-answer-integrity-red.txt`: three behavioral failures for malformed choice/score results and duplicate request options. `06-answer-integrity-green.txt`: 7 passed after full answer-contract validation. `07-answer-integrity-safety.txt`: recorded no-dependency safety lint subset exits 0.
- `08-pinned-real-pii-native.txt`: actual pinned companion CPU inference, 1 passed. Nine weights/tokenizer artifacts are verified against exact revision/size/SHA256 in `@prime/tools/runtime/pii-3bbf02b190a2-provenance.json`.
- `09-pinned-real-worker-pii-and-bursts.txt`: 3 actual Windows worker cases passed. PII then Decide then PII reload returned the same UTF-8 spans for the planted fixture. Idle eviction/reload passed. Warm and post-eviction bursts each answered 50/50 with at most 8 owned tasks.
- The repeated-input bursts exercise cache/single-flight and reload. Observed warm elapsed 290,502 microseconds; post-eviction elapsed 16,991,473 microseconds. These are single-run observations, not throughput targets, distinct native-call counts, or quality measurements.
- `10-supervised-worker-owned-faults.txt`: actual behavioral RED (replacement did not occur after adapter kill/retirement). `11-supervised-worker-owned-faults-green.txt`: two exact owned generation kills, fresh authenticated replacements, and final joined retirement pass after fixing transient classification.
- `12-supervisor-regression.txt`: 50 passed, 0 failed, 0 ignored. `13-supervisor-library-safety.txt`: recorded production-library safety lint subset exits 0.

NOT VERIFIED: the complete 500-request/fault/stampede/per-consumer policy matrix, two-minute crash loop, distinct-input native invocation counts, long-run RSS/load, calibration, Python parity, CUDA/precision matrix, or whole-phase production acceptance. The first worker compilation attempts also retained genuine transient/incoherent-source failures; they are not treated as passing results.

Pinned download failures are retained in E50. The interrupted encoder partial failed whole-file SHA256, was quarantined under the ignored model directory, and was never used for inference. A fresh full fetch passed all hashes before these real-model runs. Model-card metadata is retained separately under `phase4/pii-model-provenance`; full upstream license/release review remains unverified.
