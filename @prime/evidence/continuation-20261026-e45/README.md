# E45 — Readonly frame dispatch over actual kernel stores

VERIFIED scoped checkpoint; all-phase production acceptance remains open.

KernelToolRunner implements the real frame ToolRunner using the kernel's actual workspace,
CAS and session-filtered history. Sealed active-frame bindings pin current/root identity, role
and token; explicit issuer verification, consumed runner-bound permits, per-resource checks and
bounded local revocation fail closed. CAS expansion requires both visible CIDs and Read scope.
Pure queries use ExecuteTool resources. Persistent CAS/history reads cannot exploit a PURE
catalog label to bypass a pure-only role. Bounded result envelopes retain truncation metadata,
are stored in actual CAS and checked against byte-exact readback and BLAKE3 content identity.
Global admission permits at most 64 blocking owners; shutdown waits for their guard release.

Final five-package native suite: **205 passed, 0 failed, 2 ignored**.
Linux capability/frames/builtin suite: **84 passed, 0 failed**. These overlapping
package totals must not be added to earlier checkpoints. The Linux result does not certify the
new kernel adapter on Linux. The two run_step integration cases use a synthetic ChatModel,
assembler/tokenizer bound and verifier fingerprint, with actual frame/Biscuit/kernel/files/CAS.
They establish child-scope dispatch and authorize-all-before-any-execution, not a live S2 loop.

| Command | Exit | Raw output |
|---|---:|---|
| `members` | 0 | [01-members.txt](01-members.txt) |
| `layers` | 0 | [02-layers.txt](02-layers.txt) |
| `witness` | 0 | [03-witness.txt](03-witness.txt) |
| `library-safety` | 0 | [04-library-safety.txt](04-library-safety.txt) |
| `new-tests-safety` | 0 | [05-new-tests-safety.txt](05-new-tests-safety.txt) |
| `linux-domain` | 0 | [06-linux-domain.txt](06-linux-domain.txt) |
| `scoped-tests` | 0 | [06-scoped-tests.txt](06-scoped-tests.txt) |
| `domain-safety` | 0 | [07-domain-safety.txt](07-domain-safety.txt) |
| `library-safety` | 0 | [08-library-safety.txt](08-library-safety.txt) |
| `new-tests-safety` | 0 | [09-new-tests-safety.txt](09-new-tests-safety.txt) |
| `format` | 0 | [10-format.txt](10-format.txt) |
| `workspace-check` | 0 | [11-workspace-check.txt](11-workspace-check.txt) |
| `scoped-tests` | 0 | [13-scoped-tests.txt](13-scoped-tests.txt) |
| `domain-safety` | 0 | [14-domain-safety.txt](14-domain-safety.txt) |
| `format` | 0 | [15-format.txt](15-format.txt) |
| `scoped-tests` | 0 | [16-scoped-tests.txt](16-scoped-tests.txt) |
| `library-safety` | 0 | [17-library-safety.txt](17-library-safety.txt) |
| `new-tests-safety` | 0 | [18-new-tests-safety.txt](18-new-tests-safety.txt) |
| `format` | 0 | [19-format.txt](19-format.txt) |
| `workspace-check` | 0 | [20-workspace-check.txt](20-workspace-check.txt) |
| `render-docs` | 0 | [21-render-docs.txt](21-render-docs.txt) |
| `check-docs` | 0 | [22-check-docs.txt](22-check-docs.txt) |
| `render-docs-check` | 0 | [23-render-docs-check.txt](23-render-docs-check.txt) |
| `source-inventory` | 0 | [24-source-inventory.txt](24-source-inventory.txt) |

[commands.json](commands.json) retains exact arguments/timestamps/exits, including later checks.
The first independent native/Linux captures overlapped and loaded the same aggregate manifest;
their observed records were reconciled using retained native metadata and the collector's printed
workspace-check row. Raw outputs were not rewritten. The collector now also retains per-command
records, and subsequent capture is sequential. Earlier attempts remain, not just latest green logs.

## Security boundaries and remaining scope

Recursive traversal observes directory names/metadata to identify resources, preauthorizes bounded
workspace-local ignore files, rejects non-regular/unresolved/oversized ignore candidates and does
not consume external parent/global ignores. Denied ignored subtrees abort the full invocation.
Trusted operator inspection retains its default ignore semantics. The filesystem must stay stable;
metadata/canonicalize/open is not race-free against hostile concurrent writers. Cancellation signals
blocking work but cannot preempt OS reads; shutdown drains ownership. Cancellation/failure during
CAS publication can leave orphan blobs. Revocation is local/in-memory; it is not durable/global.
Mutable writes/shell remain rejected. Broader P2 remains paused and UI deferred.

The primary six kernel tests and abbreviated descriptor existed before production source, but
their RED was missing-API compilation. Canonical descriptor fields, additional regressions and
loop tests were added later; no full historical behavioral-TDD claim is made. The later pure-role
regression has an actual compiled [RED](review/role-policy-red.log) and identical
[GREEN](review/role-policy-green.log) after the targeted policy fix. Other initial schema-input
and missing-test-constant mistakes are retained as development failures, not production regressions.
[Review logs](retained-review-logs.json) include the agents' scoped checks and source freezes.

Full live provider/tokenizer/context/edit/test/proof composition, complete kernel portability,
new Miri/Kani proofs, full strict workspace Clippy and broad hostile/race/load acceptance remain
unfinished. Earlier advisory/vet release failures are unresolved and were not rerun here. No
dependency, audit exemption, acceptance gate or existing test contract was weakened by E45.
The previously user-approved SDK schema patch remains covered by the full SDK regression suite.

Final integrity review found the relocated builtin descriptor still named a crate-relative source path. The [failing check](review/descriptor-path-red.txt) is retained; the descriptor now names its actual repository-relative source. No Rust behavior or test assertion changed. The mechanical source/report snapshot was refreshed after this correction.

[Final integrity output](28-source-integrity.txt) validates 1,208 source/config/document hashes,
38 actual test definitions in five canonical descriptors and the preceding 27 command records.
Its own successful invocation is retained as command record 28.
