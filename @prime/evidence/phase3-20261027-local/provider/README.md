# llama.cpp production adapter evidence — 2026-10-27

The opt-in integration test exercised the actual `LlamaCppChatClient` through a local capture proxy and the pinned local model. The proxy recorded the adapter's exact JSON request bodies and raw HTTP responses under this directory. It forwarded only to `127.0.0.1:18931` and rewrote the `Connection` header to close each upstream exchange; request bodies were preserved byte-for-byte.

Latest retained run: `live-14072-run.json`; per-file byte lengths and SHA-256 values: `live-14072-sha256.json`.

| Artifact | Bytes | SHA-256 |
|---|---:|---|
| `live-14072-count-request.json` | 506 | `3540543b4cc0eab95ef15274f6e661f98b5d27201bdeb8a96fbe26bddc563357` |
| `live-14072-chat-request.json` | 506 | `3540543b4cc0eab95ef15274f6e661f98b5d27201bdeb8a96fbe26bddc563357` |
| `live-14072-count-response.http` | 208 | `a1091b9b1dba58a48aac3cf70239e5a553c7cca23f292fd47f6ceb6a66678273` |
| `live-14072-chat-response.http` | 4410 | `c8329891001174690f256e20302a39513cec2d07752e6d4df6c804c2c5b30711` |

The raw count response reports `input_tokens=174`. The raw streamed chat response contains one structured `p3_local_probe` call with `{"probe_id": "adapter-smoke"}` and a usage chunk reporting `prompt_tokens=174`, `completion_tokens=25`. The test asserted count/chat request-body equality and usage parity. Model SHA-256: `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`; server archive SHA-256: `3a3e417a0816329829081bdd443e3045ff3b2dbe866fcef89a1d919e941ce23f`.

The commands run were:

```text
cargo test -p polaris-nebius-cloud-token-factory
cargo test -p polaris-nebius-cloud-token-factory --test local_llama_cpp -- --ignored --nocapture
cargo clippy -p polaris-nebius-cloud-token-factory --all-targets -- -D warnings
```

The full provider suite passed with 43 tests across the crate's unit/integration test binaries and two ignored live tests; the dedicated local adapter integration passed when explicitly run. No actual tool execution, file mutation, sandbox plan/test, or frame proof-of-done occurred in this probe.
