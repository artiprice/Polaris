# Composer behavior

This guide describes the implemented composer and completion behavior in the interactive preview. The app has no real agent/backend execution: submitted requests are recorded in local preview state and produce simulated activity.

## Clipboard and attachment behavior

- In the app, `Ctrl+V` requests an asynchronous clipboard paste. When the clipboard provides an image, the app uses it; otherwise it uses text. `Shift+Insert` requests the same app-owned paste.
- Text paste normalizes CRLF and CR line endings to LF. Text over 1 MiB is rejected. A paste with more than five logical lines or more than 500 Unicode characters is stored as a labeled `[Pasted Content M chars]` chip; shorter text is inserted directly. M counts the normalized text’s Unicode characters, including line breaks. The chip retains the original text payload. On submission, the payload is expanded into the request text.
- Images are stored as labeled `[Image N]` chips. A decoded RGBA payload is accepted up to 64 MiB, with dimensions required to match its byte count. All attachments in a session share a 128 MiB limit.
- Text attachment previews offer `expand`, `copy`, and `close`. Expanding replaces the chip in the draft with its exact text. Image previews show a color rendering when supported by the current terminal mode. `Esc` or `Return` closes the preview; `E` expands and `C` copies text attachments. The preview buttons are clickable as well.
- A successful clipboard copy shows a two-second `Copied N word(s)` toast. A clipboard error reports an error toast instead.

Terminal clipboard shortcuts depend on which layer receives the key. `Ctrl+Shift+V` can be handled by the terminal and arrive as text paste; that path cannot retrieve image clipboard data. Use the app-owned `Ctrl+V` or `Shift+Insert` action to request text or image data from the clipboard. In Alacritty, `Shift+drag` can select terminal cells outside the app; the resulting selection color belongs to the terminal.

While a response runs, command/directive/file completion stays available. Standalone UI commands such as `/usage`, `/status` and `/model` open their popup without interrupting work; another ordinary prompt remains in the draft until the current response finishes. File mentions can be completed during work. Loading uses a single `• Working (6m 34s · esc to interrupt)` line, with actual monotonic elapsed time and a bold `esc` label. ASCII mode uses `*` and `.`. Escape interrupts the response and preserves the draft after dismissing active popups, menus, previews or selections; Ctrl+C remains an alternative. The prompt marker follows the first visible draft row, including long runs of Shift+Enter and editor scrolling.

## Draft spacing and prompt history

The bottom composer uses thin top/bottom rules with open sides and the full terminal width minus two-cell outer margins. Visible text can grow to floor(terminal height × 3/8), further limited to keep history, header, status and approval controls in view. There is no fixed six-row cap. Resizing recalculates this space and the shared editor/source-map width without changing draft text or its logical cursor. One blank row remains above the footer.

Submission removes trailing empty or whitespace-only logical lines from the saved prompt, request and session history. Interior blank paragraphs and indentation on nonblank lines remain unchanged. A rejected send leaves the live draft intact. Empty paragraphs render as one row rather than gaining an extra row from display padding.

Plain Up/Down moves within the editor's visual wrapped rows. Up on the first visual row recalls older submitted prompts; Down on the last visual row recalls newer ones. Moving past the newest history entry restores the unsent draft with its cursor, selection, attachments and undo state. Recalled entries open with the cursor at the end, so multiline history remains editable with ordinary arrows. Completion menus keep arrow priority; Shift+Up/Down still adjusts effort. Clearing the draft with Ctrl+C or loading a scenario resets browsing and discards the stashed draft. History is session-local and includes submitted UI commands; /clear removes it.

## Placeholder, goal activity and dollar settings

The empty editor displays a dark-gray “What's on your mind for Polaris?” placeholder that is excluded from source copying, history and submission. Local /goal activity has a blue active/paused timer aligned right immediately above the editor. Dollar settings, including $skills, persist until changed and work inside multiline prompts; they support bare toggles, model effort ranges, comma-separated lists, completion, mouse navigation and Status inspection. See [the complete syntax and lifecycle](prompt-settings.md). These controls store preview state and do not connect services.

## Direct effort shortcut

In the composer, Shift+Up increases the active model’s effort and Shift+Down decreases it through low, medium, high, xhigh and max. The footer updates immediately. The shortcut preserves draft text, cursor, text selection and completion focus, clamps at the endpoints, and shares the in-memory per-provider/model preference with /model. Information popups, forms, attachment previews and approval controls keep their own arrow handling.

## Selection and rich text

Drag with the left mouse button across the composer or conversation, then release to copy the selection. Double-click selects a Unicode word; holding the second press and dragging extends selection by whole words in either direction, including wrapped rows and message boundaries. The same gesture works in text attachment previews. Two clicks must land on the same word within 450 ms; dragging or scrolling resets the click sequence. Attachment chips remain atomic. Conversation selection crosses message and paragraph boundaries in either direction and stays anchored while scrolling. Text attachments expand to their actual content when copied. Conversation and text-preview source maps count rows using the same word-wrapped renderer as the display, preventing selection drift after wrapped or whitespace-only lines. The app copies semantic source text rather than terminal cell contents, preserving meaningful newlines and code indentation. Composer token selection covers the token source.

Copy selection preserves each character’s foreground, URL underline and Markdown emphasis. In the RGB dark theme, visible glyphs get a subtle 30% tint of their source foreground blended over the existing background; spaces and tabs use a neutral white-derived tint over the existing background. White text on black becomes #464646, gold mentions #4B4435, green commands #233800, and blue dollar settings #29364B. Sent-prompt and diff tints blend over their existing gray/red/green surfaces. White text and neutral whitespace on black use #464646; on the #141414 sent-prompt surface they use #545454. This is preblended RGB rather than terminal alpha. The earlier 10% tint was increased to make selection easier to see. ANSI and terminal-native modes preserve source foregrounds with neutral index 237 highlighting; no-color selection uses bold. Selection never imposes a green foreground, adds underline or uses reversed video. `@file` mentions are yellow (#F9E2AF), and file completion keyboard focus uses gold while mouse hover uses lighter cream (#FFFDE6). Command keyboard focus is green (#B6E35B), while mouse hover is lighter green (#DBF5B0). Hover takes precedence even on the keyboard-focused row, restores on pointer leave, and never moves keyboard focus or changes backgrounds. ANSI fallback uses LightYellow/White for hover; no-color distinguishes bold hover from underlined keyboard focus; complete recognized slash commands and `/plan` / `/goal` directives are green. Partial or unknown slash words stay neutral, and unselected completion rows use muted gray. Bare `/` and `@` triggers and incomplete slash tokens are muted gray while editing. These are foreground decorations; they do not change the canvas. A file mention with spaces is inserted in a quoted form such as `@"path with spaces"`.

Sent prompts and assistant replies omit their role headings; demo disclosure remains in the scripted response body. Sent prompts use a #141414 background in the default dark theme. Selected visible glyphs use their matching tint blended over #141414; selected whitespace uses a neutral tint. Unselected prompt cells retain their #141414 surface. Terminal mode inherits unselected surfaces; no-color mode inherits all surfaces. A centered clickable `To latest` control, with #262626 background and #373737 on hover, appears when conversation history is scrolled away from the bottom, unless a completion or attachment overlay is open. Its cover and mouse bounds align to complete wide glyphs beneath it.

Web addresses beginning with http://, https:// or www., plus bare domains such as example.com and example.else, are underlined in drafts, sent prompts, assistant replies and transcript popups. Link underlines remain visible without color. Detection trims sentence punctuation and avoids common filenames, local paths, email addresses and file mentions. Bare domains are a syntax heuristic, not a DNS or reachability check; explicit schemes can express ambiguous filename/domain suffixes.

Tool activity is collapsed to a gold title and a Ctrl+O expansion hint; Ctrl+O expands or collapses every tool’s body inline beneath its title, including later tool activity; it does not open a popup. Clicking an activity title toggles only that entry. Hover uses a brighter gold #FFFDE6 foreground, preserving the background. Ctrl+O clears individual overrides and applies the same expansion state to all entries.

Completed responses append a muted `Worked for <elapsed> · HH:MM` line. Elapsed time is measured from the request start; Windows uses the actual local completion clock. No summary is invented when a start time is unavailable. Ordinary responses use neutral text. Warning and error entries show explicit type labels with amber and red foregrounds, including body text. Errors use #AC4242 with no forced bold in titles or bodies (ANSI/terminal fallback Red); the labels remain visible without color. `/preview` includes a visibly labeled `messages` demonstration.

Fenced code has a numbered muted gutter and lightweight lexical colors for Rust, Python, JavaScript/TypeScript, JSON, TOML and shell aliases. Unknown languages retain plain source text. This is lexical styling, not a language server. Unified diffs use old/new line positions, `+`/`-` markers and full-row dark green/red surfaces. The `/diff` popup uses the same renderer across its full width. Source copying excludes fence labels and line-number gutters while retaining indentation, Unicode and actual diff markers. No-color mode keeps numbers, markers and severity labels without imposed colors.

## File completion

Typing `@` at a mention boundary opens workspace path suggestions. Mouse-wheel Up/Down over any command, directive or file suggestion panel moves the active suggestion one item per notch and reveals later results. This includes the full-width padding and blank top margin. Wheel movement clamps at the first/last item rather than wrapping; keyboard arrows retain their existing behavior. Scrolling suggestions preserves the draft and conversation position, and visible rows remain clickable. Outside the panel, normal conversation/composer wheel routing remains active. Command, directive and file menus clear their entire full-width overlay area, including row padding and one blank top margin row; covered transcript text and mouse targets are hidden until dismissal. Quoted mentions support spaces. Results are workspace-relative paths with `/` separators, capped at 100. Ranking checks basename and full-path prefixes first, then substrings, then fuzzy subsequences; case-insensitive matches use deterministic lexicographic tie ordering. Query results are cached for the current index generation, so rendering asks the in-memory snapshot and does not perform filesystem work.

The index is built asynchronously. Nearby paths are published first, while the deeper workspace scan continues in the background. It reads path metadata and ignore-rule files only; it does not read source file contents. It excludes hidden paths and does not follow symlink directories. It honors root and nested `.gitignore` and `.ignore` rules, plus the configured global Git excludes file when available. Refresh starts a new generation so old results cannot replace a newer snapshot.

Typing `/` at a slash-token boundary opens command or directive suggestions. A slash embedded in a URL or path, such as `/src/foo`, is not treated as a token. Standalone commands such as `/help`, `/model`, and `/status` continue to open local preview screens when submitted as the whole draft. `/plan` and `/goal` are inline directives: on submission they set planning/goal fields in local preview metadata, and `/plan` also changes the local mode preference. They do not start a real agent run or create a backend goal.

## Limits and implementation boundaries

| Payload | Limit |
| --- | ---: |
| Composer text / text clipboard paste | 1 MiB |
| Decoded image RGBA payload | 64 MiB |
| Combined session attachments | 128 MiB |

The file index uses the available local `walkdir` and `globset` implementation to walk the workspace and apply ignore rules. It is a path-only suggestion index, not a content search. User-facing completion depends on the asynchronous index becoming ready.


## Cursor presentation

Typing and mouse packets may arrive in the same input batch. Before dispatching decoded mouse events, the runtime refreshes a stale hit/source map against the current draft in memory, without painting an intermediate terminal frame. A ConPTY regression queues text and a held double-click together, then checks word selection, drag extension and exact deletion.

Middle insertion preserves the editor's logical insertion point. Changed cells and the final cursor position are buffered into one presentation transaction. Supported terminals use synchronized output; the fallback hides the cursor during painting and reveals it at the final position. Identical frames do not paint, move or toggle the cursor. Information popups retain a hidden cursor; dismissal restores the draft position. The cursor is explicitly steady while Polaris runs. Supported blink-mode reporting preserves its shape and restores its original blink state; without a reply, Polaris uses a steady block and restores the terminal's configured default style on exit. Fonts and editor text/layout behavior are unchanged. See [the rendering guide](rendering.md).

A regression reproduces the earlier Ratatui 0.29/Crossterm sequence: typing inside something painted with the cursor visible, revealed it at the rewritten tail, then moved it back to the insertion point. Four event-order tests cover ASCII insertion, Unicode, tabs, wrapping, paste, deletion, popup visibility and cleanup after paint/position/flush errors. The ConPTY smoke test uses real bytes for middle typing, Backspace/Delete, SGR mouse placement, Unicode bracketed paste and Shift+Enter, checking cursor coordinates and visibility as well as text.

The opt-in diagnostic trace includes editor_cursor_byte, the UTF-8 byte offset into the draft rather than a display column. It contains no draft contents. Raw fixture-only cursor transport captures are saved in target/conpty-traces with .cursor.ansi and .cursor.jsonl suffixes.

[Before-fix failure](../target/cursor-before.json) · [Cursor smoke output](../target/cursor-transport.json) · [Verification summary](../target/cursor-verification.json)

Five selection-tint regressions check exact blend values, keyboard/mouse selection at 60/80/120 columns, links and token colors, tabs, wide/combining characters, severity and syntax colors, gray prompts, diffs, attachment previews, fallback modes, repeated frames and exact restoration after deselection. The added ConPTY smoke test checks actual keyboard and held-mouse selection output with missing hints, Alacritty-style hints and forced ANSI; it dismisses selection before shutdown without requesting an OS clipboard write. ConPTY can coalesce invisible spaces into the preceding foreground run, so transport foreground assertions cover visible characters; renderer assertions preserve every source-cell foreground. Raw fixture captures use .selection.ansi and .selection.jsonl suffixes.

[Selection preview](../target/overlay-selection-contrast.png) (rendered Ratatui cells, not a physical terminal screenshot) · [Selection transport output](../target/selection-tints-transport.json) · [Full debug output](../target/selection-contrast-debug.json)

## Current CLI redesign verification

The current full-width trial checks, comparison and package are recorded in target/full-width-trial/verification.json. The preceding centered/open-sided composer evidence remains in target/composer-open-sides/verification.json; the wider CLI redesign evidence remains in target/cli-redesign-review/verification.json. This is separate from the historical records below. Physical Alacritty interaction and backend services remain outside automated verification.

## Verification recorded on 2026-10-19

The Polaris standalone release passed 228 top-level tests, including twenty-one real ConPTY smoke tests. Strict Clippy, formatting and the normal debug build passed. Coverage includes color-depth selection, RGB transport parity, output-mode restoration, package/help/version/error-prefix checks, preserved references and hybrid-logo regressions while retaining the earlier session-settings, goals, clipboard, selection and keymap coverage. The exact tested static executable is dist/polaris.exe. Raw release output: target/selection-contrast-release.json. Final checks: target/selection-contrast-checks.json. Package/startup evidence: target/selection-contrast-package.json. Summary: target/selection-contrast-verification.json. Physical Alacritty interaction remains unverified; backend services and skill loading remain unimplemented.


The earlier UI polish passed 119 top-level tests in `target/rich-composer`, including 13 conversation/control regressions and seven ConPTY smoke tests. It adds single-slash command suggestions, centered To latest hover styling, and mode-on display labels. Raw output: target/ui-polish-test-output.txt. The default executable was locked by a running instance during this verification; the isolated updated executable is target/rich-composer/debug/nvvi.exe.

Earlier default-target baseline:

The default Cargo target passed 116 top-level unit/integration tests with zero failures. Two child entry points marked ignored by the shared runner are explicitly executed by parent tests in isolated consoles.

Commands and observed results:

    cargo test --offline
    # 116 top-level tests passed; zero failures.
    # Conversation regressions:
    test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.02s
    # ConPTY smoke suite:
    test result: ok. 7 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 1.68s

    cargo clippy --offline --all-targets -- -D warnings
    # exit 0

    cargo fmt --all -- --check
    # exit 0

    cargo build --offline
    # exit 0

Conversation regressions cover cross-message selection in both directions, selection across scroll positions, exact-width and word-wrapped text, tabs and wide Unicode, attachment expansion when copying, character-count labels, neutral unknown slash words, To latest clicks, gray sent prompts, and selection surfaces in no-color modes. Existing tests cover editor undo, clipboard errors, image payloads, completion/indexing, popup navigation, fullscreen layout, and statistics.

Real Windows ConPTY smoke tests feed terminal bytes to the executable. They cover direct-effort Shift+Up/Down, Shift+Enter, Shift+Tab, Escape/Return popup exits, arrows, bracketed paste, live file completion, combined /plan and /goal submission, SGR mouse clicks, attachment previews, footer controls, and shutdown. They do not automate a physical Alacritty window.

Colored-buffer captures of Overview, Models, sent prompts, and To latest were visually inspected. Statistics remains explicitly labeled DEMO DATA; no actual account usage is available.

Raw full-suite output is saved in target/conversation-test-output.txt. Clipboard tests use isolated fake backends or inspect queued copy requests. The user's real OS clipboard and physical Alacritty mouse/key behavior remain manual checks.
