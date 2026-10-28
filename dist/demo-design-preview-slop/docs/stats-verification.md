# Stats rebuild verification

VERIFIED on 2026-10-21 against this checkout and the exact packaged Windows GNU release. Stats remains a design/demo preview; no live activity source was added.

## Final gate

~~~powershell
.\scripts\build_verified.ps1
~~~

Exit 0. The final log contains 304 passing top-level tests, zero failures and two ignored child entry points. Parent tests invoke those entry points in isolated consoles. Relevant raw output:

~~~text
test result: ok. 27 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 15.18s
test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.90s
~~~

These are respectively the complete ConPTY suite, normalized Stats data tests and Stats view tests. [Raw release output](../target/stats-rebuild/release.log), [gate exit result](../target/stats-rebuild/release-result.json).

~~~powershell
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
~~~

Both exited 0. Clippy output:

~~~text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.41s
~~~

## Coverage

- Totals reconcile across disjoint token categories, model rows and daily/weekly buckets. Cases include leap days, inclusive windows, streak gaps, duplicate session IDs, unknown models/categories, zero, values beyond 32-bit range, overflow and more than five models grouped into Other.
- Every calendar date remains reachable at 60x20, 80x24, 120x40 and 160x60. Chart panels retain every bucket. Closing hints, controls, source badge and exact-value inspector fit and remain pinned while content scrolls.
- All three charts have Unicode, ASCII, no-color and ANSI layout coverage; terminal-native and reduced-motion views are covered separately. Limited-color calendars retain numeric intensity levels. Tiny/missing/overlapping series and distinct hover/selected shades have regression cases.
- Keyboard/mouse tests cover tabs, ranges, styles, Details, inspection, body scrolling and overlay geometry. Opening/closing Stats preserves Main's draft/cursor, scroll, inspected agent and ongoing response. Stats and Usage demo flags are independent.
- Preference tests cover highlight versus Enter, Space doing nothing, Left returning to categories, default restoration on reopen/restart, old JSON, unknown fields and save failures. The full existing Settings/Usage/model/composer suites remain included.
- Two new real ConPTY tests send terminal bytes for /stats during a response, all chart styles, Ctrl+O, interval inspection, SGR clicks/wheel, paging, Escape, Settings confirmation and saved-style restart. Neither relies solely on synthetic application key events.

Old assertions were updated for three approved behavior changes: Stats now reserves 32 rather than 28 rows; its demo flag is independent of Usage; and its normal state says Statistics unavailable. Their conversation-preservation, navigation and source-label checks were retained. Settings category assertions now follow the inserted Statistics category, preserving the tests for Interaction, Accessibility and Providers.

## Visual review

Actual Ratatui buffers were exported to SVG. Review PNGs use Windows GDI and Cascadia Mono for dependable glyph coverage; this does not choose or change the application's terminal font. Inspected captures include:

- [Overview, 60x20](../target/stats-rebuild/stats-overview-60x20-final.png)
- [Overview, 120x40](../target/stats-rebuild/stats-overview-120x40-final.png)
- [Stacked](../target/stats-rebuild/stats-stacked-summary-120x40-final.png)
- [Lines](../target/stats-rebuild/stats-lines-summary-120x40-final.png)
- [Trends](../target/stats-rebuild/stats-trends-summary-120x40-final.png)
- [Statistics settings](../target/stats-rebuild/settings-statistics-120x40-final.png)

The narrow footer clipping found during review was corrected and now has an explicit closing-hint assertion. Small calendars use balanced panels; their partial first/last months are also identified by complete panel date ranges.

## Package and standalone checks

The release gate verified Windows-only PE imports before packaging. Exact package:

~~~text
bytes: 8152064
sha256: 0f1039c73805654a9134f566948207d8d6e1e65217915f858b6ca8eee06185bb
~~~

All four active distribution locations have identical executables, matching checksums and matching manifests. [Distribution synchronization](../target/stats-rebuild/distribution-sync.json). The previous agent-view release and matching metadata were preserved under dist/legacy-stats/d7c037263ad4; [archive record](../target/stats-rebuild/archives.json).

The exact packaged file was copied into a directory containing only polaris.exe and launched with only Windows System32 on PATH. Version/help, all six Stats scenarios at four sizes, plus ASCII/no-color/terminal-native checks passed:

~~~text
35 standalone checks passed; no preferences written
~~~

[Standalone commands and captured output](../target/stats-rebuild/standalone-results.json).

## Consistency and limits

README, Stats, Settings, UI design and distribution documentation were updated. The /model picker retains its prior behavior. All 26 existing webui files match their pre-task SHA-256 values. Existing unrelated work was retained.

Physical interaction and font appearance in the user's Alacritty window remain unverified. There is no live collection, provider/account integration, statistics database, pricing data or new network access. Demo values are explicitly labeled; normal Stats remains unavailable. The release still requires Windows system DLLs and the OS UCRT, as documented in build-info.json.
