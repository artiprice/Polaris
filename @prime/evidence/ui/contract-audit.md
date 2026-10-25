# CLI UI contract and implementation audit

2026-10-25 continuation of the 2026-10-24 implementation. This is source and focused-test evidence, not acceptance of an entire UI phase.

## Actual boundaries

| Concern | Implemented owner / contract | Remaining boundary |
|---|---|---|
| Admission | `OfflineSession::apply_directive` calls the real `KernelHandle` signed directive path and returns `CommandReceipt`. The composer keeps its prepared revision until acknowledgement. | No model task execution or complete stable-command retry/reconciliation workflow. No UI action grants an approval witness. |
| Replay | `KernelEvents` yields authoritative globally ordered `EventRecord`s. `SessionViews::apply_selected` validates every sequence while retaining only the active session. | Automatic UI recovery from lag, full transcript paging and persisted draft/history recovery are not complete. |
| Payloads | Adapter CAS reads are hash-checked and capped at 65,536 bytes. One consumed record and one read handle survive cancellation; completed handles are removed before errors are propagated. | Directive text only; general tool/artifact materialization and source paging are later work. Active-session-only resolution is covered by the final isolation regression. |
| Sessions | Real SQLite registry list (at most 100 rows) and exact-ID existence query; resume locks admission, and directive requests carry their originating session. | Full session search/paging and multi-client generation/reconciliation are not accepted. |
| Terminal | App-owned blocking terminal thread; safe Crossterm operations, fallible attempted-mode rollback, one staged Ratatui presenter, ordered bounded scheduler. | Direct Windows raw/mouse flag restoration, manual terminal matrix and Linux leg are not verified here. An uninterruptible OS write/read has no promised physical completion deadline. |
| Rendering | Same pure screen function drives live frames and plain/styled test dumps; responsive optional panel, bounded viewport, shared caret/mouse/vertical geometry, inert control/bidi painting. | Native selection/copy, full lexical highlighting, complete sheets, paging/cache acceptance and performance budgets remain open. |
| Human forms | Pure questionnaire: no defaults, explicit exclusive None of the above, optional notes, atomic local validation and conservative byte estimates. | No request service, authoritative serialized human protocol, durable answers, requester continuation or scoped cancellation dispatch. These belong to P3-X-5. |
| Diagnostics | New-file-only category trace, no typed/pasted text, bounded to 1 MiB; create-new and canary tests. | No claim of a completed cross-platform privacy/formal audit. |

## Resource and capability decisions

- Input queue: 1,024 ordinary events, 2 MiB retained payload, eight reserved cancellation events; batches stop at 256 events or four milliseconds. Adjacent resize/move coalescing cannot cross semantic barriers.
- Dirty-frame scheduler has a 60 fps ceiling and no periodic idle frame deadline. Repeated application frames emit zero bytes; caret-only output uses the same staged presenter.
- Live/dump viewport is checked before buffer allocation: at most 400 by 200 cells. Encoded staged frame cap is 8 MiB.
- Loaded view caps are enforced by the shared model (1,024 entries / 16 MiB conversation accounting). The renderer currently windows the latest 128 entries / 4,096 rows. Reaching a view limit is an explicit error; full paging is not implemented.
- Undo and prompt-history windows are each bounded to 256 entries and 8 MiB accounting. Large text chips share payload ownership and expand only on explicit submission. Images/native clipboard remain unavailable.
- Human presentation caps match UI_PLAN: eight questions, sixteen named options, 16 KiB per field, 256 KiB request estimate and conservative 128 KiB answer estimate. Wire encoding remains the authoritative future owner.
- Background model tasks, provider quotas/prices, native clipboard, human request counts and security approvals are not synthesized. The local profile labels those capabilities unavailable.

## Verification limits

The initial broad descriptor/red gate was not captured for every model/renderer/terminal slice. Later real behavioral reds cover navigation parsing, frame clearing, queue pressure, resume races, capability help, text-chip submission, edit limits, partial acquisition, failed payload ownership, completion wheel routing and selected-session payload isolation. Missing-method compile failures are identified separately. PTY descriptor consolidation retains the original misplaced descriptor as initial-terminal-descriptor.json; it does not invent earlier evidence.

The interrupted Node session lost its in-memory root command captures. Earlier tool-observed results are historical observations, not reconstructed raw files. Existing agent raw captures remain. Final commands are rerun and saved separately. Query-failure correlation was fixed after source review and has a regression, without a dedicated pre-fix behavioral red capture.

Full package tests retain the doctor readiness failure in this managed environment. The configured complete cargo-deny run could not obtain the advisory database lock on a read-only cache path; licenses/bans/sources passed as a separately identified subset. Broad dependency safety Clippy exposed unchanged OpenShell/log/sync findings; owned UI/application checks are reported separately. No check or advisory policy was weakened.

No fresh UI Miri/Kani/fuzz run, release performance baseline, external key integration, web welcome optimization, full web or desktop implementation is claimed.
