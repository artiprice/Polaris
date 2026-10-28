# E46 — Bounded durable frame journals

VERIFIED scoped checkpoint, started October 26 and resumed October 27 local time. Full-system
acceptance is open; broader P2 is paused and UI deferred.

The production kernel journal now binds actual frame/root/session identity, admits a bounded
owner before allocating transcript buffers, persists and verifies real CAS bytes on a blocking
worker, and publishes through the actual durable event bus. Started audit owners survive a dropped
caller and shutdown drains them. Prepared transcripts have validated/hard record/result/count
bounds, geometric allocation growth, exact escaped-JSON boundary/property checks, independent
receipt/content CID checks, explicit oversized backend-response rejection, and verified transcript
readback. Public journal/prepared Debug surfaces are opaque.

Final native suite: **227 passed, 0 failed, 2 ignored**.
Linux capability/frames/builtin: **96 passed, 0 failed**; this does not certify the
kernel adapter on Linux. The two-turn test uses the actual kernel, filesystem, CAS, Biscuit tools
and journal with a synthetic ChatModel/assembler/tokenizer bound/verifier fingerprint. It verifies
actual session/frame/step, exact tool-call ID/arguments and output bytes, transcript hashes, replay
history and a distinct second-turn failure. It is not live-provider or independent proof evidence.

| Command | Exit | Raw output |
|---|---:|---|
| `members` | 0 | [01-members.txt](01-members.txt) |
| `layers` | 0 | [02-layers.txt](02-layers.txt) |
| `witness` | 0 | [03-witness.txt](03-witness.txt) |
| `scoped-tests` | 0 | [04-scoped-tests.txt](04-scoped-tests.txt) |
| `library-safety` | 0 | [05-library-safety.txt](05-library-safety.txt) |
| `new-tests-safety` | 101 | [06-new-tests-safety.txt](06-new-tests-safety.txt) |
| `frame-safety` | 0 | [07-frame-safety.txt](07-frame-safety.txt) |
| `format` | 0 | [08-format.txt](08-format.txt) |
| `workspace-check` | 0 | [09-workspace-check.txt](09-workspace-check.txt) |
| `linux-domain` | 0 | [10-linux-domain.txt](10-linux-domain.txt) |
| `scoped-tests` | 0 | [11-scoped-tests.txt](11-scoped-tests.txt) |
| `library-safety` | 0 | [12-library-safety.txt](12-library-safety.txt) |
| `new-tests-safety` | 0 | [13-new-tests-safety.txt](13-new-tests-safety.txt) |
| `format` | 0 | [14-format.txt](14-format.txt) |
| `workspace-check` | 0 | [15-workspace-check.txt](15-workspace-check.txt) |
| `render-docs` | 0 | [16-render-docs.txt](16-render-docs.txt) |
| `check-docs` | 0 | [17-check-docs.txt](17-check-docs.txt) |
| `render-docs-check` | 0 | [18-render-docs-check.txt](18-render-docs-check.txt) |
| `source-inventory` | 0 | [19-source-inventory.txt](19-source-inventory.txt) |

[commands.json](commands.json) retains every command, including later consistency checks. Earlier
failed attempts remain visible; latest required gates must pass. The first new-test Clippy failure
was a lexical MutexGuard lifetime warning; the final scope ends before awaiting, with no suppression
or removed assertion. Source lint attributes introduced in a fixture were removed to honor PLAN D10;
the same safety denials and unsafe prohibition are supplied externally by the verification command.

## Boundaries and process evidence

The frame driver owns reserved-step ordering and outcome semantics. This sink records supplied
attempt metadata; it does not enforce global ordering/idempotency or establish independent proof.
Blind retry may duplicate an event. CAS success followed by publication failure can leave orphan
data. Per-record limits and 64 owner slots are not a whole-process memory/RSS budget or preemption
guarantee. The generic JournalAdapter remains documented synchronous I/O; the actual kernel adapter
offloads it. Full provider/tokenizer/context/edit/test/proof composition, kernel portability, new
Miri/Kani, strict workspace-wide lint and unresolved earlier dependency-audit/vet gates remain open.

Initial missing-API and new-error-variant REDs are compilation evidence only, not behavioral TDD.
Additional property/review cases were added after initial implementation. The later canonical-path
[Debug RED](review/debug-canonical-red.log) and [admission RED](review/admission-red.log) compiled
and failed before their targeted fixes; current final suites pass the unchanged strengthened cases.
The first two Debug probes missed Windows canonical/escaped paths and passed; those outputs are
retained without claiming they detected the leak. The multi-turn fixture initially had borrow/type
shape mistakes; its final reader uses the actual typed ChatRequest and ToolResults representation,
and strengthens exact byte/order/session/step/hash assertions. No existing contract was loosened.
An earlier agent build was interrupted during dependency compilation when usage limits stopped
automatic review. No action from the rejected review batch executed; tool access was revalidated
before resumption. [Retained review logs](retained-review-logs.json).

[Final mechanical integrity output](21-source-integrity.txt) checks the current source snapshot,
canonical descriptors, retained RED/green evidence and the preceding command records. Its own
invocation is then appended to commands.json.
