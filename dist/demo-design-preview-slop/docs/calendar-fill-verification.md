# Solid activity-calendar verification

VERIFIED: Stats Overview now paints touching full-cell background blocks rather than small foreground marks. Valid inactive days use RGB #121212; the four activity greens and thresholds remain unchanged. Out-of-range padding stays black. Wide days occupy both existing terminal columns, including the former spacer. The legend shares the tile painter.

Hover, selection and their combined state blend 20%, 35% and 45% white into the RGB activity color. ANSI and terminal-native modes use indexed background ramps. ASCII keeps filled backgrounds with color enabled; disabled color retains numeric intensity levels. No settings or data schema changed.

## Release evidence

Commands:

~~~powershell
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
.\scripts\build_verified.ps1
python target/calendar-fill/standalone.py
~~~

Formatting and Clippy exited 0. The verified wrapper exited 0 and passed 309 top-level release tests, with 0 failures. Two intentionally ignored child entry points were executed by parent tests in isolated consoles. Parent-suite counts avoid counting the child probes twice. Raw output: [release log](../target/calendar-fill/release.log); [suite totals](../target/calendar-fill/test-summary.json).

The new unit tests cover every session threshold, exact RGB colors, interaction shades and black date padding. Stats view tests check every date and column across 60x20, 80x24, 120x40 and 160x60 in RGB, ASCII, ANSI and terminal-native modes. They verify hover, click selection, exact boundaries and neighbor preservation. The real ConPTY calendar test checks all visible day backgrounds and glyphs in RGB, ANSI, terminal-native and disabled-color profiles, including normal shutdown. The full release includes 28 passing ConPTY tests.

Standalone raw output:

~~~text
14 standalone checks passed; no preferences written
~~~

The exact packaged executable launched from a directory containing only polaris.exe, with only Windows System32 on PATH. Version/help, four viewport sizes, ASCII, ANSI, terminal-native and no-color dumps exited 0. [Raw launch results](../target/calendar-fill/standalone-results.json). Plain-text dumps do not encode background colors; the buffer and ConPTY tests verify those separately.

## Visual review

Reviewed actual Ratatui buffers rendered with a local monospace font:

- [120x40 touching grid](../target/calendar-fill/calendar-solid-120x40.png)
- [60x20 compact view](../target/calendar-fill/calendar-solid-60x20.png)
- [hover](../target/calendar-fill/calendar-solid-hover-120x40.png)
- [selected](../target/calendar-fill/selected-review.png)

The full grid has no internal background gaps and the legend uses the same five shades. Short terminals preserve the existing scrolling layout; automated traversal checks all 365 dates. These are buffer renders, not physical Alacritty screenshots.

## Package consistency

All four active package directories contain identical 8,155,648-byte executables and matching manifests/checksums:

~~~text
d9738bd21edfd6e0f5c8a410e769873905c0a1c088b33096d563bb7ad80f5339
~~~

The packaged bytes match the tested GNU release executable exactly. The PE gate permits only Windows system DLLs, including OS UCRT; Rust/MinGW support is statically linked. [Package checks](../target/calendar-fill/distribution-sync.json). Previous executables and matching manifests are preserved under dist/legacy-calendar-fill/0f1039c73805, retaining their original locations. [Archive index](../target/calendar-fill/archives.json).

All 26 protected web UI files match their prior hashes. No chart, settings, source normalization or font policy changes were required. Source/test/doc snapshots are in target/calendar-fill/before; [combined evidence](../target/calendar-fill/verification.json).

## Not verified

Physical Alacritty appearance, other Windows versions and clean virtual machines were not checked. Stats remains a UI preview: demo values are labeled DEMO DATA and no live metrics collector is connected.
