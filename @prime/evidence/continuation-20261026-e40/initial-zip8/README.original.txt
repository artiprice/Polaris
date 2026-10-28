# E40 non-UI continuation evidence

VERIFIED current-tree checkpoint; complete phases and production release remain open.

| Command group | Exit | Raw output |
|---|---:|---|
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
| `vet` | 4294967295 | [vet.txt](vet.txt) |

[commands.json](commands.json) preserves exact argv, timestamps, exits and the three explicit
non-secret environment overrides for the real-artifact checks. [Artifact hashes](local-model-artifacts.json)
identify the installed files actually available for those checks; no model is downloaded here.

The scoped suite reports **114 passed, 0 failed, 2 opt-in tests ignored**. The two ignored tests
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

Limits: caller-to-worker bearer auth requires the parent to establish endpoint ownership; it does
not prove mutual server identity or the Windows default DACL. Automatic supervisor credential
provisioning, full kernel assembly, idle/RSS policy, calibrated model parity/PII-NER/CUDA, OCR/nano,
preemptive parser memory/time enforcement, real sanitizer sandbox workloads, broad fuzz/formal and
release gates remain unfinished. The SDK Document loses typed locations; the real worker CAS
record preserves the richer local locations, tagged untrusted. Canceled native inference cannot
be preempted; capacity remains owned until it ends and the process supervisor must enforce limits.

Earlier [E39 evidence](../continuation-20261026/README.md) is retained separately. All other phase
estimates keep their earlier audit basis. This continuation changes no UI implementation.
