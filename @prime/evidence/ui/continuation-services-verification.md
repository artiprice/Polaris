# Continuation services verification (2026-10-25)

This records only the cold CLI service and preference slice. It does not close UI4 or M1. The JSON files contain the exact commands and exit codes; adjacent stdout/stderr files are the raw process output.

## Descriptor and red evidence

The service descriptor was present before the implementation work: [continuation-services-descriptor.json](continuation-services-descriptor.json). The config schema descriptor is [config_cli_preferences.json](../../../specs/descriptors/config_cli_preferences.json).

**Red gap:** no service-specific pre-implementation behavioral failure transcript was retained in the evidence tree. The earlier handoff describes an initial fixture assertion around `.gitignore` without a `.git` fixture, but its raw output is absent here and that was a fixture setup issue rather than a demonstrated product defect. This record does not present it as red evidence.

## Green results

- `cargo test --offline -p polaris-config` exited 0. Raw output: [stdout](continuation-services-config.stdout.txt), [stderr](continuation-services-config.stderr.txt). The CLI preference integration group passed 5 tests; all other config groups and doctests also passed.
- `cargo test --offline -p polaris-cli --lib cli_settings::tests` exited 0: 7 passed. Raw output: [stdout](continuation-services-settings.stdout.txt), [stderr](continuation-services-settings.stderr.txt). Includes memory-only refusal, worker save result/generation, cancellation and config persistence behavior.
- `cargo test --offline -p polaris-cli --lib ui_services::tests` exited 0: 5 passed. Raw output: [stdout](continuation-services-workers.stdout.txt), [stderr](continuation-services-workers.stderr.txt). Tests create isolated workspaces and do not call the host clipboard.
- `cargo test --offline -p polaris-cli --lib` exited 0: 45 passed. Raw output: [stdout](continuation-services-app-lib.stdout.txt), [stderr](continuation-services-app-lib.stderr.txt). This includes the service tests and current controller tests, but is not a full package/integration/terminal run.
- Scoped safety Clippy on all targets exited 0 for `polaris-config`, `polaris-ui-model`, `polaris-ui-cli`, and `polaris-cli` using `--no-deps` and `-D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::as_conversions -D clippy::indexing_slicing`. Raw output: [command record](continuation-services-all-targets-safety.json), [stdout](continuation-services-all-targets-safety.stdout.txt), [stderr](continuation-services-all-targets-safety.stderr.txt). Generic style warnings remain in stderr; these five safety lints did not fail.

## Boundaries not exercised

Clipboard tests reject oversized requests and inspect generation behavior without reading or changing the system clipboard. The Windows backend uses the absolute SystemRoot PowerShell executable, constant script text, UTF-8 text stdin/stdout, a 1 MiB + 1 read bound, and deadline/cancellation child kill plus wait. Platform clipboard success, unsupported-tool behavior, actual OS timeout/cancel, and native selection are **UNVERIFIED**.

The index delegates traversal and existing ignore, secret-path, symlink/reparse, containment and timeout policy to `polaris-tools-builtin`; unit coverage confirms ignored entries, `.aws`, and path-count truncation. No dedicated junction/case-alias/deleted-suggestion test was added in this slice. Settings persistence modifies only one global `[ui.cli]` field through the config owner, preserving unknown keys; runtime/session/demo overrides remain memory-only. Full settings UI retry/unsaved presentation and restart/malformed-config screen flows still need integrated verification.

UI4-3/UI4-4 and M1 remain **PARTIAL / OPEN**. Hot-path allocation profiling, visual QA, native platform matrix, live human/backend integration, WEB-OPT and desktop remain out of scope or pending.
