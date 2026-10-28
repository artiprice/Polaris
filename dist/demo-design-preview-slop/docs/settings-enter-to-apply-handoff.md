# Settings Enter-to-apply implementation handoff

Implementation completed and verified on 2026-10-21. See [completion evidence](settings-enter-to-apply-verification.md) for the final behavior, checks, package and remaining limits. The handoff below is the original pre-implementation snapshot; its pending-state descriptions are historical.

## 1. Objective and authorization

Implement the user's approved Settings navigation change in the Polaris Rust terminal UI.

The user said: "in settings when we choose option, to apply it we have to Enter the variant that we want. if we do full left (the last left in options) we go to left bar", then approved the plan with "Proceed".

Definition of done: navigation and mouse selection only highlight candidates; Enter applies the focused candidate; Left from the first variant returns to category navigation; tests and help agree. Also finish the previously requested build wrapper and refresh the release in dist after verification.

The user explicitly authorized implementing, fixing failures, and running builds without asking them to execute commands after every change. Do not ask for implementation permission again.

Workspace: C:/mDev/nvvi-code
Shell: PowerShell
Product/package/binary: Polaris / polaris / polaris.exe
This remains a UI preview: no live inference, authentication, provider connections, or real usage source.

## 2. Verified current state and limitations

VERIFIED by source inspection in the handoff-producing turn:
- src/settings.rs still has SettingsAction::{Category, Select, Choose}.
- Choose immediately mutates the setting and calls preferences.commit(field).
- Left, Right, Enter, and Space currently share the immediate-change path.
- SettingsViewState has categories_focused, reveal_selection, page_rows. No tentative variant field exists.
- The new navigation implementation has NOT been applied.
- Tool history: read-only shell calls work; apply_patch and command execution repeatedly failed. During handoff creation, node_repl recovered and successfully wrote this file. Earlier failures were:
  - apply_patch: "Failed to write file ..."
  - exec_command: "helper_unknown_error: setup refresh had errors"
  - node_repl: "failed to write kernel assets: The system cannot find the path specified. (os error 3)"
- These are tool failures, not proof of a project permissions or source-code problem. Retest access in the new session; do not keep rerunning an unchanged failing build.

VERIFIED from dist/build-info.json:
- Current packaged release: dist/polaris.exe
- Target: x86_64-pc-windows-gnu
- Size: 7,992,320 bytes
- SHA256: 4aaf593b0f259efd5cdd6983b556c98470418c7d7e543653b0cd1201f830693d
- The user's successful command was: python scripts/build_static.py --offline
- Static Rust/MinGW support; Windows system DLLs including OS UCRT remain required.
- That release was packaged WITHOUT a successful full release suite. Do not call it fully verified.

There are many existing uncommitted changes from prior work, untracked web assets, and intentional image deletions. Inspect status and preserve them. Do not reset the workspace or revert unrelated work. No subagent delegation is authorized.

## 3. Approved behavior: exact navigation contract

Separate APPLIED preference values from the HIGHLIGHTED candidate.

### Category/sidebar focus
- Up/Down selects a category; preserve existing category cycling.
- Right or Enter enters controls without applying anything.
- Entering a setting initially highlights its currently applied variant.
- Entering Usage initially highlights the currently applied bar-style row.
- Left in the sidebar stays there.
- At narrow widths, the existing category tabs serve as the same focus region.

### Control focus
- Up/Down moves between setting rows; preserve existing row cycling.
- Moving to another row discards the previous row's unconfirmed candidate.
- Left/Right moves the candidate within the current row without applying or saving.
- Right on the last variant stays on it; no wrapping.
- Left on the first variant returns to category focus, retaining the current category and discarding the unconfirmed candidate.
- Example: Auto | Always | Never, highlighting Always -> Left highlights Auto -> another Left returns to categories.
- Enter applies exactly the highlighted variant. It must not advance/cycle to another value.
- Space does not apply settings.
- Tab/Shift+Tab switches category/control focus. Leaving controls discards the candidate; reentering initializes from applied values.
- Esc closes Settings and discards unconfirmed choices. Previously confirmed changes remain applied.
- Scrolling or resizing preserves valid current focus/candidate and keeps the focused control reachable. Wheel scrolling does not apply anything.

### Mouse
- Hover changes presentation only.
- Clicking a variant highlights that row/value and moves focus into controls.
- Enter is required to apply mouse-selected variants too. This assumption was explicitly included in the approved plan.
- Clicking a category changes category and discards the candidate.
- Clicking a row title focuses that row and initializes its candidate from the applied value.
- Keep real click bounds, occlusion, and hover fills aligned.

### Usage styles and provider action
- Usage retains three vertical preview rows: Strip, Ticks, Segments.
- Up/Down or click chooses a candidate row; Enter applies that style.
- Each row has only one horizontal choice, so Left returns to categories from any Usage row.
- Continue rendering all three labeled 42% sample previews using the shared usage::bar renderer.
- Strip remains the fresh-install default.
- Manage providers is an action: focus/click highlights it; Enter opens the existing Providers screen. It must not write UI preferences.

### Applying and saving
- Persistent fields: bar style, theme, color policy, ASCII/Unicode, reduced motion.
- Interaction mode remains session-only.
- Before Enter: do not mutate app.preferences.values, effective app.theme/ascii/reduce_motion, app.mode, saved preferences, override metadata, or configuration files.
- After Enter: update the chosen field, then use the existing commit/sync path.
- Reconfirming an already applied value is normally a no-op.
- Exceptions: Enter retries a failed save; Enter on an active launch override explicitly saves that field and clears its override.
- Unrelated launch overrides must never be saved by committing a different field.
- Save errors retain current session values and show the existing regular-weight red error.
- Leaving Settings never rolls back already confirmed changes.

## 4. Existing architecture and implementation approach

Primary implementation: src/settings.rs
Integration: src/ui.rs, src/app.rs, src/runtime.rs, src/interaction.rs
Persistence: src/preferences.rs
Tests: tests/settings.rs, tests/preferences.rs, tests/usage.rs, tests/conpty_smoke.rs

### Current Settings model
- app.tab: category index
- app.selected: row index
- app.scroll: content scroll offset
- app.settings.categories_focused: category/control region
- app.settings.reveal_selection: reveal focused row
- app.settings.page_rows: measured viewport
- app.preferences.values: effective configured choices, including transient CLI overrides
- app.preferences.saved: values intended for persistence; may contain a pending failed save
- app.preferences.overrides: fields overridden by launch flags
- app.preferences.status: Session, Defaults, Saved, Warning(message), Error(message)
- app.preferences.commit(field): copies ONLY the selected field into saved state, clears its override, saves atomically
- settings::sync_effective(app): updates effective Theme/ascii/reduced-motion from preferences

Categories:
0 Appearance: Theme [Dark, Terminal]; Glyphs [Unicode, ASCII]
1 Usage: three vertical rows [Strip, Ticks, Segments]
2 Interaction: Mode [Auto, Plan]
3 Accessibility: Color [Auto, Always, Never]; Motion [Normal, Reduced]
4 Providers: Manage providers action

Current private choices(app,row) returns (labels, applied index, optional PreferenceField).
Current STYLES order is [ContinuousStrip, FineTicks, SegmentedRail].
UsageBarStyle::ALL has a different order; do not accidentally use it for Settings row indexing.

### Suggested implementation structure
- Add highlighted_value: Option<usize> to SettingsViewState.
- Add SettingsAction::Apply, or an equivalently explicit apply action.
- Keep Category and Select focused on navigation.
- Change Choose {row,value} to highlight only; update all repository callers.
- Introduce helpers for entering controls, returning to categories, resolving applied/current candidate, and detecting an unapplied choice.
- On category change, clear highlighted_value, selected row and scroll consistently.
- On row focus, clamp row index, initialize highlighted_value from choices(...).1, and reveal the row.
- On explicit variant click, set row focus and the clicked candidate, with no persistence call.
- For Usage, the candidate style is determined by the focused row; applied style remains the shared preference.
- Only Apply may perform Settings preference mutations or open Providers.
- Bound left/right indices; do not reuse the old modulo arithmetic.
- Keep layout/render functions read-only.
- Preserve App::open reset behavior and nested Providers -> Settings restoration. Confirm that unconfirmed choices cannot survive a fresh Settings opening.

## 5. Presentation and documentation

Distinguish applied and focused states:
- Applied choice remains marked independently of focus.
- Candidate focus uses the existing gray focus surface and a visible foreground difference.
- Show "Enter to apply" when the focused candidate differs from applied state.
- Usage already has [x]/[ ] markers for applied style; moving highlight must not move [x].
- For ordinary option groups, provide an applied marker that remains understandable in no-color/terminal-native modes; do not rely only on green.
- Do not change the actual theme, glyph set, motion behavior or Usage style merely to preview keyboard navigation.
- Keep save errors visible while a new candidate is highlighted.
- Keep compact layouts and mouse targets correct when labels gain markers.

Update:
- src/ui.rs Settings hint: arrows select, Enter applies, Left at first variant returns to categories; avoid advertising Space as apply.
- docs/settings.md: replace immediate-on-click / Enter-or-Space language.
- docs/usage.md, docs/ui-design.md and README.md: explain confirmation before application.
- Settings still saves immediately AFTER Enter; this is not an Apply-and-Save dialog or staged multi-field form.

## 6. Tests to add/update

Do not weaken or remove checks to achieve green results. Existing tests that assume immediate Choose/Right/Space application need to follow the new contract and assert the intermediate unchanged state.

### Settings keyboard and mouse
- Enter from sidebar only enters controls.
- Right changes highlight but not applied value, effective appearance, mode, overrides or saved file.
- Enter applies the exact highlighted value, without cycling.
- Right clamps at last option.
- Left from second -> first stays in controls; next Left returns to categories.
- Left from a single-option Usage/provider row returns to categories.
- Space does not apply.
- Tab/Shift+Tab and category/row changes discard unconfirmed candidates.
- Esc/reopen discards candidate while preserving confirmed changes.
- Mouse click highlights only; Enter applies.
- Hover never changes keyboard candidate or applied value.
- Usage current marker stays on applied style until Enter.
- Provider click does not open it; Enter does; no preference file is written.
- Session mode changes only on Enter and is not persisted.
- Underlying draft, cursor, working response and conversation scroll survive navigation.

### Persistence
- Use isolated config directories; never write real user preferences.
- Snapshot file bytes before arrow/click navigation and prove they remain identical.
- Enter changes only the intended field.
- Restart loads confirmed values and never unconfirmed ones.
- Unrelated CLI overrides remain transient.
- Explicit Enter on current CLI override saves only that field.
- Enter on already applied ordinary value performs no write.
- Enter after failed save retries and preserves existing error behavior.

### Rendering/resizing
- Cover 60x20, 80x24, 120x40, 160x60.
- Cover Dark, terminal-native, ANSI/no-color, ASCII/Unicode.
- Pending indicator and applied/focused markers remain visible.
- Highlighted controls remain reachable after scrolling/resizing.
- Fills remain inside actual click bounds.
- Opaque sheets still remove covered source/hit cells.

### Existing callers needing attention
- tests/preferences.rs has a choose helper that currently expects immediate application: highlight then Apply for positive persistence tests; keep separate tests proving highlight itself does nothing.
- tests/usage.rs and tests/settings.rs call SettingsAction::Choose directly: update their intent.
- Existing Settings ConPTY test clicks Segments and immediately expects it saved: now assert click alone keeps Ticks, then send Enter and assert Segments is saved.
- Keep the restart verification.
- Prefer explicit CSI-u unmodified Enter bytes in deterministic modifier-sensitive ConPTY cases: b"\x1b[13;1u".

## 7. Two pre-existing release blockers still pending

### A. Settings overlays protected header rows
VERIFIED from repeated user release-suite output:
tests/live_activity.rs::opening_popups_keeps_the_conversation_star_artwork fails:
"logo changed for /settings at 2,4"
Actual cell is the Settings upper-left outline, replacing a previously blank protected header-area cell.

Current src/ui.rs popup_area:
Screen::Settings => area.height.saturating_sub(6),

PROPOSED FIX, NOT YET APPLIED OR VERIFIED:
Screen::Settings => area.height.saturating_sub(9).max(14),

The existing generic height clamp remains:
let height = preferred.min(area.height.saturating_sub(6));
bottom is area.bottom() - 2.

At 120x40 this reserves seven rows above Settings; at the shortest supported height it retains the 14-row usable minimum.
- Keep tests/live_activity.rs assertions intact.
- Update tests/bottom_sheets.rs's Settings exact-position expectation from sheet.y == 4 to == 7 because the corrected layout explicitly reserves the header.
- Keep its before/after cell comparison checks.
- Verify shortest-window Settings interaction, not just the formerly failing large layout.
- This proposed formula was not tested; inspect and refine if the retained tests expose a real problem.

### B. ConPTY fixture sometimes samples the user's physical Shift key
VERIFIED from one user trace:
raw Enter had no modifiers; decoded Enter had SHIFT; screen had already changed from Status to Home.
The test timed out because it expected KeyModifiers(0x0), even though the overlay closed.

Source: input::recover_console_shift deliberately calls GetAsyncKeyState(VK_SHIFT) for legacy Enter/Tab. Do not remove this production fallback; the user needs legacy Shift+Enter/Shift+Tab.

PROPOSED FIX, NOT YET APPLIED OR VERIFIED:
In conpty_escape_and_return_close_status_help_stats_and_apply_model, replace the four Return fixtures (Status, Models, Help, Stats) b"\r".as_slice() with b"\x1b[13;1u".as_slice().
- Preserve strict modifier, screen and selected-model assertions.
- Keep Esc fixtures.
- Keep other legacy/native decoder tests; consider a separate legacy overlay-close check that validates its actual behavioral contract without claiming modifiers were transmitted explicitly.
- Do not increase timeouts, ignore tests, or loosen assertions as a shortcut.
- Most subsequent user runs passed this smoke test; that does not fix its environmental input dependency.

## 8. Build wrapper and verification workflow

The user requested a script wrapping:
$env:RUST_TEST_THREADS = '1'
python scripts/build_static.py --offline --test

scripts/build_verified.ps1 has NOT been created; previous attempts failed to write it.

Create a wrapper that:
- Resolves repository location using PSScriptRoot and works from another current directory.
- Temporarily sets RUST_TEST_THREADS=1.
- Runs python scripts/build_static.py --offline --test.
- Propagates nonzero native exit codes.
- Restores the caller's environment variable and working directory in finally.
- Does not blindly loop on unchanged failures or bypass tests.
- Agent reruns it after diagnosing and fixing each failure.

Suggested content:

~~~powershell
$ErrorActionPreference = 'Stop'
$previousThreads = $env:RUST_TEST_THREADS
$buildExit = 1
Push-Location (Split-Path -Parent $PSScriptRoot)
try {
    $env:RUST_TEST_THREADS = '1'
    & python (Join-Path $PSScriptRoot 'build_static.py') --offline --test
    $buildExit = $LASTEXITCODE
}
finally {
    $env:RUST_TEST_THREADS = $previousThreads
    Pop-Location
}
exit $buildExit
~~~

Suggested order:
1. Check local write/exec access and git status.
2. Implement navigation and targeted regressions; fix the two pre-existing blockers.
3. Run cargo fmt, focused Settings/persistence/Usage/live-activity tests, and focused ConPTY tests.
4. Run cargo clippy --offline --locked --all-targets -- -D warnings.
5. Run the full development suite serially, then the wrapper for the exact static release.
6. Diagnose each failure and rerun after a real correction; keep test assertions meaningful.
7. Capture/inspect actual Settings buffers at narrow and wide sizes.
8. Verify the packaged executable matches the release source and checksum metadata.
9. Run standalone startup/version/Settings/Usage dump checks on the packaged executable.
10. Update docs and distribution metadata to the actual final result.

Build tooling:
- .cargo/config.toml defines cargo static as:
  build --release --locked --target x86_64-pc-windows-gnu --target-dir target/static
- scripts/build_static.py --offline --test builds, runs the release suite, verifies PE system-only imports, then copies to dist/polaris.exe and updates SHA256SUMS.txt/build-info.json.
- Without --test it skips the suite; this was explicitly authorized only for the earlier emergency packaging request. Prefer a verified build for this implementation.
- Never claim a test-gated package was produced when tests stopped the script before its copy step.
- Dist also has non-legacy sibling copies in word-selection, message-presentation and overlay-surfaces. Inspect their role before synchronizing; do not rewrite legacy archive directories.
- Windows may lock a running executable. Identify an actual lock before requesting the user close a process; do not kill unrelated applications.

## 9. Completion reporting

Follow AGENTS.md and relevant workspace skills:
- State definition of done.
- Label checked facts VERIFIED, assumptions LIKELY/GUESSING.
- Show concise actual command-output evidence.
- Never weaken tests to make them pass.
- Report implementation, checks, documentation, packaging, and material limitations separately.
- End with:
  - ✅ DONE & VERIFIED
  - ⚠️ NOT DONE / NOT VERIFIED
  - 🔁 CONSISTENCY CHECK
- A successful compile alone does not establish runtime correctness.
- Physical Alacritty keyboard/font/mouse appearance remains unverified unless actually observed.

## 10. New-session starting instruction

"Read AGENTS.md and this handoff. Implement the already-approved Settings Enter-to-apply behavior, fix the two documented release blockers without weakening their tests, add build_verified.ps1, and run your own verification/build cycle until corrected or a concrete external blocker prevents progress. Preserve existing workspace changes. Do not ask for implementation permission again."
