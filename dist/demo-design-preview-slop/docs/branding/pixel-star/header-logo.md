# Polaris pixel-star header

The header renders a stepped five-point star inspired by the supplied primary pixel-art reference, with a green body and mint facets. It uses Ratatui cells directly: no runtime image files, image protocol, custom fonts, animation or new dependencies. Reference images and checksums are in [assets/branding/references](../assets/branding/references); the [branding guide](branding/README.md) records the rename and rollback.

## Geometry and lifecycle

| Variant | Terminal cells | Logical pixels | When used |
| --- | --- | --- | --- |
| Welcome | 15×7 | 15×14 | Empty conversation with enough body height |
| Compact | 11×5 | 11×10 | Submitted conversation, work in progress, or body height below 11 rows |
| Emblem | 11×1 | Centered asterisk | Extreme body height below seven rows |

Two vertical pixels share each terminal cell using ▀, ▄ and █. Both masks are connected silhouettes with a top tip, wide arms and a bottom-center notch. The mint facet follows the upper-right and lower-right edges. No monogram or enclosing chip remains. ASCII uses separately tuned asterisk masks with the same cell footprints.

Typing retains the welcome star while it fits. Submission switches to compact; /clear restores welcome. Popups and attachment previews retain the underlying variant. Resizing restores the larger mark when room returns. Below 60×20 the existing resize message remains.

The header starts two rows down and two columns in, with three columns between artwork and metadata. Four metadata rows are vertically centered using whole-cell coordinates. Polaris stays bold; the version stays muted and regular-weight. Long metadata truncates at grapheme boundaries. History starts one blank row after the header. Composer/footer placement and their single blank spacer are unchanged. Shared geometry keeps conversation scrolling and source selection consistent with the artwork height.

## Palette and fallbacks

| Role | RGB dark theme | ANSI/terminal color |
| --- | --- | --- |
| Body | #24D046 | Green |
| Mint facet | #96D6AC | LightGreen |
| Empty pixels | Black canvas | Black in dark mode; inherited in terminal mode |

The star is regular-weight. A cell with two differently colored occupied halves uses foreground/background colors only in the dark color theme. Terminal-native and no-color modes impose no artwork backgrounds; differing occupied halves become one full block, preserving the silhouette. No-color inherits foreground as well. ASCII uses asterisks in the body color, or inherited foreground without color.

All application colors outside the logo remain unchanged. The original theme source, chip renderer, specification and palette are preserved in [legacy-nvvi](branding/legacy-nvvi/README.md). Recoloring only the new star to #76B900/#B6E35B provides the former green family while keeping the Polaris name and star geometry.

## Implementation

src/logo.rs owns the private variants, masks, palette and header rectangles. src/ui.rs renders the shared header. Rendering, conversation source mapping, scroll limits, completion and attachment overlays use the same layout. Onboarding and the scenario gallery also use it. No public API or appearance setting was added.

## Verification on 2026-10-19

Four logo unit tests and eight header integration tests cover connected masks, tip/notch anchors, widths, exact palette cells, welcome/chat switching, popup and preview preservation, metadata styling/truncation, tall drafts, resizing, source-map scrolling, selection isolation, ASCII, ANSI, terminal-native and no-color modes. Full-frame background checks allow only the exact occupied mint/body halves. Three branding tests cover package/CLI identity, placeholder/vendor consistency and preserved reference/rollback files.

The debug and exact packaged static release suites each passed 209 top-level tests, including seventeen real ConPTY transport tests. Two ignored helper entry points are launched by parent tests; repeated child executions are excluded from the count. Strict Clippy, formatting, and five PE-import gate tests passed.

Commands:

    cargo test --offline
    cargo clippy --offline --all-targets -- -D warnings
    cargo fmt --all -- --check
    python scripts/build_static.py --offline --test
    python -m unittest discover -s scripts -p test_static_binary.py

[Release output](../target/polaris-release.json) · [Final checks](../target/polaris-checks.json) · [Package/startup evidence](../target/polaris-package.json) · [Summary](../target/polaris-verification.json)

[Welcome 80×24](../target/overlay-polaris-welcome.png) · [Compact chat 80×24](../target/overlay-polaris-conversation.png) · [Model popup 80×24](../target/overlay-polaris-models.png) · [Size comparison](../target/polaris-size-matrix.png)

These are reconstructed actual-buffer previews using fixed font settings, not Alacritty screenshots. Physical Alacritty font appearance, key reporting and live resizing remain unverified. No backend behavior was added.
