# CLI mouse delivery and source-only selection repair — E38

Date: 2026-10-25. Scope: Windows native input ownership, Demo profile activation, shared source selection and selected editor mutations. Full UI/M1 acceptance remains open. Earlier E33–E37 evidence is historical and preserved.

## VERIFIED changes and evidence

- Native Windows input acquisition enables mouse, window, extended and VT input and clears Quick Edit using the existing safe platform adapter. Input close is fallible and idempotent after successful restore, retains failed snapshots for retry, and precedes terminal cleanup. The terminal owner restores the complete original input mask. No authored unsafe, new dependency or lint suppression was added in this repair.
- The production profile picker applies a clicked visible row through the same guarded profile action as Enter. The actual binary is exercised with a synthetic single SGR click on Demo; no second Enter is needed. Explicit startup is also available: cargo run --offline -p polaris-cli -- --demo.
- One private gesture owner covers composer, transcript and question answer/notes editors. Character drag uses graphemes and real source spaces; double-click/drag uses shared word units; triple-click uses logical source lines including existing newline. Direction reversal, Unicode, soft wraps and chip atomicity have regression coverage.
- Strict source hits are separate from caret/drag boundaries. Decorative padding cannot begin a word/line selection or retain stale transcript selection; only source spans receive selection background. Mouse release preserves editor selection for Delete/Backspace, typing, paste and Undo; late copy completion does not mutate the range.
- Optional diagnostic mouse traces record only event category, button, coordinates and modifiers. Existing create-new/privacy/size/error policies remain, and typed/paste/answer canaries are excluded.

## Exact current test commands

1. cargo test --offline --locked -p polaris-ui-model -p polaris-ui-cli

   277 passed; 0 failed; 1 ignored opt-in CPU benchmark. [Command record](mouse-repair-checked-core.json), [raw stdout](mouse-repair-checked-core.stdout.txt), [raw stderr](mouse-repair-checked-core.stderr.txt).

2. cargo test --offline --locked -p polaris-cli --lib --test ui_startup --test ui_session --test ui_key_trace --test rtk_cli --test ui_terminal

   145 passed; 0 failed: 108 library, 2 RTK, 5 key trace, 3 session, 7 startup, 20 native terminal. [Command record](mouse-repair-checked-app.json), [raw stdout](mouse-repair-checked-app.stdout.txt), [raw stderr](mouse-repair-checked-app.stderr.txt).

These two commands total **422 passing tests**, excluding config, broader workspace targets, focused/repeat runs and visual exports. The ignored CPU benchmark was exercised in historical E37, not rerun as a performance claim here.

3. cargo clippy --offline --locked --no-deps -p polaris-ui-model -p polaris-ui-cli -p polaris-cli --all-targets -- -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::as_conversions -D clippy::indexing_slicing

   Exit 0; this is the five selected safety-rule subset. [Command](mouse-repair-checked-safety.json), [stdout](mouse-repair-checked-safety.stdout.txt), [stderr with remaining warnings](mouse-repair-checked-safety.stderr.txt). Full strict Clippy remains failing; see the preserved [strict run](mouse-repair-clippy.txt). No warning gate was weakened to call that strict run green.

4. cargo fmt --check -p polaris-ui-model -p polaris-ui-cli -p polaris-cli

   [Exact result](mouse-repair-format-check.json), [stdout](mouse-repair-format-check.stdout.txt), [stderr](mouse-repair-format-check.stderr.txt).

## Meaningful RED and green mapping

[Controller initial behavioral RED](mouse-repair-controller-red.stdout.txt) has six failures and one retained passing selected-delete case. Missing behaviors include Demo click activation, whole-line and word drag, padding ownership and notes selection. [Expanded green](mouse-repair-expanded-controller.stdout.txt) has 13 controller regressions. [Mouse-trace RED](mouse-repair-trace-red.stdout.txt) and [green](mouse-repair-trace-green.stdout.txt) preserve the diagnostic behavior transition. Native profile click RED is in [its original record](mouse-repair-profile-click-red.txt). Descriptor: [ui_cli_mouse_selection_repair.json](../../../specs/descriptors/ui_cli_mouse_selection_repair.json).

Model tests include arbitrary Unicode properties requiring all returned units to be nonempty, in bounds and on extended-grapheme boundaries. Renderer tests distinguish source from padding, tabs/wide cells/chips, current/cached transcript boundaries and label occlusion. Terminal tests cover snapshot/readback/rollback/fallible retry and close → guard restore → input drop without reapplying raw mode.

## Real child-console readback

The ConPTY child reads actual OS mode bits, rather than inferring them from emitted terminal sequences:

~~~text
CONPTY_MODE_BITS_PROBE_START initial=0x000001f7 raw=0x000001f0 native=0x000003b8 after_input_close=0x000001f0 restored=0x000001f7 CONPTY_MODE_BITS_PROBE_END
~~~

[Raw child output](mouse-repair-console-mode-probe-8388.raw). The final 20-test native target includes the readback probe and profile click path. Synthetic SGR input proves decoder/controller transport behavior; it does not prove delivery of physical clicks by Alacritty.

## Actual-buffer visual review

Four production paint_ui_frame captures were rasterized and inspected: [word drag](mouse-repair-after/word-drag.png), [Unicode selection](mouse-repair-after/unicode-selection.png), [wrapped transcript line](mouse-repair-after/transcript-line.png), [question notes line](mouse-repair-after/notes-line.png). Their JSON cell/style captures and SVG exports sit alongside the PNGs. Selection shading ends at actual source; blank row tails are not filled. Raster font metrics are a review aid, not physical Alacritty screenshots or proof of installed font glyph support.

## Consistency and open boundary

Main PLAN/PHASES, PHASES_STATUS E38, UI_PLAN/UI_PHASES and the existing module inventory are synchronized without promoting any UI gate to PASS or altering generated blocks. Exact repository/inventory checks are recorded in [mouse-repair-consistency.json](mouse-repair-consistency.json).

UNVERIFIED: physical Alacritty 0.16.1 keyboard/mouse interactions, Linux/other host matrix, real OS clipboard success/images, full settings restart/error flows, full strict lint/formal/fuzz/performance/release and complete UI/M1 acceptance. Prior doctor readiness and advisory-cache failures were not rerun or relaxed. Demo remains labelled local fixture state with no P2/P3 provider/human/approval authority. Web welcome, WEB-OPT and desktop remain deferred. Pre-existing local durable state changes were not reverted.

Inventory checker correction: the first three local attempts incorrectly required each inventory path to be a single source file or a Cargo package directory. The existing CSV schema also permits semicolon-separated paths and test directories. Those failed checker transcripts are preserved; the final checker validates every declared path for existence, all module/task references, anchors and dependency acyclicity. No product test assertion was changed.
