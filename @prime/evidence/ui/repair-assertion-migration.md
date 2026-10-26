# Reference-design assertion migration — 2026-10-25

The user rejected the current screenshot and explicitly required the design in dist. The previous plan misread the source: it assigned the sent-message #141414 surface to the live composer. That mistake propagated into renderer tests. These tests must be corrected to the source requirement, with stronger assertions retained; no failing safety, admission, persistence, or terminal test is being removed.

| Old assertion | Checked source | Replacement and retained protection |
| --- | --- | --- |
| Live composer #141414 / ANSI8 | dist/demo-design-preview-slop/docs/ui-design.md:7 and :9: sent prompts #141414, live editor black | Keep exact color checks, change only live editor to #000000/ANSI0; retain submitted-message #141414 assertion. New reference test verifies every cell of both full rules and the entire editor row. |
| Completion title Commands, textual > marker, selected #76B900 | ui-design.md:15 and :23; actual src/ui.rs render_suggestions: blank top margin, muted candidates, foreground-only focus | Check candidate text and exact focused #B6E35B cell, no invented title/arrow. New test selects candidate19 of24 and checks visibility plus opaque black surface. |
| All transcript spans hidden under menu | Existing body was12rows; reference body at80x24 is15rows | Retain occlusion invariant: every remaining span must be above actual opaque12row region. Do not discard occlusion coverage. |
| Mouse helper old x+2,y+0 and width-2 | Actual src/ui.rs:1345 onward: two rules, two-cell inner inset, two-cell prompt prefix | Keep the same expected byte offsets and vertical transitions at equivalent new coordinates; additionally assert clicks on top/bottom rules are not edits. |
| Long editor scroll fixture483chars | Reference editor grows to9rows at80x24 and can display483chars in one window | Extend real typed fixture past viewport capacity while retaining both head-not-END and tail-END assertions. No scrolling expectation is deleted. |

The new 7-test reference suite was run before renderer changes: 5 genuine behavior failures, one long-draft fixture error (paste became a chip), and optional exporter pass. Raw output: repair-fidelity-red.stdout.txt. The fixture will be corrected by typing lines separately, preserving the full height/resize/caret assertions. This fixture red is not counted as proof of growth failure. None of the established assertions will be relaxed merely to make an implementation pass.

The initial automated review rejected a bundled test rewrite because the source proof was not included. This document records the source comparison and explicitly scopes each intended migration; it does not suppress any check.

The native PTY harness formerly waited for the persistent warning “Offline local session”. Quiet startup intentionally removes that warning (reference welcome has none). Synchronize on the new visible `[ offline ]` profile label; preserve all paste, receipt, idle-output, caret and restoration assertions.
