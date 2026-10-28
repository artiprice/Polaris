# E40 non-UI continuation evidence

VERIFIED current-tree checkpoint; complete phases and production release remain open.

| Command group | Exit | Raw output |
|---|---:|---|
| `vendored-docx` | 0 | [vendored-docx.txt](vendored-docx.txt) |
| `vendored-docx-tests` | 0 | [vendored-docx-tests.txt](vendored-docx-tests.txt) |
| `vendored-biscuit` | 0 | [vendored-biscuit.txt](vendored-biscuit.txt) |
| `zip-graph` | 0 | [zip-graph.txt](zip-graph.txt) |
| `summary` | 0 | [summary.txt](summary.txt) |
| `members` | 0 | [members.txt](members.txt) |
| `layers` | 0 | [layers.txt](layers.txt) |
| `witness` | 0 | [witness.txt](witness.txt) |
| `workspace-check` | 0 | [workspace-check.txt](workspace-check.txt) |
| `scoped-tests` | 0 | [scoped-tests.txt](scoped-tests.txt) |
| `sdk-full-tests` | 101 | [sdk-full-tests.txt](sdk-full-tests.txt) |
| `real-model` | 0 | [real-model.txt](real-model.txt) |
| `real-worker-process` | 0 | [real-worker-process.txt](real-worker-process.txt) |
| `scoped-clippy` | 0 | [scoped-clippy.txt](scoped-clippy.txt) |
| `scoped-format` | 0 | [scoped-format.txt](scoped-format.txt) |
| `advisories` | 1 | [advisories.txt](advisories.txt) |
| `vet-format` | 0 | [vet-format.txt](vet-format.txt) |
| `vet` | 4294967295 | [vet.txt](vet.txt) |

[commands.json](commands.json) preserves exact argv, timestamps, exits and the three explicit
non-secret environment overrides for the real-artifact checks. [Artifact hashes](local-model-artifacts.json)
identify the installed files actually available for those checks; no model is downloaded here.

The scoped suite reports **119 passed, 0 failed, 2 opt-in tests ignored**. The two ignored tests
were then explicitly executed: one direct model call and one actual authenticated Windows child
worker call through S1Client::connect_named_pipe. These are integration cases, not a parity or
calibration result. The child receives a generated credential through stdin, kept open until EOF
shutdown; token bytes never enter argv, environment, test output or this evidence directory.

The full SDK suite has **54 passed, 2 failed**: `blast_alert_requires_schema_three` and
`session_lifecycle::versioned_constructor_validates_schema`. Their historical schema-boundary
assertions remain unchanged. The newly added exhaustive hexadecimal test passes. No assertion
was removed or loosened. Proto mutation fuzz tests use the same positions with checked remainder;
wire schema assertions use equivalent From conversions. The generator guard changes exactly the
three known unimplemented-status casts and fails closed on count or primitive-cast drift.

Safety Clippy covers all targets of six packages, warnings plus unwrap/expect/panic, numeric
casts, indexing and unchecked arithmetic. This is a scoped result, not full-workspace strict
Clippy or a transitive dependency safety audit. Workspace compilation retains pre-existing UI
dead-code warnings. The advisory and vet exits above are preserved as failures, without exemptions.

The [initial ZIP8 checkpoint](initial-zip8/README.md) preserves the dependency-security RED.
The final [resolved ZIP graph](zip-graph.txt) uses 9.1.1. The published-source DOCX backport
is checked by the [provenance command](vendored-docx.txt) and [tamper suite](vendored-docx-tests.txt),
retaining the exact upstream MIT license. Only the ZIP requirements and an upstream test
fixture lifetime change differ from the published Rust source. No advisory exemption was added.
The adversarial Python tests were authored with the checker; no pre-implementation RED was
captured for that tool. This is explicitly distinct from the retained behavioral RED/GREEN
logs for archive NUL names, busy-pipe retries and runtime waiting. S1 cancellation/lock RED
and GREEN are preserved as annotated tool-transcript copies, not original redirected process
output. The bounded-transport helper has post-fix GREEN coverage only, without a separate RED.

Shutdown RPCs signal transport before awaiting the owner, and native drain bounds include
lock contention. Transport graceful waiting is capped at 120 seconds; final runtime waiting
is capped at 30 seconds. Started blocking/native work may remain alive after a wait expires;
these bounds do not establish preemption or release its held resources.

Limits: caller-to-worker bearer auth requires the parent to establish endpoint ownership; it does
not prove mutual server identity or the Windows default DACL. Automatic supervisor credential
provisioning, full kernel assembly, idle/RSS policy, calibrated model parity/PII-NER/CUDA, OCR/nano,
preemptive parser memory/time enforcement, real sanitizer sandbox workloads, broad fuzz/formal and
release gates remain unfinished. The SDK Document loses typed locations; the real worker CAS
record preserves the richer local locations, tagged untrusted. Canceled native inference cannot
be preempted; capacity remains owned until it ends and the process supervisor must enforce limits.

Earlier [E39 evidence](../continuation-20261026/README.md) is retained separately. All other phase
estimates keep their earlier audit basis. This continuation changes no UI implementation.

Final [consistency commands](consistency-commands.json) retain phase-table, percentage, inventory and whitespace checks. [Structural output](structural-consistency.txt) and [final input hashes](verification-inputs.json) pin the checkpoint inputs. [Linux compilation output](worker-client-linux-final.txt) is compilation evidence only.

The final cached-RustSec advisory check has six unmaintained-crate failures and no ZIP vulnerability finding. Cargo-vet reports 804 unvetted dependencies. The first vet invocation stopped on newline formatting; [that failure](vet-before-format.txt) and [its command](vet-before-format-command.json) are retained. `cargo vet fmt` then normalized formatting with parsed policy equality asserted; every exemption and audit requirement was preserved.
