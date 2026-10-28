# Usage and progress bars

/usage opens a full-width adaptive bottom sheet and preserves the draft, conversation position and ongoing preview response. Its preferred height is 30 rows. Expanded cache/subagent details can grow to 40 rows in stacked layouts; two-column layouts keep the compact height. The information area scrolls while the source badge, Details button and closing hint stay pinned.

## Information and sources

Session shows cost, input/output/total tokens, wall/API time and proposed changes. Context shows known consumption and capacity. Current-window and weekly gauges include reset metadata when supplied. Models list input, output and cost. Ctrl+O expands cache read/write, hit rate and subagent call/token/model details. Subagent counts are included in session/model totals.

At 104 inner columns or more, session/gauges and models/details use two columns. Narrower views stack them. Tables and metadata wrap complete graphemes without horizontal scrolling. Rails are capped at 55 cells.

No live source is connected. Missing values remain explicitly unavailable rather than zero. A known numerator without usable capacity stays a count without a percentage. Unknown reset information is not replaced with a countdown.

D switches between unavailable data and a labeled DEMO DATA fixture. Sample cost and quotas do not describe NVIDIA NIM prices or a provider offering.

## Choosing a style

**Strip is the default.** Open /settings, choose Usage, then highlight Strip, Ticks or Segments with Up/Down or a click. Each row previews the same clearly labeled 42% sample through the actual Usage renderer. Enter applies the highlighted style and saves automatically; navigation and clicks alone leave the applied style unchanged. There are no style shortcuts or separate comparison gallery in Usage.

The legacy --scenario usage-bars argument opens Settings → Usage. It is omitted from /preview. Details expansion remains session-only and does not affect inline conversation tools.

| Action | Keyboard / mouse |
| --- | --- |
| Expand / collapse details | Ctrl+O / click Details or Less / Space |
| Scroll information | Up/Down, PgUp/PgDn / mouse wheel |
| Sample / unavailable data | D |
| Close sheet | Escape or Return |

## Bar designs

Ticks has a thin rail, quarter-position ticks and a bright marker. Strip uses a half-height filled strip. Segments separates narrow filled cells with spaces. All three use the same ratio and numerical labels.

Normal usage is green with a pale leading marker. At 80–99% it becomes gold and says Near limit. At 100% it becomes regular-weight muted red and says Limit reached. Above capacity, the rail stays bounded while the actual percentage and Over limit label remain visible. Positive usage below 1% says <1%. Unknown values show Unavailable without an empty-looking zero-percent rail.

ASCII, ANSI, terminal-native and no-color modes retain numerical/status labels. No-color strip and segment shapes distinguish used from unused capacity. There is no decorative animation.

See [Settings and persistence](settings.md) for navigation, configuration paths and launch overrides.

## Verification

Usage ratio, style geometry, missing-data and layout regressions live in tests/usage.rs. Settings interaction/rendering and persistence cases live in tests/settings.rs and tests/preferences.rs. The ConPTY suite highlights styles through keyboard/mouse, confirms them with Enter, checks Usage, and restarts the executable with isolated saved preferences. Actual buffer captures and command results for Enter-to-apply are recorded under target/settings-enter-to-apply. They are rendered Ratatui buffers, not physical Alacritty screenshots.
