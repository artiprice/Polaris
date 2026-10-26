# CLI reference-design repair — 2026-10-25

VERDICT: focused design/interaction repair verified; full UI_PLAN and model-execution acceptance remain incomplete.

## What changed

The initial UI plan misread the demo's #141414 submitted-message surface as the live editor background. The renderer also diverged in header/footer placement, hero wording/spacing, per-glyph star colors, growing wrapped input, role captions and opaque menu composition. These are corrected to the actual dist reference. Footer controls dispatch real local actions. Startup and /clear are quiet. Ordinary task rejection is concise and preserves the draft; the app still has no model execution profile.

The screen API adds FooterAction and footer_action_at(view, viewport, x, y), sharing painter and mouse geometry. The editor's top/bottom rules, text inset, wrapping, caret and mouse helpers share one shape. Selected completion candidates remain visible beyond the first page; covered glyphs are actually cleared.

## Commands and raw results

- [repair-final-tests.json](repair-final-tests.json): exact offline cargo command selecting CLI libraries and affected integration targets. Exit0, 92passed/0failed/0ignored. [Raw stdout](repair-final-tests.stdout.txt), [raw stderr](repair-final-tests.stderr.txt), [aggregate](repair-test-totals.json). UI CLI46; app46, including25library,3key-trace,3session,5startup and10terminal-harness checks. The latter are nine native PTY/ConPTY scenarios and one VT-parser regression.
- [repair-final-safety.json](repair-final-safety.json): owned-target no-deps Clippy with unwrap/expect/panic/as-conversions/indexing-slicing denied, exit0. Style warnings remain. This does not claim every prompt/formal/dependency gate is green.
- [repair-consistency.json](repair-consistency.json): final format, workspace membership/layers/docs/generated-block and diff checks.
- [repair-inventory.json](repair-inventory.json): CSV structure and referenced descriptor tests.

The older four-package run's doctor failure and advisory-cache lock failure remain recorded in verification.txt. Those tests were not waived or rerun as part of this focused design repair. No new runtime dependency, lint exemption, commit or deployment was introduced. A cached resvg tool under ignored target/ui-repair renders SVG evidence only; it is not a product dependency.

## Reference and visual evidence

[Cell/color comparison](repair-reference-comparison.json) reports zero non-footer glyph/foreground differences against saved dist welcome buffers at60×20,80×24,100×30,120×40,160×60. The dynamic footer is explicitly excluded because product state is offline/medium and demo state is a sample named model/high; footer geometry/colors/actions have separate tests. Black canvas/editor and sent-message surface assertions are retained.

[Repaired80×24](repair-after/welcome-80x24.png), [reference80×24](repair-after/reference-80x24.png), [repaired60×20](repair-after/welcome-60x20.png), [repaired120×40](repair-after/welcome-120x40.png). PNGs rasterize actual Ratatui SVG exports; they are engineering previews, not physical terminal screenshots. Raw SVG/plain exports for all five sizes are in repair-after/. Before-repair exports are in repair-before/. The80×24 and60×20 PNGs were visually inspected; Windows font/host appearance remains outside this evidence.

Native color test removes COLORTERM/NO_COLOR only from the child environment and confirms RGB star bytes; [captured raw output](repair-auto-color.ansi). Native slash test exercises bare slash, filtered Help, Tab acceptance, Enter, Esc, then quit. Existing receipt, literal-paste, idle-zero-output, newline, unsent-prefill and cleanup assertions remain intact.

## Red/green integrity

- repair-fidelity-red: five genuine visual behavior failures, one invalid long-draft fixture (paste became a chip), optional exporter pass. The fixture was corrected to type lines; its initial red is not counted as growth proof. Final height/resize/caret assertions remain strict.
- repair-portable-palette-red and repair-conversation-red: genuine missing foreground and role-caption regressions, green in final suite.
- repair-red.txt (controller): three genuine notice/startup failures plus one invalid ambiguous /h→/help assumption. The latter was corrected to /hel and is not counted as a product defect. repair-green.txt is its focused green run.
- repair-native-terminal initially exposed missing VT projection behavior. repair-vt-parser-red independently failed on right-edge autowrap; corrected parser also implements CSI X erase-character output. No product receipt/color assertions were relaxed. Final native suite is green.
- [Assertion migration](repair-assertion-migration.md) maps every obsolete visual expectation to source and retained stronger checks. Automatic review initially rejected the bundled test edit; after recording the source proof the explicit patch was accepted. No unresolved approval block remains.

## Still incomplete

No model/provider task execution integration, durable human-question popup/service, full settings persistence, clipboard/semantic selection, full syntax/sheet suite, paging/cache/performance acceptance, Linux leg, physical terminal input/restoration matrix, Miri/Kani/fuzz or complete UI-phase acceptance is claimed. Explicit None of the above remains in the pure questionnaire model, not a live backend workflow.

Final read-only review caught a shared-tail notice clearing regression: Ctrl+V feedback disappeared immediately. repair-notice-lifetime-red captures that behavioral failure; edit-scoped clearing preserves unavailable-action feedback and cursor-navigation state, and the final92-test run covers it. The early side-effecting supports_ansi() probe was removed; Windows live TTY auto chooses RGB as an explicit policy, with actual ConPTY coverage and explicit ANSI override. This is not a universal RGB capability probe or proof of exact Windows output-mode restoration.
