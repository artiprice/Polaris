# Polaris terminal header

The active CLI mark is a minimal North Star, drawn directly in terminal cells. It has no runtime asset dependency. The web welcome artwork remains independent and unchanged.

## Artwork

The welcome mark is nine columns by five rows:

```text
    ╷
    │
╶───✦───╴
    │
    ╵
```

The compact mark is five columns by three rows:

```text
  ╷
╶─✦─╴
  ╵
```

The app bar uses the single-cell ✦ emblem. The center is #C0E58C, adjoining rays are #76B900, and tips are #6B6B6B. All artwork is regular-weight, on the inherited canvas, without a filled package. ASCII uses *, - and | with identical dimensions. ANSI uses the theme's portable palette; terminal-native and no-color modes inherit the host surface.

## Layout and lifecycle

The header, history, composer and footer share a full-width content area with two-column outer margins and no width cap. The supported minimum is 60×20. The app bar begins two rows below the top, showing ✦ Polaris, a gray regular-weight v0.1.0alpha, and a clipped workspace path on the right when space permits. An explicitly configured local account label occupies a quiet secondary row; it does not authenticate.

The welcome mark and greeting are centered within the body above the composer. Nine or more available body rows use the 9×5 mark; five through eight use 5×3; less space omits the hero while retaining the app-bar emblem. Typing alone does not dismiss the welcome. Submitted history or active work removes the hero and keeps the compact app bar. /clear restores the clean welcome without a layout-shifting notice. Popups cover the underlying layout rather than choosing a different artwork variant.

The composer stays at the bottom, between thin horizontal rules with open sides. Visible text grows to at most floor(terminal height × 3/8), with further bounds for history, header, status and controls. The footer has exactly one blank row above it. See [UI composition](ui-design.md) and [composer behavior](composer.md).

## Verification and history

Exact glyph, palette, breakpoint, source-copy and resize assertions live in tests/header_logo.rs. tests/design_review.rs checks the complete scenario matrix at 60×20, 80×24, 100×30, 120×40 and 160×60; it can export actual Ratatui buffers with POLARIS_CAPTURE_DIR. These SVG exports are engineering previews, not physical-terminal screenshots. Current command output and captures are in target/cli-redesign-review.

The preceding microcircuit/North Star renderer, specification and artwork tests are archived in [branding/microcircuit-north-star](branding/microcircuit-north-star/). Earlier brand references remain in [branding](branding/README.md).
