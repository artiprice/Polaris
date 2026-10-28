# Statistics

/stats opens a black adaptive sheet with open sides, a thin top rule and pinned controls. Overview and Models are tabs inside /stats; the /model model picker is separate and unchanged. The preferred sheet height is 32 rows, bounded by the terminal. Underlying conversation, composer draft, agent inspection and ongoing responses are preserved.

## Sources and definitions

No live activity collector or provider telemetry is connected. Normal /stats shows Unavailable, not zero. D or Sample opens clearly labeled deterministic DEMO DATA. This toggle is independent from /usage. Sample sessions, durations and model activity are synthetic; they do not describe an account or provider quota.

All tabs and chart styles use one normalized dataset. Total tokens are disjoint uncached input + output + cache read + cache write. Percentages use that same denominator. Unknown categories propagate Unavailable rather than zero. Session duration is elapsed time per recorded session; totals are sums of these durations, not a claim about non-overlapping wall time. Current streak ends on the selected period's last day. Missing calendar dates interrupt streaks; session IDs are counted once.

All time, Last 7 days and Last 30 days filter every summary and chart. Daily charts use day totals; All time uses calendar-week totals with partial edge weeks. Sample dates are fixed so comparisons and captures remain reproducible. Daily and weekly buckets reconcile with the same model and overall totals. All token counters are 64-bit; arithmetic overflow becomes unavailable.

## Overview

A summary strip shows total tokens, session count and active days. The activity calendar uses Monday-first weeks, labeled months and four session-count levels: 1, 2, 4 and 8 sessions. Colored day cells are solid, touching background blocks: dark gray for inactive days and four green levels for activity. Thresholds remain consistent when filters change. Wide calendars use two columns per day; narrower ones use one or wrap complete weeks into stacked panels. The legend uses the same touching blocks. Hover lightens the activity color by 20%, selection by 35%, and both together by 45% in RGB dark mode. ANSI and terminal-native views use indexed background shades; ASCII retains filled blocks when color is enabled. With color disabled, `.`, `1`, `2`, `3`, `4` retain intensity information. Every date remains reachable by vertical scrolling.

Highlights include the top model, busiest day, current/longest streak and longest session. Details adds uncached input/output/cache counts and total session time. Hover, click or [ / ] inspects a day; exact values stay visible in the pinned inspector.

## Models: three designs

Stacked, Lines and Trends share identical records, buckets, model ordering and colors. Up to five models are shown individually, with remaining models folded into Other. Palette identity is independent from ranking. ASCII/no-color charts use numbered model markers matching their legends.

- Stacked (default): vertical composition columns, thin baseline, a shared zero-based axis and partial-block precision. Tiny values are not forced to a full cell. Unknown stacks show a question mark. Narrow charts split at bucket boundaries rather than dropping dates.
- Lines: connected thin model curves on the same zero-based scale as Stacked, with no area fill. Missing values leave gaps; known zero sits on the baseline. Hover/click a model to emphasize its curve while retaining all totals.
- Trends: separate three-row mini charts with consistent time boundaries. Each has its own explicitly labeled zero-to-peak scale. Counts and shares remain available for comparing volume; curve height alone does not imply equal volume.

Ranked model rows show total tokens and share. Details reveals uncached input/output/cache figures per model. Hover/click a bucket or use [ / ] to inspect its interval. The pinned inspector shows the interval total and exact selected-model count; click a model row to choose which model remains inspected.

## Keyboard and mouse

| Action | Input |
| --- | --- |
| Switch Overview / Models | Tab, Shift+Tab, Left / Right, or click |
| Change period | R or click a date filter |
| Preview another chart | C in Models or click Stacked / Lines / Trends |
| Expand / collapse breakdowns | Ctrl+O or Details / Less |
| Inspect earlier / later day or bucket | [ / ] |
| Scroll body | Up / Down, PageUp / PageDown, mouse wheel |
| Toggle demo / unavailable | D or Sample |
| Close and restore conversation | Escape or Return |

Hover only changes inspection and visual tone. It never changes date filters, tabs, chart preference or saved settings. Title, source label, controls, exact-value inspector and closing hints remain pinned while the information body scrolls.

## Saving a default

Settings → Statistics previews all three designs with the actual Stats painters. Navigate or click to highlight; Enter applies and saves. Space does not apply. Left from a style returns to categories. Pending and applied choices remain distinct; save failures keep the session choice active and report the error.

Switching styles inside /stats is an immediate, sheet-local preview labeled Preview. It does not write preferences. Closing the sheet discards this preview; reopening uses the saved default. Old settings files without stats_chart_style use Stacked, preserving unknown fields and unrelated preferences. Values serialize as stacked, lines or trends.

## Demo gallery

Use /preview or --scenario with stats (unavailable), stats-demo (Overview), stats-stacked, stats-lines, stats-trends or stats-details. For example:

~~~powershell
.\dist\polaris.exe --preview --interactive --scenario stats-stacked
~~~

Use C to compare designs on the same data, R to compare periods, and Ctrl+O for details. No live metric collection, new provider behavior, account access or network requests are introduced.

Build evidence, captures and package checks: [verification report](stats-verification.md).

Solid calendar blocks: [verification and package evidence](calendar-fill-verification.md).
