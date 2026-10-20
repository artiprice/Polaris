# S0-6 Needle 3 spike

RESULT: PASS (windows)

Conditions (VERIFIED): Windows 11 x64, Python 3.12.15 venv (uv), cactus-needle from PyPI (installed via `python -m uv pip install`), engine 3.3.3 (`libneedle.dll` auto-fetched from HF to `C:\Users\Admin\.cache\cactus-needle\v3\3.3.3\`), needle3.cact 35.34 MB (2-bit published archive), NEEDLE_TELEMETRY=0 DO_NOT_TRACK=1. WSL2 not needed/not tried.

## Commands
```
python -m uv venv --python 3.12 .venv
python -m uv pip install --python .venv/Scripts/python.exe cactus-needle psutil
.venv/Scripts/needle.exe fetch
.venv/Scripts/python.exe bench.py          # output: bench-windows.txt
.venv/Scripts/needle.exe build --platform windows-x86_64 --out C:/Users/Admin/.polaris/models/needle3/win
needle.exe --model needle3.cact --tools tools.json --serve      # native runner
```
Native files (VERIFIED): `C:\Users\Admin\.polaris\models\needle3\win\{needle.exe 1.57MB, libneedle.a, needle.h, needle3.cact 35.3MB}`. (The Python path caches weights in the HF cache, not ~/.polaris.)

## Measurements (VERIFIED, python API, one machine, CPU, single run of bench.py)
- Model load (Needle(tools=3 tools)): 2.80 s (incl. first-time cache); first call 0.32 s.
- Tool call, 50 calls: p50 316.8 ms, p95 439.0 ms. Reported decode 183 tok/s, prefill 526 tok/s.
- Embedding, 100 sentences: dimension 3072 (list of floats), p50 50.3 ms, p95 68.9 ms.
- RSS: 112 MB after first call, peak working set 128 MB; engine self-reports peak_ram_mb 122 (python) / 80-82 (native exe). Native exe process was ~69 MB at idle after serving.
- Accuracy on my 7 queries (informal, not a benchmark): 4/7 correct. "read the file src/main.rs", "run the tests for crate polaris-core", "search for TODO", "open Cargo.toml" correct; "test the crate net-utils" and "find where parse_args is defined" returned empty `[]` (missed); weather query correctly refused.

## Tool-call output shape (RAW)
```
{"type": "call", "success": true, "error": null, "error_code": null, "reason": null, "function_calls": [{"name": "read_file", "arguments": {"path": "src/main.rs"}}], "suppressed_calls": [], "reasoning": "'read the file src/main.rs' -> read_file with path 'src/main.rs'.", "confidence": 0.8676, "prefill_tps": 525.6, "decode_tps": 183.5, "peak_ram_mb": 122.4, "validation": {"ungrounded": [], "negation": false}}
```
tools.json format: array of `{name, description, parameters:{type:object,properties,required}}` (see tools.json).

## --serve protocol (VERIFIED by curl)
- Plain HTTP/1.1 on 127.0.0.1 only, default port 8080 (`--port N`). Startup line: `needle serving on http://127.0.0.1:8080  (POST /complete {"input":"..."}, POST /reset)`.
- `POST /complete` body `{"input":"<text>"}` -> 200 `application/json`, same JSON object as above. Tools fixed at startup via --tools. Turns accumulate as one conversation (per --help) ; `POST /reset` -> 200 `{}`.
- No embedding endpoint: `POST /embed` -> 404 (VERIFIED). Embeddings only via Python `agent.embed(text)` or C `needle_embed`.
- HTTP latency, 10 sequential curls: 0.34-0.53 s total (median ~0.41 s), slightly slower than in-process.
```
curl -s -X POST http://127.0.0.1:8080/complete -H "Content-Type: application/json" -d '{"input":"search for TODO in the code"}'
HTTP/1.1 200 OK  Content-Type: application/json
{"type":"call","success":true,...,"function_calls":[{"name":"search","arguments":{"query":"TODO"}}],...,"confidence":0.8282,...}
```
Other CLI flags: --prompt, --system, --depth N, --threads N, --forced, --tool-index, --max.

## Caveats
- Embeddings are NOT a trained sentence-embedding model. Per the porting doc (documentation, not independently tested by me): with no embedding head `needle_embed` returns the confidence head's probe pool as a unit-norm 3072-float vector, "not a contrastively trained embedding" but a usable similarity signal. I did NOT evaluate retrieval quality (LIKELY weak vs Token Factory embeddings; UNVERIFIED). 3072 dims also exceeds typical index sizes.
- More than 5 tools: python wrapper retrieves top 5 (docs); accuracy degrades past ~two dozen tools (docs).
- Telemetry is on by default in the binary (disabled via env vars above).
- Latency ~0.3 s/call is for a short 3-tool prompt; not tested with long prompts or concurrent builds (other builds were running, so numbers may be pessimistic).
- Engine is closed prebuilt binary downloaded from HF (license of model Apache-2.0 per task; not re-verified by me).
