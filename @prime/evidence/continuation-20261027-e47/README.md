# E47 — Bound prompt assembly and tokenizer transport

VERIFIED scoped checkpoint. Full-system acceptance remains open; P2 is paused and UI deferred.

The production KernelPromptAssembler now runs the actual bounded PII vault before any counting
request, takes schemas/effects from the initialized catalog, filters by the real frame role, and
counts complete sanitized requests through the model's actual tokenizer contract. The explicit
VllmChatClient uses one configured chat HTTP owner and the same model/messages/tools/reasoning
wire projection for same-origin `/tokenize`. Deployment support must be verified; this does not
assert Nebius offers the endpoint. Empty tool arrays are omitted, input/encoded/response sizes
are bounded, and counts/capacity/token IDs are validated without heuristic fallback. The caller's
JSON response budget must accommodate the required complete token-ID vector, not just token strings.

The assembler's `run_frame` checks actual frame/root/role/model/effort/token selection and uses
the same tokenizer/chat owner and bound tool runner. Generic low-level frame injection remains
trusted caller composition. Context/scratchpad inputs are explicit snapshots, not an invented
resolver for opaque frame CIDs. Whole tool exchanges are retained or archived atomically as groups;
the archive notice is recounted exactly and visibility additionally requires full-CID CAS Read
authority. One cumulative archive-byte budget covers all candidate writes. Per-count timeout and
encoded context construction bounds fail closed.

Final native seven-package regression: **288 passed, 0 failed, 3 ignored**. Linux SDK/capability/
frames/context/builtin/provider domain: **176 passed, 0 failed, 1 ignored**. Linux does not certify the kernel adapter.
The real loopback HTTP/frame test uses production masking, adapters, driver, CAS and durable event
bus. Counted/dispatched wire identity, masked email data and durable request/event identities are
checked. The server's token IDs/count and verifier fingerprint are synthetic fixtures; no live model,
actual tokenizer execution or independent proof is established by them.

| Command | Exit | Raw output |
|---|---:|---|
| `assembler-red` | 101 | [01-assembler-red.txt](01-assembler-red.txt) |
| `assembler-first-build` | 101 | [02-assembler-first-build.txt](02-assembler-first-build.txt) |
| `assembler-tests` | 101 | [03-assembler-tests.txt](03-assembler-tests.txt) |
| `assembler-tests` | 0 | [04-assembler-tests.txt](04-assembler-tests.txt) |
| `kernel-safety` | 101 | [05-kernel-safety.txt](05-kernel-safety.txt) |
| `kernel-safety` | 0 | [06-kernel-safety.txt](06-kernel-safety.txt) |
| `binding-deadline-red` | 101 | [07-binding-deadline-red.txt](07-binding-deadline-red.txt) |
| `deadline-context-behavior-red` | 101 | [08-deadline-context-behavior-red.txt](08-deadline-context-behavior-red.txt) |
| `cumulative-archive-behavior-red` | 101 | [09-cumulative-archive-behavior-red.txt](09-cumulative-archive-behavior-red.txt) |
| `bound-assembly-green` | 0 | [10-bound-assembly-green.txt](10-bound-assembly-green.txt) |
| `native-regression` | 0 | [11-native-regression.txt](11-native-regression.txt) |
| `linux-domain` | 0 | [12-linux-domain.txt](12-linux-domain.txt) |
| `kernel-safety` | 0 | [gates/01-kernel-safety.txt](gates/01-kernel-safety.txt) |
| `provider-safety` | 0 | [gates/02-provider-safety.txt](gates/02-provider-safety.txt) |
| `sdk-safety` | 0 | [gates/03-sdk-safety.txt](gates/03-sdk-safety.txt) |
| `frame-context-safety` | 0 | [gates/04-frame-context-safety.txt](gates/04-frame-context-safety.txt) |
| `format` | 0 | [gates/05-format.txt](gates/05-format.txt) |
| `workspace-check` | 0 | [gates/06-workspace-check.txt](gates/06-workspace-check.txt) |
| `members` | 0 | [gates/07-members.txt](gates/07-members.txt) |
| `layers` | 0 | [gates/08-layers.txt](gates/08-layers.txt) |
| `witness` | 0 | [gates/09-witness.txt](gates/09-witness.txt) |
| `render-docs` | 0 | [22-render-docs.txt](22-render-docs.txt) |
| `check-docs` | 0 | [23-check-docs.txt](23-check-docs.txt) |
| `render-docs-check` | 0 | [24-render-docs-check.txt](24-render-docs-check.txt) |
| `source-inventory` | 0 | [25-source-inventory.txt](25-source-inventory.txt) |

`commands.json` remains authoritative if later integrity commands extend this table. Earlier
failed gates are retained and final gates must pass. Initial missing-API REDs are compilation
failures only. The later timeout, encoded-context and cumulative archive regressions compiled
and failed before targeted fixes; final assertions remain. The new archive test parser was corrected
from an assumed marker to the actual SDK `§ref:` form without removing retrieval/content assertions.
Extra SDK dispatch/property and review cases followed implementation; no universal TDD claim is made.
Descriptors were normalized to the workspace registry and all listed test names resolve to actual
definitions. One unimplemented descriptor-only field-privacy test entry was removed rather than
reported as executed; field visibility was inspected in source. No executable check was removed.

The read-only review's claim of a 32-KiB *default* JSON limit was rejected: that value is a private
test config; ClientConfig has explicit caller-supplied limits. The real response bound remains
enforced and its token-array sizing requirement is documented. Model-owner mismatch and archive
write amplification findings led to bound execution and cumulative-write limits.

The first final-integrity helper incorrectly compared implemented manifests with the skeleton
template and failed on polaris-cli. The generator explicitly preserves implemented packages;
the corrected helper follows that same ownership rule, checks generated root/skeleton manifests,
and retains implemented-package hashes and actual layer/member edge checks. The initial failure
is retained in commands.json; no manifest or executable test assertion was changed for it.

The tokenizer transport schema was checked against upstream vLLM's
[protocol](https://raw.githubusercontent.com/vllm-project/vllm/main/vllm/entrypoints/serve/tokenize/protocol.py),
[router](https://raw.githubusercontent.com/vllm-project/vllm/main/vllm/entrypoints/serve/tokenize/api_router.py)
and [serving code](https://raw.githubusercontent.com/vllm-project/vllm/main/vllm/entrypoints/serve/tokenize/serving.py).

Remaining limits: DFA/entropy masking has degraded semantic coverage, not complete DLP. Archives
preserve sanitized model-visible history; failed fitting/cancellation may leave bounded orphans.
On the final allowed count the fitter may omit all old history; it does not promise maximal
retention. Owner slots/per-count deadlines do not establish global RSS, tokenizer billing/admission,
OS-I/O preemption or full lifecycle acceptance. Live tokenizer/provider, complete edit/test/proof,
kernel Linux portability, new Miri/Kani, strict workspace lint and prior advisory/vet gates remain
open. The existing unsafe allocation-instrumentation test prevents a universal all-target unsafe
ban; production provider plus new targets pass scoped bans. Retained review logs include failed
broader lint attempts, with no new suppressions or dependency exemptions.
