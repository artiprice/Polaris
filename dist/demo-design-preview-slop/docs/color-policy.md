# Consistent Polaris colors

The default dark theme uses the same RGB palette in supported Windows VT consoles, including direct double-click launches without COLORTERM or WT_SESSION. The color-depth policy does not modify fonts, terminal profiles or global environment variables. Current artwork and palette roles are documented in [the UI design](ui-design.md).

## Options and precedence

| Option | Behavior |
| --- | --- |
| --color auto | Enable color unless NO_COLOR is present or TERM=dumb |
| --color always | Enable color even with NO_COLOR or TERM=dumb |
| --color never | Disable color regardless of requested depth |
| --color-depth auto | Default: use RGB for Windows VT dark sessions, existing RGB environment hints, or deterministic dumps |
| --color-depth truecolor | Explicitly request RGB |
| --color-depth ansi | Use the existing ANSI/indexed fallback palette |
| --theme terminal | Keep terminal-native appearance and inherited base colors; depth does not select the dark theme |

On non-Windows platforms automatic depth retains the existing COLORTERM=truecolor/24bit and WT_SESSION hints. Dumps remain plain text without escape sequences regardless of depth. Invalid or missing depth values are rejected.

The Windows session duplicates the actual stdout handle, saves its output mode, enables processed output and VT processing, and restores the exact original mode after terminal teardown. Failure to enable VT selects the fallback; an explicitly requested truecolor dark session instead returns an actionable error when Windows VT output is unavailable. Disabled color does not raise that depth error. Existing VT input handling remains separate and unchanged.

Crossterm's color enablement is synchronized with the resolved application policy so its own NO_COLOR handling cannot silently suppress --color always. No environment variables are removed or changed by the application. The original output mode is preserved on normal return, setup error and unwinding; process termination outside normal Rust cleanup is not covered.

## Diagnostics

Use a new trace path:

    cargo run -- --preview --interactive --key-trace target/colors.jsonl

The opt-in trace adds theme, color_enabled and color_depth. Read the colors or start event for the final session choice: bootstrap precedes VT setup. Depth is truecolor, ansi, terminal, or none when color is disabled. These fields contain no environment contents or prompt text. Existing trace privacy and create-new-file behavior remain.

## Evidence on 2026-10-19

At the color-policy revision, the development and exact packaged static release each passed 217 top-level tests, including nineteen ConPTY transport tests and two terminal-decoder tests. The two new color transport tests launch ten default-environment sessions and eight override/theme sessions. They compare reconstructed styled cells, not just text or the presence of an escape sequence.

For both welcome and conversation, default output matches across absent hints, Alacritty-style hints, COLORTERM=24bit, WT_SESSION, and an unrecognized hint. At that historical revision, assertions included black canvas, the then-current chip/star palette, gray sent prompts, gold tool titles and magenta auto mode. Current tests assert the unfilled #C0E58C North Star, #76B900 rays, #141414 sent prompts and preserved semantic tool/mode colors. Override tests cover forced RGB/ANSI, NO_COLOR, TERM=dumb, --color always, --color never and terminal-native mode. Child environments use private environment blocks; host preferences are untouched.

The isolated console probe verifies exact output-mode restoration with VT initially on and off after normal return, a controlled setup error and a controlled panic. Policy tests cover hint/VT/dump combinations, explicit overrides, validation and rejection of a non-console output handle. Strict Clippy, formatting, development build and five PE-import gate tests passed.

    cargo test --offline
    cargo clippy --offline --all-targets -- -D warnings
    cargo fmt --all -- --check
    python scripts/build_static.py --offline --test
    python -m unittest discover -s scripts -p test_static_binary.py

[Development output](../target/color-consistency-debug.json) · [Release output](../target/color-consistency-release.json) · [Checks](../target/color-consistency-checks.json) · [Package/startup checks](../target/color-consistency-package.json) · [Verification](../target/color-consistency-verification.json)

Raw fixture-only color captures and diagnostic records are in target/conpty-traces, with profile/scenario suffixes .ansi and .colors.jsonl. Physical Alacritty and double-click appearance after this change, legacy consoles, other Windows versions and non-Windows execution remain unverified. Matching RGB output does not guarantee identical host font rendering or display processing.

Windows VT processing is documented by [Microsoft Learn](https://learn.microsoft.com/en-us/windows/console/console-virtual-terminal-sequences); Windows Console RGB support is described by [the Windows Console team](https://devblogs.microsoft.com/commandline/24-bit-color-in-the-windows-console/).

The later cursor-presentation release passed 222 tests, including twenty ConPTY transport checks, while preserving this palette and depth policy. Current package evidence is linked from [composer verification](composer.md#verification-recorded-on-2026-10-19).

The later copy-selection release passed 228 tests, including twenty-one ConPTY transport checks. Copy highlighting now preserves the source foreground and blends a 30% matching tint over the current background in RGB dark mode; spaces use a neutral tint. ANSI and terminal-native modes use neutral index 237, while no-color selection uses bold. The output-depth policy and all other palette roles remain unchanged. See [selection rules and current evidence](composer.md#selection-and-rich-text).

## Current CLI redesign

RGB profile comparisons assert every reconstructed symbol and background, plus foreground for every visible glyph. ConPTY can coalesce invisible spaces into a preceding SGR foreground run; exact source-cell foregrounds are checked separately in Ratatui buffer tests. No visible glyph or background is excluded. Current development/static commands and package evidence are in target/cli-redesign-review/verification.json. Physical terminal appearance remains unverified.

## Saved UI color policy

Settings → Accessibility saves Auto, Always or Never. Explicit --color overrides the saved value for that launch; it is not persisted by changing unrelated settings. Color-depth selection remains independent and is not saved. See [Settings](settings.md).

## Activity calendar fills

The Stats calendar uses solid background fills on valid day cells, including both columns in wide layouts. Inactive days use #121212; active days retain the four existing green intensity colors. Day cells touch with no internal gaps; out-of-range positions stay black. The legend shares the same palette. RGB hover/selection lightens the source activity color by 20%/35%, or 45% when combined. ANSI and terminal-native calendars use indexed green/gray ramps. Color-disabled calendars retain textual intensity markers and do not introduce background colors. No global color policy, font or preference changes are made.
