# CLI reference parity continuation — 2026-10-25

This is a scoped implementation checkpoint (E37), not full UI/M1 acceptance. The target supplied by the user is Alacritty with ```cargo run --offline -p polaris-cli```. Alacritty 0.16.1 is installed; physical key/mouse interaction was not available to this session.

## What changed and its source

| Area | Reference / requirement | Current implementation and boundary |
|---|---|---|
| Welcome, palette, sheets | dist src/theme.rs, logo.rs, ui.rs and docs/color-policy.md | Black live editor, source palette and footer geometry; page-specific bottom sheets preserve the global footer. Tiny layouts remain bounded. No exact pixel parity claim for every screen. |
| Input and editor | dist src/input_windows.rs, input.rs, composer.rs; user's Alacritty failures | Safe Windows record transport; key releases no longer split pending VT/paste; DEL is Backspace; ambiguous Ctrl+Enter does not become Ctrl+J. Shift+Enter, Unicode deletion and middle paste have native app regressions. Effort changes retain editor caret. |
| Mouse / hover / copy | dist src/interaction.rs, theme.rs | Typed bounded hits; no full-transcript hover tint; role-specific completion/tool hover, local choice-row tint, semantic cross-entry Unicode selection and word drag. Windows right/middle release identity is tracked. Native clipboard success remains unverified. |
| Commands and demo | dist src/app.rs, models.rs, preferences.rs, ui.rs | Clean labelled Demo Welcome, typed slash routing, local model/provider/session/settings/status/help/preview screens and approval decisions; unknown slash input is not swallowed as a response. All 27 source scenario names plus questions/editor are reachable through the real binary. Demo effects have no execution/approval authority or durable offline state. |
| Questions | New user requirement; absent from original 27 reference screens | Composer replacement with no defaults, green selected tick/text, exclusive None of the above, independent Unicode answer/notes editors, tabs and Submit review, all-at-once validation, retained main draft, local interruption. FreeText paint/click/vertical navigation uses wrapped editor geometry. Real P3 request/answer service is not implemented. |
| Usage / Stats | dist src/usage.rs, stats.rs, stats_data.rs, stats_chart.rs | Explicit fixture totals, model/cache/subagent breakdown, capacities/reset labels; wide/narrow Usage, three bar styles; Stats Overview/Models, ranges, calendar/day inspector and three chart styles. Fixture data is visibly labelled, not account telemetry. |
| Agents / transcript / tools | dist src/agents.rs, agents_ui.rs, code_view.rs, syntax.rs | Four distinct roster fixtures (2/7/2/9 rows), seven lifecycle states and viewing actions; expandable presentation-only tools, rich inline/fenced code and source-mapped gutters. Agent progression and independent full child transcripts remain incomplete. |
| Repaint cache | dist src/ui/cache.rs, docs/rendering.md | Persistent opt-in exact-content cache used by app paint and fresh hit metadata; old APIs preserved. 32 MiB accounting ceiling plus 1 MiB exact source-snapshot ceiling. Full pane caching/source paging and long-history performance acceptance remain open. |

[Original reference binary dumps](parity-reference-binary.stdout.txt): 27/27 exit 0 at 120×40, isolated POLARIS_CONFIG_DIR was not created. App captures use the actual production paint_ui_frame, not a separate gallery renderer: 29 scenarios at 60×20, 80×24, 100×30, 120×40 and 160×60, plus question review/free-text exports. [Capture command](parity-capture-checked.json), [raw output](parity-capture-checked.stdout.txt).

## Final scoped commands

```text
cargo test --offline --locked -p polaris-config -p polaris-ui-model -p polaris-ui-cli
exit: 0
297 passed; 0 failed; 1 ignored (opt-in CPU benchmark)

cargo test --offline --locked -p polaris-cli --lib --test ui_startup --test ui_session --test ui_key_trace --test rtk_cli --test ui_terminal
exit: 0
95 library + 2 RTK + 4 trace + 3 session + 7 startup + 18 terminal = 129 passed; 0 failed
```

[Core command](parity-final-core.json), [stdout](parity-final-core.stdout.txt), [stderr](parity-final-core.stderr.txt); [app command](parity-final-app-checked.json), [stdout](parity-final-app-checked.stdout.txt), [stderr](parity-final-app-checked.stderr.txt). Total: 426 passing scoped tests. Repeats/captures and the separately executed benchmark are not added to that total. This is not the whole workspace.

```text
cargo clippy --offline --locked --no-deps -p polaris-config -p polaris-ui-model -p polaris-ui-cli -p polaris-cli --all-targets -- -D clippy::unwrap_used -D clippy::expect_used -D clippy::panic -D clippy::as_conversions -D clippy::indexing_slicing
exit: 0
cargo fmt --check -p polaris-config -p polaris-ui-model -p polaris-ui-cli -p polaris-cli
exit: 0
```

[Safety command](parity-final-safety-checked.json), [raw stderr](parity-final-safety-checked.stderr.txt); [format command](parity-final-format.json). Generic Clippy/dead-code warnings remain. These five selected lints do not establish the entire prompt's strict lint list, dependency-tree safety, no allocations or formal correctness. Earlier three unsafe test-fixture accesses were corrected with checked access; assertions were retained.

## Meaningful regressions and evidence

| Regression | Observed failure | Evidence |
|---|---|---|
| Slash routing / ordinary prompt | /model swallowed, unknown slash simulated, ordinary response forced questions | [RED](parity-controller-red.stdout.txt), final app target |
| Unicode notes | Backspace left half the combining grapheme | [RED](parity-question-editor-red.stdout.txt), final model target |
| Field replacement / undo limits | Capped selection replacement rejected; undo exceeded aggregate cap | [RED](parity-question-budget-red.stdout.txt), [GREEN](parity-question-budget-green.stdout.txt) |
| Key diagnostics | Trace could not distinguish Shift/Ctrl Enter, deletion keys or release | [RED](parity-trace-red.stdout.txt), final trace target; text/paste canaries excluded |
| Sheet position | Model title at row 4 instead of source-derived row 24 | [RED](parity-bottom-sheet-red.stdout.txt), [GREEN](parity-bottom-sheet-green.stdout.txt) |
| Real binary scenario catalog | working/usage-demo rejected by stale validator | [RED](parity-binary-scenarios-red.stdout.txt), final startup target exercises all 29 |
| Windows release identity | Right/middle release reported Up(Left) | [RED](continuation-input-mouse-red.stdout.txt), [GREEN](continuation-input-mouse-final.stdout.txt) |
| Free-text form / review focus | Missing wrapped editor metadata; active question stayed green in review | [RED record](questionnaire-free-text-red.json), [GREEN record](questionnaire-free-text-verification.json), final renderer/controller targets |

The cache's initial missing-API compile failure is a specification/build RED, not a behavioral RED. Historical foundation/service initial-red gaps remain documented in continuation-verification.md.

ConPTY consumes the mouse DECSET bytes in this environment: the minimal [raw probe](continuation-input-conpty-mouse-modes.raw) retains start/end markers but omits those controls. Exact balanced mouse/CSI-u bytes are instead asserted at the direct CrosstermOps writer layer. Remaining PTY cleanup checks are retained. This corrects the observation boundary rather than claiming direct restored OS-mode inspection or physical Alacritty success.

## CPU-only cache measurement

Frozen workload: 200×60 TestBackend, 56 entries / 143,854 source bytes, 64 samples per group. Units below are ms; no writer/flush/display latency is included.

| Build / path | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|
| Debug uncached | 176.924 | 220.320 | 224.896 | 239.274 |
| Debug cold | 179.410 | 227.712 | 230.095 | 235.352 |
| Debug warm | 13.867 | 17.601 | 18.065 | 18.084 |
| Release uncached | 21.585 | 22.586 | 23.686 | 24.726 |
| Release cold | 23.985 | 28.217 | 30.739 | 31.014 |
| Release warm | 3.888 | 4.400 | 4.746 | 4.805 |

[Debug command/results](renderer-cache-benchmark-debug.json), [release command/results](renderer-cache-benchmark-release.json), [cache verification](renderer-cache-verification.json). Cache retained 6,692,782 accounted bytes, including 143,854 exact snapshot bytes. Cold paths add overhead. The warm optimized microcase is under 8 ms; debug and cold paths fail that number. Full UI6 performance acceptance remains OPEN, including 1k/10k/100k histories, paging, RSS/allocations, floods, slow/failing output and physical latency. The existing per-entry/total source render window is not full 16 MiB source paging.

## Actual app visual evidence

- [Welcome](parity-after/welcome-120x40.png)
- [Questions](parity-after/questions-120x40.png), [review](parity-after/questions-review.png), [free-text](parity-after/questions-free-text.png)
- [Models](parity-after/models-120x40.png), [Usage](parity-after/usage-demo-120x40.png), [Stats](parity-after/stats-demo-120x40.png), [Agents](parity-after/agents-mixed-120x40.png)

JSON cells and code-generated SVGs accompany the PNGs. Rasterization uses Consolas, not a screenshot of the user's Alacritty window. [Raster command](parity-final-raster.json). Visual inspection found and corrected the questions→onboarding route and top-anchored sheets; capture success alone was not treated as semantic screen correctness. Not every fixture is pixel-identical to dist.

## Independent failures / unverified work

```text
cargo test --offline --locked -p polaris-cli --test headless
exit: 101
2 passed; 1 failed
doctor failed: polaris: required setup prerequisites are missing; follow the reported onboarding actions
```

[Current headless command](parity-headless-current.json), [stdout](parity-headless-current.stdout.txt), [stderr](parity-headless-current.stderr.txt). The assertion is unchanged; full CLI-package green is not claimed. Historical advisory audit could not obtain its read-only cache lock; it was not rerun or represented as current security clearance. No new dependency version was added by this parity slice.

Still unverified/incomplete: physical Alacritty/Windows Terminal and Linux input/mouse/teardown matrix; actual clipboard success/images and native failures; full notes clipboard gestures; timed agent progression and independent child conversation payloads; complete diff scrolling/tool summaries/overlay pixel parity; settings retry/restart/malformed-file screens; complete source paging/pane cache and broad performance/allocation/release gates; P2/P3 execution/provider/human-request authority, persistence and races; broader Miri/Kani/fuzz/system acceptance. Web welcome, its separate WEB-OPT workstream and desktop remain deferred.

## Consistency and preservation

UI_PLAN, UI_PHASES, UI_MODULES and main PLAN/PHASES/PHASES_STATUS link E37 without promoting a UI gate to PASS. Existing .polaris/state changes were not reverted. No commit/deploy/publication was performed. Final consistency commands are recorded separately and appended below after inspection.

An automatic approval review rejected a proposed temporary report overwrite because it could discard retained evidence. Reading the target proved it absent; this complete report was created with create-new semantics, without overwriting an existing evidence file.


### Final consistency results

All commands below exited 0. Raw stdout/stderr and exact command records share the linked stem.

```text
check-docs: 93 packages x (PLAN section, PHASES task, STATUS row) + 13 spikes + 26 member-setup rows -> OK
PHASES.md: up to date (11 generated blocks)
PHASES_STATUS.md: up to date (10 generated blocks)
check-members: 93 packages -> OK
check-layers: 93 packages, 269 edges -> OK
UI inventory: 60 unique modules, 41 tasks, dependencies acyclic, EXISTING paths and phase maps OK
git diff --check: exit 0 (existing Cargo.toml CRLF normalization warning only)
```

[Docs](parity-docs-check.json), [generated sections](parity-generated-check.json), [members](parity-members-check.json), [layers](parity-layers-check.json), [inventory](parity-inventory-check.json), [diff check](parity-diff-check.json), [machine index](parity-verification.json). Final capture rerun: 42 controller tests passed; this repeat is not added to 426. Eight PNG exports were visually inspected, including the corrected two-column Usage and wrapped free-text answer/Submit review.
