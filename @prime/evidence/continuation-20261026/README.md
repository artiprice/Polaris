# Non-UI continuation — 2026-10-26

This is a scoped implementation checkpoint, not full phase or production-release acceptance.
The working tree also contains independent UI changes, which this continuation preserves.

Implemented surfaces include concrete Tavily/SearXNG providers, bounded web extraction,
multi-source search orchestration, sandbox stdio resource contracts and the real OpenShell
adapter, RMCP protocol integration, durable dropped-chunk recovery, and research execution over
capability-checked tools, VCS receipts and CAS. Daemon composition and final combined consistency
results belong in the command record below.

The research acceptance uses actual local PowerShell processes and temporary Git/CAS state.
It does not demonstrate sandbox isolation. The MCP stdio protocol fixture also uses a test-only
local process; production launch is exclusively through the SDK interactive sandbox boundary.
No live Tavily/SearXNG service or sandbox-backed MCP acceptance is claimed.

## Current-tree command results

Exact commands, timestamps, elapsed times and exit statuses are in [commands.json](commands.json).

| Check | Observed result | Scope |
|---|---|---|
| Workspace members / dependency layers | PASS | 93 packages; actual normal edges match ownership metadata |
| Approval-witness construction policy | PASS | Zero violations; checker unchanged |
| Workspace all-target check | PASS | Existing CLI dead-code warnings remain |
| Scoped package tests | PASS | 157 passed, zero failed, one ignored live-gateway test |
| Full SDK package tests | FAIL | 53 passed, two unchanged schema-boundary failures |
| Scoped all-target Clippy, `--no-deps -D warnings` | PASS | Changed packages plus SDK; no clean dependency-wide Clippy claim |
| Scoped formatting and tracked diff whitespace | PASS | Exact paths/arguments are in the command record |
| Dependency advisories | FAIL | Four unmaintained dependencies after the ctutils fix |
| Cargo vet | FAIL | 724 unvetted dependencies |

The ignored test is `live_gateway_create_exec_sync_destroy`, which requires an explicitly
configured OpenShell gateway and mTLS certificates. The local gateway probe failed with
connection refused; this test was not silently counted as passing.

## Evidence and limits

Raw logs are copied here from the task's `@prime/tmp/20261026` directory, with command results
listed in `commands.json`. Historical outputs retain
their original names; a failing output is not replaced with a success claim.

- The full SDK run reports two failures: `blast_alert_requires_schema_three` and
  `versioned_constructor_validates_schema`. Their fixed schema-version assumptions conflict
  with the additive schema-4 event contract. Neither test was weakened or removed.
- The original approval-cache tests were moved to verifier integration tests and now obtain
  witnesses from the real `LocalDirectiveVerifier`. Their workspace/source/expiry/future
  assertions remain. The source-policy checker itself is unchanged.
- Dependency audit initially reported vulnerable `ctutils 0.4.3`, plus four unmaintained
  dependencies. The exact update to `0.4.4` and the subsequent audit are retained separately:
  the vulnerability finding is absent after the update, but all four unmaintained findings
  remain and the advisory gate fails. No exemptions were introduced.
- `cargo vet --locked` reports **724 unvetted dependencies** in the current expanded graph.
  This is an open release gate, not a successful vetting result.
- The OpenShell CLI's configured local gateway reported connection refused. No gateway or
  Docker service was restarted, and no reference VM restart was performed.
- Search race deadlines bound selection and validation; cooperative source teardown is
  drained afterwards. A blocking third-party adapter can delay teardown.
- RMCP schema normalization rejects unsupported assertion semantics and references instead
  of silently weakening validation. This is an interoperability limit, not full JSON Schema
  coverage.
- Research model usage remains supplied by the caller; there is no independently validated
  live provider cost/token telemetry in this acceptance.
- The research run lease coordinates `ResearchLoop` instances. Trusted low-level store and VCS
  callers must still coordinate with the run owner; per-write locking does not authorize a
  caller to replace active orchestration state.
- The daemon's default P1 kernel profile supports authenticated health and event replay. It
  explicitly denies unavailable mutation/terminal operations. A generic embedding seam permits
  richer real kernel/terminal profiles; the default executable is not a full remote agent loop.
  Daemon-side token issuance, persistent revocation and dedicated single-instance locking remain
  open. The new daemon's remote mTLS configuration has no fresh live remote deployment evidence.
- The existing conservative AST verifier can request human approval when unsafe-analysis
  coverage is incomplete. This continuation does not convert that uncertainty into approval.

Formal verification, broader crash/contention/adversarial matrices, calibrated S1 inference,
real model/tool/runtime composition, live keyed services, and all-phase exit gates remain open.
Phase 2's broader completion work remains paused; the stdio adapter is required by P7 MCP.
