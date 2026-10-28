# M3 local model and exact tokenization audit (2026-10-27)

## Verdict

VERIFIED: a local Ollama model endpoint is running and lists `nemotron-mini:latest` (`ed76ab18784f`, GGUF, 4.2B, Q4_K_M, 4096 context). The model metadata advertises `completion` and `tools` capabilities. This proves availability, not dependable tool-call behavior or the M3 plan/edit/test/proof run.

VERIFIED: current repository code provides exact chat counting only through the Token Factory `VllmChatClient` `/tokenize` contract. That route is not interchangeable with Ollama. Ollama `/api/chat` returns `prompt_eval_count` after it has processed a chat request; that is not a preflight tokenizer service and no inference was requested here.

VERIFIED from current llama.cpp primary source: a strong adapter path exists. `/v1/chat/completions` and `/v1/chat/completions/input_tokens` are registered separately; both use `oaicompat_chat_params_parse` on the provided chat request and model metadata. The input-token handler counts the resulting rendered prompt through the loaded model tokenizer, including the same chat template and tool definitions, without generation. A pinned server release must still be exercised with parity regressions before relying on it. No new adapter implementation or external binary/model download was made during this audit.

## Local evidence

Commands and observed output:

```text
Get-Command ollama,llama-server,vllm -ErrorAction SilentlyContinue | Select-Object Name,Source
ollama.exe C:\Users\Admin\AppData\Local\Programs\Ollama\ollama.exe

ollama --version
ollama version is 0.40.11

ollama list
NAME                    ID              SIZE      MODIFIED
nemotron-mini:latest    ed76ab18784f    2.7 GB    6 days ago

GET http://127.0.0.1:11434/api/version
{"version":"0.40.11"}

GET http://127.0.0.1:11434/v1/models
{"object":"list","data":[{"id":"nemotron-mini:latest","object":"model","created":1792504424,"owned_by":"library"}]}
```

Bounded GET `/api/tags` returned one model:

```json
{
  "name": "nemotron-mini:latest",
  "digest": "ed76ab18784f5a01c9ec5b3c250e964d4f9c7a983e59ba041bb995f0fe2e8fb3",
  "size": 2697402546,
  "details": {"format":"gguf","family":"nemotron","parameter_size":"4.2B","quantization_level":"Q4_K_M","context_length":4096},
  "capabilities":["completion","tools"]
}
```

`ollama show --modelfile nemotron-mini:latest` identifies the local weight blob as `C:\Users\Admin\.ollama\models\blobs\sha256-1dcbd925825b41744ddc2fc3047db6d3ad0aecf8d336f4fadc044eaaf79779d5` (2,697,386,784 bytes). It shows a custom Nemotron template with `<extra_id_0>`, `<extra_id_1>`, `<tool>`, and `<toolcall>` markers. Reusing this weight in another server needs a template/tool-parser equivalence check.

`Get-Command llama-server,vllm -ErrorAction SilentlyContinue` returned no executable. The known Hugging Face `models--Qwen--Qwen2.5-1.5B-Instruct-GGUF` cache contains only a 0-byte `.incomplete` blob under its `blobs` directory. It is not a usable local model cache.

No request to generate text, emit tool calls, or exercise actual mutation/sandbox behavior was sent.

## Source contract

- llama.cpp route registration, [server.cpp](https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/server.cpp), lines 164–190: both `/v1/chat/completions` and `/v1/chat/completions/input_tokens` are registered.
- llama.cpp handler source, [server-context.cpp](https://raw.githubusercontent.com/ggml-org/llama.cpp/master/tools/server/server-context.cpp), lines 5361–5382 and 6394–6440: chat and count paths both parse the OpenAI chat body with `oaicompat_chat_params_parse(vocab, body, meta->chat_params, files, session)`; the count route tokenizes its rendered prompt with `tokenize_mixed` and returns `input_tokens` without queuing generation.
- Official [llama.cpp Windows CPU x64 release asset](https://github.com/ggml-org/llama.cpp/releases/download/b11804/llama-b11804-bin-win-cpu-x64.zip), release `b11804`; official attestation records SHA-256 `3a3e417a0816329829081bdd443e3045ff3b2dbe866fcef89a1d919e941ce23f`. Archive byte size has not been checked, and it was not downloaded.
- Official [Ollama API source documentation](https://github.com/ollama/ollama/blob/main/docs/api.md): documents chat and reports `prompt_eval_count` in the completed response, including in the tool-call example. No external token-count route is documented there.

## Next concrete implementation route

Use a pinned llama.cpp Windows server with one model loaded, and extend the existing OpenAI-compatible client adapter so `count_prompt` posts the exact same serialized `model`, `messages`, `tools`, and related request fields to `/v1/chat/completions/input_tokens`, while dispatch uses those same fields against `/v1/chat/completions`. Strictly parse `input_tokens`, validate `/v1/models` identity at startup, reject unsupported multimodal fields, and add an on-server parity regression for tool schemas, assistant/tool history, BOS/template behavior, and response counts. The server binary and model artifacts should be kept under workspace-local runtime/cache paths; their size and hashes must be recorded before any download.

Using the existing Nemotron weight would avoid another model download, but its Ollama-specific chat-template and tool-parser mapping into llama.cpp is not verified. The official Qwen GGUF repository lists a Q4_K_M artifact around 1.12 GB; its exact hash and current archive size were not verified here, so that is only a candidate, not a download plan.

## Scope limits

No code, adapter, descriptor, model, or binary was changed or downloaded. No model inference, reliable tool-call rate, sandbox execution, proof-of-done, or P3 acceptance was established. P2 remains paused, and the local M3 live plan/edit/sandbox-test/proof requirement remains incomplete until the same actual model/server stack is exercised through the production frame driver and proof gate.

## Second pass — pinned local runtime and actual inference (2026-10-27)

The preceding audit records its then-current state and must be read as a first-pass snapshot. Since that pass, the authorized bounded setup changed the local state: downloaded and SHA-256-verified the official llama.cpp b11804 Windows CPU x64 archive, fetched the pinned public Qwen2.5-1.5B-Instruct Q4_K_M GGUF, and ran actual local inference. The two workspace artifact paths are ignored by `.gitignore`; this README and the accompanying `qwen-smoke-manifest.json` are under the ignored evidence/log directory.

VERIFIED: b11804 executable identifies as build 11804 / commit `c7fc50941`; archive SHA-256 is `3a3e417a0816329829081bdd443e3045ff3b2dbe866fcef89a1d919e941ce23f`. Qwen file is 1,117,320,736 bytes at pinned repository revision `91cad51170dc346986eccefdc2dd33a9da36ead9`, SHA-256 `6a1a2eb6d15622bf3c96857206351ba97e1af16c30d7a74ee38970e434e9407e`.

VERIFIED: hidden loopback server PID 15100 (`C:\\mDev\\Polaris\\@prime\\tools\\runtime\\llama-b11804\\llama-server.exe`) is still running at `127.0.0.1:18931`, with CPU threads/batch threads 2, context 4096, one slot, Jinja template enabled, and alias `qwen2.5-1.5b-instruct`. `/props` reported `supports_tools=true` and `supports_tool_calls=true`. Earlier Nemotron check on the same pinned server reported `supports_tools=true`, `supports_tool_calls=false`, so it was not used for M3.

VERIFIED: three actual local chat requests with a structured function schema were each first POSTed to `/v1/chat/completions/input_tokens` and then dispatched to `/v1/chat/completions` using byte-identical request bodies (see per-trial raw JSON and hashes in this folder). Each count returned `input_tokens=207`; each completion reported `usage.prompt_tokens=207`, `completion_tokens=25`, `finish_reason=tool_calls`, and one structured `p3_local_probe` call with the matching probe id. These prove one successful tool-call generation and exact count/usage parity in three smoke trials, not tool execution, a reliability rate, or full M3.

The local inference request was an explicitly bounded function-call smoke only. It did not execute the tool, mutate files, run a sandbox plan/test, or satisfy the frame proof-of-done gate. Adapter implementation remains pending the root's E49 seal acknowledgement; subsequent adapter tests must verify the production serialized request projection and the same-body count/chat contract. The first-pass “no inference/download” statements above describe only that earlier pass and are not current-state claims.
