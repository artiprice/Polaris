# CLI interactive continuation — 2026-10-25

**VERIFIED scoped checkpoint; full UI/M1 acceptance remains open.** The implementation covers CLI model, renderer, input/terminal ownership, app controller, isolated demo, text clipboard/index workers and CLI preferences. Main tracking: [UI_PLAN §1.3](../../UI_PLAN.md#13-current-interactive-continuation-checkpoint-2026-10-25), [UI_PHASES](../../UI_PHASES.md), [PHASES_STATUS §E36](../../PHASES_STATUS.md#e36--cli-editor-pointer-safe-native-input-and-isolated-demo-2026-10-25), [59-module inventory](../../UI_MODULES.csv).

## Behavior checked

- One checked Unicode textarea geometry drives paint, caret, wrapped rows, click/drag and sticky-column movement; composer selection, word/line navigation, replacement and chip preview have model/controller cases. Enter submits, Shift+Enter inserts newline, Ctrl+Enter is intentionally unbound; Ctrl+J/Alt+Enter remain fallbacks.
- Cell repaint hides the caret, paints, restores the editor position and then reveals it. Shift+Up/Down effort changes do not leave the physical caret in the footer. Identical frames still emit zero bytes.
- Typed current-state hit maps, opaque clipping, footer/completion/tool hover, completion click, editor drag and canonical single-entry Unicode transcript selection are integrated. Idle mouse movement reuses valid metadata; meaningful changes invalidate it. Cross-entry selection is not implemented.
- The safe Windows public console wrapper owns the reader, bounded polling/CSI/paste and mode snapshot. Delayed Alt numeric Unicode commits preserve `$`; C0 control keys and modifier sequences are decoded. Oversized bracketed paste emits one limit error, discards through its closing marker and cannot execute its tail as commands. Reader restoration precedes terminal restoration.
- `cargo run --offline --locked -p polaris-cli -- --demo` starts explicit labelled scripted data without durable offline session/log/provider/config writes. F5/footer switches profiles with separate drafts/transcripts. Streaming, cancellation, reset/replay and three-question single/multi/free-text form are covered, including exclusive None of the above, no defaults, Tab notes and atomic Enter submission. Popup arrival does not replace another active sheet. This proves demo interaction, not real provider execution or P3 human-request durability.
- Clipboard/index/settings workers stay outside rendering. Paste validates generation plus session/profile/composer revision; cut waits for copy acknowledgement. Empty copy cannot clear the user's clipboard. Explicit one-field global saves preserve unrelated TOML keys and never persist untouched launch overrides; demo settings are memory-only. Tests do not access the actual OS clipboard.

## Exact final test commands and raw results

```text
cargo test --offline --locked -p polaris-config -p polaris-ui-model -p polaris-ui-cli
exit: 0
```

[Command record](continuation-final-core.json), [raw stdout](continuation-final-core.stdout.txt), [raw stderr](continuation-final-core.stderr.txt). Summing the raw result lines gives **263 passed, 0 failed, 0 ignored**, including doctests. This is three packages, not the whole workspace.

```text
cargo test --offline --locked -p polaris-cli --lib --test ui_startup --test ui_session --test ui_key_trace --test rtk_cli --test ui_terminal
exit: 0
test result: ok. 58 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.80s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.99s
test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.13s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 3.26s
```

[Command record](continuation-final-app.json), [raw stdout](continuation-final-app.stdout.txt), [raw stderr](continuation-final-app.stderr.txt). These are **87** selected app tests; the two commands above total **350**. The separate headless failure below is not included or hidden in that total.

After replacing the temporary manually built native example with a **self-spawned test-executable fixture**, the native target was rerun:

```text
cargo test --offline --locked -p polaris-cli --test ui_terminal
exit: 0
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.40s
```

[Command record](continuation-final-native-selfcontained.json), [raw stdout](continuation-final-native-selfcontained.stdout.txt), [raw stderr](continuation-final-native-selfcontained.stderr.txt). This repeat is **not** added to 350. The native-record fixture no longer depends on `target/debug/examples/native_input_fixture.exe`, has no ignored tests and performs no production key-content logging. Graceful `/quit` checks remain in the paste and modifier tests. Earlier [Windows audit](continuation-windows-input-final.txt) mentions the now-removed example; this final target supersedes that fixture detail.

## Safety, format and dependency policy

```text
cargo clippy --offline --locked --no-deps -p polaris-config -p polaris-ui-model -p polaris-ui-cli -p polaris-cli --all-targets -- -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::as_conversions -D clippy::indexing_slicing
exit: 0
```

[Command record](continuation-final-safety-checked.json), [raw stdout](continuation-final-safety-checked.stdout.txt), [raw stderr](continuation-final-safety-checked.stderr.txt). Generic style warnings remain. These five owned-target lints do not prove dependency-tree safety, formal correctness or zero allocations.

```text
cargo fmt --check -p polaris-config -p polaris-ui-model -p polaris-ui-cli -p polaris-cli
exit: 0
cargo deny --offline --locked check bans licenses sources
exit: 0
```

[Final format record](continuation-final-format-checked.json), [format stdout](continuation-final-format-checked.stdout.txt), [format stderr](continuation-final-format-checked.stderr.txt); [dependency policy record](continuation-final-dependency-policy.json), [raw stdout](continuation-final-dependency-policy.stdout.txt), [raw stderr](continuation-final-dependency-policy.stderr.txt). Earlier format failure for import ordering is retained in `continuation-final-format.*`; imports were formatted and the final check passed. No lint configuration or safety assertion was relaxed.

## Meaningful red evidence and limits

| Regression | Observed pre-fix behavior | Raw red evidence | Current green |
|---|---|---|---|
| Presenter caret staging | Changed cells ended with reveal before editor positioning | [stdout](continuation-cursor-real-red.stdout.txt) | Final renderer/presenter suite and native effort/caret case |
| Editor/controller | Ctrl+Enter added a newline; Home went to document start; vertical desired column was lost | [stdout](continuation-controller-real-red.stdout.txt) | Final app library/controller suite |
| Pointer integration | Mouse move projected `None` instead of `Some(Completion(0))` | [stdout](continuation-pointer-red.stdout.txt) | Final app controller and renderer suites |
| Explicit preference persistence | Runtime override survived saving the selected field | [stdout](continuation-services-runtime-override-red.stdout.txt), [stderr](continuation-services-runtime-override-red.stderr.txt) | [focused green](continuation-services-runtime-override-green.stdout.txt), final app suite |

Missing-API compile failures are not behavioral reds. The original broad foundation and initial cold-service behavioral-red capture gaps remain documented in [foundation](verification.txt) and [service evidence](continuation-services-verification.md). Later regression reds do not retroactively fill those gaps.

## Visual and consistency evidence

The 14 controller continuation tests exported actual projected Ratatui buffers using an opt-in capture directory. [Capture command](continuation-visual-capture.json), [stdout](continuation-visual-capture.stdout.txt), [raster command](continuation-visual-raster.json). Reviewed exports:

- [Demo welcome](continuation-after/demo-welcome.png)
- [Questions with None/notes](continuation-after/demo-questions.png)
- [Settings](continuation-after/demo-settings.png)
- [Multiline selection](continuation-after/textarea-selection.png)

JSON/SVG source buffers are alongside each PNG. These are rendered application buffers, not a physical terminal screenshot. Earlier [reference repair](repair-verification.md) retains the five-size welcome design comparisons.

[Final consistency commands and raw output](continuation-final-consistency.json):

```text
check-members: 93 members matched, table has 93 -> OK
check-layers: 93 packages, 269 edges -> OK
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: up to date (10 generated blocks)
UI inventory: 59 unique modules; dependency IDs and EXISTING paths -> OK
```

`git diff --check` exited 0; it emitted only the existing Cargo.toml CRLF normalization warning. Manifest edges are registered in `tools/workspace.toml`; Cargo.lock adds direct edges without new package versions. Main/UI planning documents keep phase acceptance partial. Existing `.polaris/state` working-tree changes were not reverted. No commit, deployment or publication was performed.

## Remaining failures and unverified acceptance

```text
cargo test --offline --locked -p polaris-cli --test headless
exit: 101
test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.40s
doctor failed: polaris: required setup prerequisites are missing; follow the reported onboarding actions
```

[Command record](continuation-headless-environment.json), [raw stdout](continuation-headless-environment.stdout.txt), [raw stderr](continuation-headless-environment.stderr.txt). The existing doctor assertion remains unchanged; full CLI-package green is not claimed.

```text
cargo deny --offline --locked check advisories
exit: 1
failed to acquire advisory database lock: failed to obtain lock file 'C:\Users\Admin\.cargo\advisory-dbs\db.lock': attempted to take an exclusive lock on a read-only path
```

[Command record](continuation-final-advisories.json), [raw stderr](continuation-final-advisories.stderr.txt). Advisory freshness/security verdict is unverified; no exemption or writable-cache workaround was introduced.

Other open gates: actual OS clipboard success/platform failures (including images), direct Windows raw/mouse OS-mode inspection, physical Windows Terminal/Alacritty and Linux keyboard/mouse/accessibility matrix; cross-entry transcript selection and word-drag extension; full interactive tool/diff demo cards; settings error/retry/restart/malformed-config screens; full source paging/layout/transcript caches and measured allocation/latency/release budgets; actual P3 human request/answer/cancel protocol and provider/model integration; broader Miri/Kani/fuzz/whole-system acceptance. ConPTY cleanup assertions observe alternate-screen/paste/cursor output, while pure guard tests inspect mode calls; these do not directly read the restored Windows console raw/mouse mode. A bounded settings worker can complete an already accepted filesystem save after its shutdown grace period. Web welcome, its separate WEB-OPT workstream and desktop remain deferred. No full UI phase PASS follows from this checkpoint.
