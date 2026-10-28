# Windows keyboard regression verification

Verified on 2026-10-18 in this Windows workspace. The user launches Alacritty with:

```powershell
cargo run -- --preview --interactive
```

## Reproduced failures

The old synthetic key tests bypassed the terminal transport. At that baseline, the new harness started the actual Nvvi executable under Windows ConPTY and wrote terminal bytes into its input pipe. Its bootstrap checks confirm both input and output are terminals.

Two controlled runs exposed different failures before application key dispatch:

1. With VT input disabled, ordinary text reached the draft, but CSI-u Shift+Enter produced no input event within the smoke-test deadline.
2. With VT input enabled and the old Crossterm Windows reader, the six printable characters `[13;2u` entered the draft; the Escape introducer was lost. Inspection of the installed reader showed C0 characters with virtual key zero going through virtual-key translation.

## Runtime correction

The Windows guard now enables VT input and restores the original console mode on exit. A local ReadConsoleInputW reader preserves the Unicode control field when the virtual key is zero. It keeps native key mapping, resize records, repeat counts, and UTF-16 surrogate pairs. The shared decoder recognizes enhanced keys, reverse-tab, and CSI/SS3 navigation. Bracketed paste is bounded and inserted as text rather than dispatched as Enter actions.

The old native-record regression still checks Shift+Enter and ordinary Return. Native records are injected with VT disabled because enabling VT re-encodes them and can discard their modifiers. Its encoded-control check runs with VT enabled. This test establishes reader behavior only; the separate ConPTY smoke tests establish the runtime transport behavior with VT enabled throughout.

## Evidence from the default Cargo target

```text
cargo build --offline
Finished `dev` profile [unoptimized + debuginfo]

cargo test --offline
# ConPTY suite:
test conpty_csi_u_shift_enter_inserts_newline_without_submitting_draft ... ok
test conpty_shift_tab_encodings_toggle_mode_in_both_directions ... ok
test conpty_arrow_and_bracketed_paste_do_not_submit_input ... ok
test conpty_escape_and_return_close_status_help_stats_and_apply_model ... ok
test result: ok. 4 passed; 0 failed; 1 ignored

cargo clippy --offline --all-targets -- -D warnings
Finished `dev` profile [unoptimized + debuginfo]

cargo fmt --all -- --check
# exit code 0, no output
```

The complete default-target suite passed 61 parent/unit/integration tests with zero failures. Two ignored child entry points are explicitly launched by their parent tests in isolated consoles. Helper-child executions are not double-counted.

ConPTY checks cover CSI-u Shift+Enter without submission, legacy and CSI-u Shift+Tab, Status/Help/Stats Escape and Return, Down+Return model selection with the expected active model, arrows, bracketed paste, graceful Ctrl+C shutdown, and balanced keyboard protocol output. Trace privacy checks reject literal draft text.

Renderer regressions check exactly one blank row between the composer and footer for empty, single-line, and multiline drafts at 120×40, 80×24, and 60×20. A colored buffer render at 100×30 was also inspected: prompt on row 27, blank row 28, footer row 29 (zero-based).

## Limits and diagnosis

Physical key presses in the user's Alacritty window were not automated or observed. These tests verify terminal bytes through ConPTY and the real executable; they do not establish which bytes an individual terminal configuration emits. Live resize interaction and font appearance remain manual checks.

For diagnosis, use a new trace filename:

```powershell
cargo run -- --preview --interactive --key-trace target/keys.jsonl
```

The trace records TTY detection, raw/decoded key kinds, modifiers, and state transitions. Printable draft characters become Text. Existing trace files are rejected rather than overwritten.

## Rich composer update

The color-policy and composer regressions expand verification to 228 top-level tests, including twenty-one ConPTY transport tests. See [composer behavior and verification](composer.md#verification-recorded-on-2026-10-19) for the current results and limits. The 61-test count above records the earlier keyboard-only baseline.

## Color transport and output-mode restoration

The [color policy guide](color-policy.md) records environment-controlled RGB cell comparisons, override precedence and exact stdout-mode restoration after normal return, setup error and unwinding. Input mode, physical font preferences and terminal profiles are unchanged. Physical Alacritty appearance remains a manual check.

## Cursor repaint regression

[Cursor presentation](composer.md#cursor-presentation) documents the before-fix failure, hidden-paint/position/reveal ordering and real-ConPTY midline typing, mouse, Unicode paste, deletion and newline checks. The trace adds only editor_cursor_byte, without recording draft contents.
