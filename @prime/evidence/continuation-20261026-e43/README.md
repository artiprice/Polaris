# E43 context, rate admission, release inventory and SDK schema evidence

VERIFIED scoped checkpoint; full-system assembly, phase acceptance and production release remain open.

| Command group | Exit | Raw output |
|---|---:|---|
| `members` | 0 | [members.txt](members.txt) |
| `layers` | 0 | [layers.txt](layers.txt) |
| `witness` | 0 | [witness.txt](witness.txt) |
| `workspace-check` | 0 | [workspace-check.txt](workspace-check.txt) |
| `scoped-tests` | 0 | [scoped-tests.txt](scoped-tests.txt) |
| `sdk-full-tests` | 0 | [sdk-full-tests.txt](sdk-full-tests.txt) |
| `safety-clippy` | 0 | [safety-clippy.txt](safety-clippy.txt) |
| `sdk-production-clippy` | 0 | [sdk-production-clippy.txt](sdk-production-clippy.txt) |
| `sdk-schema-tests-clippy` | 0 | [sdk-schema-tests-clippy.txt](sdk-schema-tests-clippy.txt) |
| `scoped-format` | 0 | [scoped-format.txt](scoped-format.txt) |
| `release-sbom` | 0 | [release-sbom.txt](release-sbom.txt) |
| `release-sbom-repeat` | 0 | [release-sbom-repeat.txt](release-sbom-repeat.txt) |
| `advisories` | 1 | [advisories.txt](advisories.txt) |
| `vet` | 4294967295 | [vet.txt](vet.txt) |
| `sbom-validator-build` | 0 | [sbom-validator-build.txt](sbom-validator-build.txt) |
| `sbom-schema` | 0 | [sbom-schema.txt](sbom-schema.txt) |
| `sbom-schema-negative` | 1 | [sbom-schema-negative.txt](sbom-schema-negative.txt) |
| `sbom-schema-structural-negative` | 1 | [sbom-schema-structural-negative.txt](sbom-schema-structural-negative.txt) |
| `render-docs` | 0 | [render-docs.txt](render-docs.txt) |
| `check-docs` | 0 | [check-docs.txt](check-docs.txt) |
| `render-docs-check` | 0 | [render-docs-check.txt](render-docs-check.txt) |

The five-package scoped suite reports **155 passed, 0 failed, 2 ignored**. The separate SDK suite
reports **56 passed, 0 failed** and overlaps that suite; counts must not be added. Workspace
all-target compilation, member/layer/witness checks, formatting and the recorded scoped safety
Clippy commands pass. This is not full-workspace strict Clippy, a formal proof or full acceptance.
Two ignored cases remain ignored, not implicitly certified. Exact arguments and times are in
[commands.json](commands.json). Subsequent record/inventory checks are added there as captured.

## Implemented behavior and evidence limits

P3 context assembly archives full structured omitted ChatMessage groups in bounded versioned CAS
JSON. Full-CID/short-reference/version/count/group and byte-cap checks protect typed expansion.
The archive uses borrowed serialization and bounded reads. The notice consumes prompt budget and
identifies registered polaris_expand output as untrusted history. Preserved roles do not confer
current policy authority. Twelve focused tests pass; provider dispatch composition and archive
ownership authorization must still be supplied by the composition root.

P3 RateLimitOp carries an actual DualLimiter permit with noncloneable admitted work through
bounded cancellation-aware admission, dispatch marking and usage reconciliation. Six integration
cases and a compile-fail retry contract pass. Dropping uncertain work does not refund reservation.
This is a real operator; the complete production S2 agent pipeline is not assembled by this change.

P10 cargo xtask release-sbom reads locked/offline Cargo metadata, rejects changed Cargo.lock bytes,
invalid Gregorian timestamps and incomplete/duplicate graph identities, and writes deterministic
SPDX 2.3 JSON with BLAKE3 lock provenance. Atomic no-clobber and force replacement preserve an
existing output on failed replacement, including the tested Windows sharing violation. This is not
a hostile same-user filesystem or power-failure durability proof. The inventory covers the **full
resolved Cargo workspace graph**, including development/build/platform dependencies; it is not a
target-binary bill of materials, independent licence audit or vulnerability/security certification.
Unknown and unconcluded licensing remains NOASSERTION.

The real inventory has **1027 packages, 3829 relationships**
and 94 described elements (workspace members plus the lockfile).
Two runs with the same explicit UTC input produce identical 989,120-byte files.
SHA256: `d3511cb356ce54630a9f6afd540b3a3cd366e307505cb9dffc20613433cdc02b`. [Artifact](release.spdx.json), [provenance](sbom-summary.json).
The [official SPDX schema](https://raw.githubusercontent.com/spdx/spdx-spec/v2.3/schemas/spdx-schema.json)
is retained with its digest. It only types creationInfo.created as a string; it does **not** constrain
the date format. An initial helper therefore accepted an impossible date; the original erroneous
PASS message and failed harness assertion are retained under [before-validator-review](before-validator-review/README.md).
The repaired independent helper applies the official schema plus a separate RFC3339 date-time
check: the real inventory passes, the same impossible-date fixture fails, and the wrong-type
fixture fails structural validation. This does not prove every semantic SPDX requirement.

The user explicitly approved the reviewed SDK schema-test correction before the two test files
were changed. [Exact proposed patch](review/sdk-schema-proposal/proposed-tests.diff) and the
independent-copy evidence are retained. Current maximum, minimum, future rejection, identity,
round trips and property checks remain. No production schema version changed. E41/E42 SDK failure
logs remain historical evidence and are superseded only for this current test result.

## Test-first chronology and open gates

The context byte-cap regression and SBOM invalid-calendar/duplicate/missing-graph cases have
compiled behavioral RED output. New API compile failures are compile evidence, not behavioral
RED. Other added post-implementation contracts are GREEN coverage, not invented historical TDD.
[Retained review files and hashes](retained-review-logs.json) preserve those distinctions.
[E42](../continuation-20261026-e42/README.md) separately records native idle/explicit eviction,
actual model reload and Windows/Linux model lifecycle contracts; its historical results are not
added to E43 totals or presented as a fresh all-system test run.

Cached advisories still report six unmaintained crates and cargo-vet reports **804 unvetted
dependencies**. Both gates remain failed without exemptions. Live keyed providers/cloud,
complete plan/edit/test/proof dispatch, calibrated S1/PII-NER, model pressure/admission assembly,
broader stress/formal/portability and release/installers remain unfinished. P1 historical acceptance
is preserved, broader P2 remains paused and UI remains deferred. Percentage estimates describe
scoped engineering judgment; they do not replace mandatory phase exit gates.
