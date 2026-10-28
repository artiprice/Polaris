# Polaris microcircuit and North Star

The active header combines a fine single-line gray microchip, short pins and a small four-point North Star. Bright green, earlier lime and one mint highlight are concentrated at the center. There is no inner die rectangle, monogram, filled pixel-star silhouette or double-line border.

## Geometry

Welcome: 15×7 cells, including padding:

    ␠␠␠␠│␠␠│␠␠│␠␠␠␠
    ␠␠┌─┴──┴──┴─┐␠␠
    ␠␠│␠␠␠␠╷␠␠␠␠│␠␠
    ──┤␠␠╶─✦─╴␠␠├──
    ␠␠│␠␠␠␠╵␠␠␠␠│␠␠
    ␠␠└─┬──┬──┬─┘␠␠
    ␠␠␠␠│␠␠│␠␠│␠␠␠␠

Compact: 11×5 cells:

    ␠␠␠│␠│␠│␠␠␠
    ␠┌─┴─┴─┴─┐␠
    ─┤␠␠─✦─␠␠├─
    ␠└─┬─┬─┬─┘␠
    ␠␠␠│␠│␠│␠␠␠

Here ␠ denotes one ordinary space; the renderer uses spaces, not visible markers. The 11×1 extreme-height fallback is a centered ─✦─ with four spaces on each side.

## Palette and fallbacks

| Role | RGB dark theme | ANSI/terminal color |
| --- | --- | --- |
| Four-point star | #24D046 | Green |
| Short signal traces | #76B900 | Green |
| One mint highlight | #96D6AC | LightGreen |
| Upper/left frame edges | #B8B8B8 | Gray |
| Lower/right edges and remaining pins | #6B6B6B | DarkGray |
| Package surface, including border | #1C1C1C | Black in dark mode |
| Outside package | #000000 | Black in dark mode |

Mint is placed at local (7,2) in welcome, on the upper star ray; compact uses (5,0), the top-center pin. The package occupies local x=2..12, y=1..5 in welcome and x=1..9, y=1..3 in compact. Pins and surrounding padding are unfilled. The fallback emblem is unfilled.

All logo glyphs are regular-weight and one cell wide according to the same Unicode-width library as the application. Terminal-native and no-color modes impose no artwork backgrounds; no-color also inherits foreground. ASCII maps the frame to +, -, | and the star to *, retaining dimensions and color roles. All application theme colors outside the artwork remain unchanged.

## Layout and lifecycle

The existing shared header layout is unchanged. Welcome appears with an empty conversation and sufficient height. Submission or work switches to compact; an unsent draft also switches below 11 body rows. Below seven body rows the fallback emblem is used. /clear and resizing restore welcome when appropriate. Popups, attachment previews and approval retain the underlying artwork, subject to ordinary popup occlusion.

Artwork starts two rows down and two columns in. Three columns separate it from four vertically centered metadata rows. Polaris stays bold; version stays muted and regular-weight. Long metadata truncates at grapheme boundaries. History, source selection, scrolling and composer/footer placement retain their existing geometry.

src/logo.rs owns the private grids, semantic color roles, surface coverage and shared layout. Logo rendering requires no runtime assets or additional dependencies; color-depth launch options are documented separately. The [branding guide](branding/README.md) links both earlier designs and rollback sources. The [exact palette](branding/hybrid-palette.json) records this revision.

## Verification on 2026-10-19

Four logo unit tests cover exact grid dimensions, junctions, star/trace/highlight anchors, all glyph widths, ASCII mapping, palette fallbacks, regular weight and bounded package surfaces. Eight header integration tests preserve metadata, lifecycle, popup/preview, resize/tall-draft, source-map, selection and fallback coverage. Full-frame background assertions allow only the approved package rectangle. Existing composer, keyboard, mouse, statistics and message tests remain active.

The development and exact packaged static release suites each passed 228 top-level tests, including twenty-one real ConPTY transport tests. Two ignored helper entry points are launched by parent tests; their repeated child executions are excluded from totals. Strict Clippy, formatting, development build and five PE-import gate tests passed.

    cargo test --offline
    cargo clippy --offline --all-targets -- -D warnings
    cargo fmt --all -- --check
    python scripts/build_static.py --offline --test
    python -m unittest discover -s scripts -p test_static_binary.py

[Debug output](../target/selection-contrast-debug.json) · [Release output](../target/selection-contrast-release.json) · [Final checks](../target/selection-contrast-checks.json) · [Package/startup evidence](../target/selection-contrast-package.json) · [Summary](../target/selection-contrast-verification.json)

[Welcome](../target/overlay-polaris-hybrid-welcome-80x24.png) · [Compact](../target/overlay-polaris-hybrid-conversation-80x24.png) · [Model popup](../target/overlay-polaris-hybrid-models-80x24.png) · [Size comparison](../target/polaris-hybrid-size-matrix.png)

These are actual Ratatui buffers reconstructed with fixed font settings, not physical Alacritty screenshots. Physical Alacritty font appearance, key emission and live resizing remain unverified; other Windows versions or clean virtual machines were not checked. The application remains a UI preview with no backend behavior added.

The [color policy](color-policy.md) selects this same palette in default Windows VT sessions with or without terminal environment hints; the logo grids and palette roles are unchanged.
