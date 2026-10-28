# NV mixed double-line header badge

The selected B design uses a double-line package with thin pins, layered dark-gray package/die surfaces, soft off-white upper/left edge highlights, a green die outline, and a bold light-green NV monogram. It renders directly into Ratatui cells without image protocols, custom fonts, animation, dependencies, or runtime assets.

## Geometry

The welcome badge is 15×7 cells at all supported terminal sizes:

```text
    │  │  │
  ╔═╧══╧══╧═╗
  ║ ┌─────┐ ║
──╢ │ NV  │ ╟──
  ║ └─────┘ ║
  ╚═╤══╤══╤═╝
    │  │  │
```

The package uses true mixed junctions: ╧ and ╤ join thin vertical pins to double horizontal edges; ╢ and ╟ join thin horizontal pins to double vertical edges. Three short pins sit above and below, with one connection on either side. The inner die uses single-line corners and edges.

Chat uses an 11×5 package with the same mixed strokes and NV, omitting the inner frame. Popups, pending approval, and attachment previews retain the underlying welcome or compact chip. The unfilled six-cell ─[NV]─ emblem is reserved for an extreme available-body-height fallback below seven rows.

Typing retains the welcome badge while it fits. Submission switches to compact; /clear restores welcome. If fewer than 11 rows remain above the composer/footer, an unsent draft also uses compact. The composer currently caps visible text at six rows; an additional notice can trigger this fallback in a 20-row terminal. Resizing restores the larger badge when room returns. Below 60×20 the existing resize message remains.

The header starts two rows down and two columns in, with three columns between artwork and metadata. Four metadata rows are vertically centered using whole-cell coordinates. Title stays bold, version stays muted and regular-weight, and overlong labels truncate at grapheme boundaries. History starts one blank row after the header. Composer/footer placement and their single blank spacer are unchanged.

## Palette and fallbacks

| Role | RGB dark theme | ANSI dark theme |
| --- | --- | --- |
| Package surface | #1C1C1C | Black |
| Die surface | #303030 | DarkGray |
| Package upper/left edges | #B8B8B8 | Gray |
| Package lower/right edges and pins | #6B6B6B | DarkGray |
| Inner die outline | #76B900 | Green |
| Bold NV | #B6E35B | LightGreen |

Welcome fills the closed 11×5 package rectangle and nested 7×3 die rectangle, including border cells. Compact fills its 9×3 package and a 5×1 center patch behind NV. Neither variant fills pin cells or surrounding padding. The extreme-height fallback emblem stays unfilled. Top and left edges are highlighted; bottom and right edges are shaded, with corner colors following their horizontal edge.

The canvas outside the chip stays black. Terminal-native and no-color modes impose no artwork backgrounds and continue to inherit their canvas. No-color retains geometry and bold NV. RGB ASCII retains the same surfaces while mapping vertical wires to |, horizontal wires to -, and corners/junctions to +; the emblem becomes -[NV]-. These are Nvvi palette choices, not a claim of official NVIDIA branding.

## Implementation

src/logo.rs owns the three private variants, exact cell grids, semantic ink and surface roles, palette, and header rectangles. src/ui.rs passes the available body height into the shared layout. Rendering, scroll limits, conversation source mapping, completion, and attachment overlays use that layout. Onboarding and the scenario gallery use the shared header. No public API, CLI option, or appearance selector was added.

Thin A and fully double C remain comparison artifacts only. The [A/B/C comparison](../target/nv-chip-comparison.png) holds grid size, colors, metadata, and font settings constant; B is the shipped geometry. That comparison records the earlier unfilled stroke study. The [current before/after comparison](../target/nv-chip-surfaces/comparison.png) shows the gray surfaces and edge highlights with identical glyphs, layout, metadata, and font settings.

## Verification on 2026-10-19

Twelve header regressions cover the exact welcome and compact grids, mixed pin junctions, NV placement and boldness, RGB/ANSI colors, bounded gray surfaces, black/inherited canvas, single-cell glyph widths, ASCII/no-color rendering, lifecycle restoration, metadata truncation, and available-height fallback. Tall-draft cases include notice/no-notice states and repeated resizing through 60×20, 80×24, 100×30, and 120×40. Full-frame checks cover welcome, compact, and popup states across RGB/ANSI, ASCII, terminal-native, and no-color modes. Selection cannot alter chip surfaces. Existing selection/source-map, statistics canvas, and borderless-composer checks remain active.

The complete release suite passed 180 top-level tests, including fourteen ConPTY smoke tests. Two ignored child entry points are executed by their parent tests in isolated consoles. Strict Clippy and formatting checks passed. The normal debug executable was rebuilt. The standalone executable and checksum were refreshed at dist/nvvi.exe.

Commands:

    cargo clippy --offline --target-dir target/rich-composer --all-targets -- -D warnings
    cargo fmt --all -- --check
    python scripts/build_static.py --offline --test
    cargo build --offline --target-dir target/rich-composer

[Release output](../target/working-indicator-checks.json) · [Verification summary](../target/working-indicator-verification.json) · [Packaged imports and checksum](../dist/build-info.json) · [Minimal-PATH startup checks](../target/portable-smoke-output.json)

[Welcome](../target/header-welcome-80x24.png) · [Compact chat](../target/header-conversation-120x40.png) · [Model popup](../target/header-models-80x24.png) · [Tall draft with notice](../target/header-tall-60x20.png)

These are reconstructed buffer previews using fixed font settings, not Alacritty screenshots. Physical Alacritty line continuity, font weight, keyboard reporting, and live resizing remain unverified. No backend behavior was added.
