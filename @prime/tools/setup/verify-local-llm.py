"""P0-ENV-6: verify a local OpenAI-compatible endpoint (Ollama) for the P3 walking skeleton.

Checks: /v1/models, streaming SSE chat completion, and a tool-call round in the
OpenAI format. Standard library only. Usage:
    python -I @prime/tools/setup/verify-local-llm.py [BASE_URL] [MODEL] [--require-tools]
Defaults: http://localhost:11434/v1  nemotron-mini
"""

import json
import sys
import time
import urllib.request

REQUIRE_TOOLS = "--require-tools" in sys.argv
_POS = [a for a in sys.argv[1:] if a != "--require-tools"]
BASE = _POS[0] if len(_POS) > 0 else "http://localhost:11434/v1"
MODEL = _POS[1] if len(_POS) > 1 else "nemotron-mini"

TOOLS = [
    {
        "type": "function",
        "function": {
            "name": "run_tests",
            "description": "Run the test suite of a Rust crate and return the summary.",
            "parameters": {
                "type": "object",
                "properties": {"crate_name": {"type": "string", "description": "Cargo package name"}},
                "required": ["crate_name"],
            },
        },
    }
]


def post(path: str, body: dict) -> urllib.request.addinfourl:
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json", "Authorization": "Bearer local"},
    )
    return urllib.request.urlopen(req, timeout=300)


def main() -> int:
    ok = True
    with urllib.request.urlopen(BASE + "/models", timeout=10) as r:
        models = [m["id"] for m in json.load(r)["data"]]
    print(f"models: {models}")

    # 1) streaming text completion (SSE)
    t0 = time.perf_counter()
    first = None
    chunks = 0
    text = []
    with post("/chat/completions", {
        "model": MODEL, "stream": True, "max_tokens": 64,
        "messages": [{"role": "user", "content": "Reply with exactly: polaris ready"}],
    }) as r:
        for raw in r:
            line = raw.decode().strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                break
            delta = json.loads(payload)["choices"][0]["delta"].get("content") or ""
            if delta and first is None:
                first = time.perf_counter() - t0
            chunks += 1
            text.append(delta)
    total = time.perf_counter() - t0
    print(f"stream: chunks={chunks} ttft_ms={int((first or 0) * 1000)} total_ms={int(total * 1000)} text={''.join(text).strip()!r}")
    ok &= chunks > 1

    # 2) tool call (non-streaming, OpenAI tool_calls format)
    t0 = time.perf_counter()
    with post("/chat/completions", {
        "model": MODEL, "stream": False, "tools": TOOLS, "max_tokens": 128,
        "messages": [{"role": "user", "content": "Run the tests for the crate polaris-kernel."}],
    }) as r:
        msg = json.load(r)["choices"][0]["message"]
    calls = msg.get("tool_calls") or []
    print(f"tool_call: ms={int((time.perf_counter() - t0) * 1000)} calls={json.dumps(calls)[:300]} content={str(msg.get('content'))[:120]!r}")
    if calls:
        fn = calls[0]["function"]
        args = json.loads(fn["arguments"]) if isinstance(fn["arguments"], str) else fn["arguments"]
        print(f"tool_call parsed: name={fn['name']} args={args}")
    else:
        print("tool_call: model returned no tool_calls (model capability, not a client failure)")

    # Two separate verdicts: the endpoint (what P0-ENV-6 needs) and the model's tool calling (flaky for
    # small local models). `--require-tools` makes a missing tool call fail the run.
    print(f"RESULT: endpoint {'PASS' if ok else 'FAIL'}; tool_calls {'PRESENT' if calls else 'ABSENT'}")
    if REQUIRE_TOOLS and not calls:
        return 1
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
