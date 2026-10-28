# Polaris UI design

## Composition

Use one full-screen Ratatui alternate-screen session with a black canvas and a full-width content area with two-cell outer margins, without a width cap. Keep two rows of top margin, a small North Star app-bar emblem, regular-weight Polaris and a gray regular-weight version. The workspace clips at the right; an explicitly configured local account label gets a secondary row. The welcome body centers the larger North Star above “What’s on your mind for Polaris?”; submitted history or active work retains only the app bar. Artwork, header and history share one geometry calculation. See [the header specification](header-logo.md).

Conversation history fills the center. Tool activity shows a compact [ title ] control in gold and a Ctrl+O hint until expanded. Ctrl+O toggles all tool bodies inline; clicking a title toggles that entry, and hover brightens its foreground to #FFFDE6. Sent prompts and assistant replies omit role headings. Sent prompts use #141414. A semantic text map supports continuous source selection across messages, paragraphs and wrapped Unicode. The centered To latest control uses #262626, brightening to #373737 on hover; its overlay expands to whole underlying glyph cells so copying and hit bounds agree.

The bottom composer has thin top/bottom rules and open sides, a plain > pointer on the first visible draft row, and a black interior. Visible text grows to floor(height × 3/8), additionally bounded to preserve header/history and status controls; there is no fixed six-line cap. Editor wrapping, source maps, cursor placement and mouse hit areas use the same width. An empty editor shows the display-only #525252 placeholder. Working status, right-aligned blue goal state and notices sit above the outline. Approval controls use compact rows below 28 terminal rows and full descriptions in taller windows. The terminal backend hides the cursor during repainting and restores its insertion position before revealing it.

Exactly one blank row separates the composer from the footer. A clickable shortcuts hint sits left when it fits; compact [ mode on ], [ model ] and [ effort ] controls sit right. Semantic mode colors remain magenta/gold. Controls use #343434 outlines, #181818 hover and #191919 focus backgrounds without reversed video. Fill changes stay within the actual control target. Permanent warning counts are omitted; /status contains details.

Completed work adds a muted elapsed-time and local-clock summary. Code blocks use numbered gutters and lexical syntax colors; diffs use full-row green/red surfaces with explicit change markers. Severity is encoded in visible Warning/Error labels and amber/red foregrounds, with ordinary messages remaining neutral. Error titles and bodies use the reference muted red #AC4242 at regular weight; ANSI and terminal-native fallback uses Red, and no-color retains labels without forcing bold.

Secondary commands open black bottom sheets with fine outlines spanning the shared full-width content area, a separated title, and pinned hints, preserving the underlying conversation and draft. Covered text and hit areas are cleared across the full terminal width. Command, directive and mention completion menus are opaque across the terminal width, with one blank top margin row and muted gray unfocused labels. They clear covered text and its mouse targets. Esc returns to the previous view. Return closes information views; selection menus keep Return as their confirmation key. Forms retain their field-navigation and explicit save controls.

## Palette

The default dark theme uses RGB automatically when Windows VT output is available, including launches without terminal environment hints. --color-depth auto|truecolor|ansi controls depth independently of --color auto|always|never. The terminal-native theme retains its host appearance. See [color policy and verification](color-policy.md). The session restores its original output mode after teardown.

Default canvas: #000000. Accent: #76B900. Muted: #858585. Auto mode: #F5C2E7. Plan mode and file mentions: #F9E2AF. Chart series use green, cyan, and gold. These are implementation choices rather than official NVIDIA terminal tokens.

Command/directive/file completion focus and hover alter foreground only. Copy selection preserves source foregrounds and emphasis, with a 30% source-color tint over the existing background in RGB dark mode. Spaces/tabs use the neutral text tint; ANSI and terminal-native modes use neutral index 237. No-color selection uses bold without reversed video. The North Star has a #C0E58C center, #76B900 rays and #6B6B6B tips, all regular-weight with no filled surface. ASCII preserves geometry and user Unicode. See [the active palette](branding/north-star-palette.json).

## Models and settings

/model shows NVIDIA reference models once each, with a trailing current check. Wide layouts show descriptions alongside; narrow layouts show the selected model description below. A [ Provider: … · Change ] control sits below the list, followed by five effort capsules. Left/Right adjusts low, medium, high, xhigh, or max effort; Enter applies it with the model. In the composer, Shift+Up increases and Shift+Down decreases the active model’s effort immediately, without opening a popup. Preferences are per-model in-memory preview state, without claims of backend capability.

Provider configuration separates each label/value group with a blank row. Credential source means a reference or environment variable name. No credentials, endpoints, or availability are validated. Provider lists separate rows with a blank line and use bounded control focus/hover styling. Settings reserves the protected header rows while retaining a usable minimum sheet height, with a 20-column category sidebar at widths of at least 100 columns and wrapping category tabs below that. Appearance, Usage, Interaction, Accessibility and Providers share a compact detail form with gray hover/focus, pinned save status and scrolling. Arrows and clicks highlight candidates separately from applied markers; Enter applies each field and immediately autosaves persistent preferences. Interaction and Providers remain session-only. See [Settings and persistence](settings.md).

## Usage

Usage has an adaptive 30-row sheet; stacked cache/subagent detail can expand to 40 rows, while wide two-column detail keeps the compact height. Strip is the default; all three styles are chosen and previewed only in Settings. The pinned Details control expands cache/subagent information. Session/context/limits and models/details share two columns at 104 inner cells, stacking on narrower screens. Missing data is explicit; all sample cost/quota information is labeled. See [Usage behavior and bar styles](usage.md).

## Statistics

Overview and Models tabs use the same deterministic 365-day sample, visibly marked DEMO DATA. The calendar spans 2025-10-19 through 2026-10-18, aligns Monday/Sunday correctly, omits month labels when edge space is insufficient, and uses small muted inactive dots with round green intensity marks. Wide layouts align summary metrics into two columns; narrower layouts keep a compact summary. Summaries derive favorite model, token/session totals, active days, and streaks from that fixture.

Models plots three series of tokens per interval with green/cyan/gold colors, date endpoints, token-axis labels, and percentages derived from the same totals. Tab or Left/Right switches views; R cycles date windows. D toggles demo/empty state. Return and Esc close the popup. Short popups scroll the logical content while the tabs, date range, and demo label remain visible.

## Input

Shift+Enter inserts a composer newline; Shift+Tab toggles auto/plan. Normalize native BackTab, Tab with Shift, control-character Return/Tab/Escape, CSI-u, and legacy CSI Z reverse-tab events before dispatch. Ignore release events for application actions.

The Windows session enables ENABLE_VIRTUAL_TERMINAL_INPUT and restores the exact original mode on exit. Disabling VT input lets ConPTY consume CSI-u before the app sees it. Crossterm's Windows key parser can then discard C0 controls with virtual key zero; the local record reader preserves those controls instead. Native console events, resize events, UTF-16 surrogate pairs, and repeated key records remain supported. The shared decoder handles enhanced keys, CSI/SS3 navigation, reverse-tab, and bounded bracketed paste. Mouse events decoded during an input batch refresh the rendered hit/source map before dispatch, so a click immediately following typing sees the current draft. Paste never dispatches embedded newlines as submit actions.

The app requests CSI-u reporting directly and pops it on exit. If a received legacy Enter/Tab record lacks Shift, the existing fallback samples only the currently held Shift state for that key; it can miss Shift after release. Explicit CSI-u modifiers are preferable. No terminal configuration is modified.

The optional --key-trace PATH creates a new JSONL file with TTY detection, raw/decoded key kinds, modifiers, and app state, including selection byte length without selected contents. Printable draft characters become Text; draft contents are omitted. Trace files are never overwritten.

Esc closes every popup. Return closes status, usage, stats, diff, transcript, and help; in model/provider/session menus it confirms the selection. Provider forms use Tab/Shift+Tab, Ctrl+S, and Esc. Footer/help strings must match these actions. D owns Usage/statistics demo toggling so Return can close them. Settings uses Tab/Shift+Tab for category/control focus and arrows or clicks to highlight candidates. Enter applies the exact candidate; Space does nothing. Right stops at the last variant, and Left from the first returns to categories. Leaving a row or controls discards unconfirmed choices, while confirmed changes remain applied.

## Composer and completion

The composer accepts text or image clipboard payloads with bounded payload sizes. Submitted prompts omit trailing blank rows but preserve interior blank paragraphs as single rows. Plain Up/Down browses prompt history only at the first/last visual editor row, restoring the unsent draft after the newest entry. Clearing or changing scenario discards the temporary history draft. Long pasted text becomes a labeled attachment chip; expanding a text preview restores the exact text into the draft. File mentions and keyboard-focused file suggestions are gold (#F9E2AF); mouse-hovered files are lighter cream (#FFFDE6). Commands use #B6E35B keyboard focus and #DBF5B0 hover. Hover preserves keyboard selection and all backgrounds, with ANSI/no-color fallbacks. Mouse-wheel navigation belongs to the full suggestion panel, including its blank margin and padding. Each notch moves one suggestion and reveals hidden rows without scrolling the conversation; wheel navigation clamps at list boundaries. Keyboard navigation and clicking visible suggestions remain available. Web links are underlined while preserving Markdown emphasis and the surrounding surface. Recognized complete slash commands/directives are green; incomplete and unknown slash words stay neutral, and copy selection preserves foregrounds with a subtle matching background tint (a neutral tint for whitespace), without adding underline; no-color selection uses bold. URL underlines remain intact. Mouse drag selects source text, and releasing the mouse copies the selected text. Double-click selects a Unicode word; holding the second press and dragging extends by whole words across wrapped lines, paragraphs, and messages. Selection row counts use the same wrapped Paragraph behavior as rendering, including whitespace-only rows. The detailed rules and limits are in [the composer guide](composer.md).

Composer preview behavior remains within the preview application: `/plan` and `/goal` are parsed into submission metadata, and plan mode is a local preference. There is no real agent/backend execution. Existing standalone commands still open their local preview screens. Dollar-menu focus matches the blue setting-token accent, with a lighter blue hover tone and current saved values in row details. Dollar settings, including the session $skills list, have their own completion kind and structured parser; values persist in session state until changed, with a per-submission snapshot. Settings-only entries update local state without starting a reply. See [dollar settings and goal display](prompt-settings.md).

## Preview boundary and manual verification

Inference, authentication, connections, account lookup, coding tools, shell access, file edits, persistent history, and real usage metrics are not implemented. Fixtures remain labeled. Optional account-email labels do not imply sign-in.

Physical key reporting in the user’s Alacritty window, live resizing, and terminal-specific font appearance remain manual checks; rendered buffers do not establish those behaviors.

## Agent work view

The [agent roster and conversation guide](agents.md) defines the black bottom activity strip, pinned Main row, responsive child viewport, scoped keyboard/mouse focus and preview-only lifecycle. Agent threads share transcript rendering and copying while retaining separate scroll and tool expansion; the composer continues targeting Main.

## Rebuilt Statistics

/stats uses a black, open-sided adaptive sheet with pinned tabs, date filters, chart variants and exact-value inspection. Overview has a responsive session calendar and highlights; Models offers Stacked, Lines and Trends from shared normalized records. Settings → Statistics uses the same painters and Enter-to-apply persistence. See [Stats behavior and data definitions](stats.md).
