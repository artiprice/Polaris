# Rendering refactor verification

VERIFIED: demand-driven frame scheduling, shared transcript/editor layouts, in-memory hit-map refreshes, buffered output and a steady cursor are implemented. The accepted screen design is preserved. See [rendering behavior](rendering.md) for the cache and terminal policy.

## Commands and release result

~~~powershell
cargo fmt --all -- --check
cargo clippy --offline --locked --all-targets -- -D warnings
.\scripts\build_verified.ps1
python target/render-refactor/standalone.py
~~~

Formatting and strict Clippy exited 0. The static release wrapper exited 0. Parent-suite totals are **322 passed, 0 failed**, including 29 passing ConPTY tests. Two intentionally ignored child entry points are executed through their parent tests in isolated consoles. Child probe summaries are not double-counted. [Raw release output](../target/render-refactor/release.log), [parent-suite totals](../target/render-refactor/test-summary.json).

Standalone raw output:

~~~text
42 standalone checks passed; no preferences written
~~~

These checks launch the exact packaged executable from a directory containing only polaris.exe, with only Windows System32 on PATH. They cover version/help, seven screens at 60x20 through 160x60, ASCII, ANSI, terminal-native and no-color. [Raw launch results](../target/render-refactor/standalone-results.json).

## Measured performance

The release-mode diagnostic uses the production UI and cursor backend with a fixed 120x40 TestBackend. It runs three times; every case has a fresh thread/cache, one separately timed cold draw and 20 warm draws. Typing includes the edit, scroll-bound preparation and complete draw. Hover moves over unchanged blank content. This measures CPU work, not monitor presentation latency.

| 1,000-entry case | Warm median across runs | Warm p95 across runs |
| --- | --- | --- |
| Identical | 0.317–0.363 ms | 0.353–0.569 ms |
| Typing | 0.313–0.351 ms | 0.317–0.506 ms |
| Unchanged hover | 0.314–0.333 ms | 0.377–0.429 ms |

The previous repeated identical-frame probe measured 125–162 ms median with the same representative two-line entries. [Original raw output](../target/render-audit/cpu-probe.log), [original repeated runs](../target/render-audit/cpu-repeat-results.json). The final warm typing p95 meets the 16.7 ms target in all three runs. Cold 1,000-entry preparation remains about 68–76 ms; width/theme changes still require reflow. [Final raw benchmark output](../target/render-refactor/performance-results.json), [benchmark source](../target/render-refactor/cpu_probe.rs), [build command](../target/render-refactor/performance.py).

For 20 identical frames the backend recorded zero paints, cursor moves, hides and reveals. Its logical flush method is still called by Ratatui, but the buffered writer commits zero bytes for empty output; the writer regression checks the actual downstream write/flush counts. Cursor-only movement produces one move without visibility toggles.

## Input and output evidence

The scheduler tests use a fake monotonic timeline to check the 60 fps ceiling, pending work, idle behavior and event ordering. Semantic batching retains keys, paste, clicks, drag/release and wheel boundaries while coalescing ordinary moves and resize notifications. Cache tests check entry reuse, individual changes/expansion, width/theme invalidation and shared editor wrapping. Existing source-copy, Unicode, selection tint, attachment, popup, agent and navigation assertions remain in force. String equality assertions now borrow Arc<str> sources without changing expected text.

The new ConPTY regression sends 1,000 complete mouse packets at a nominal 1 ms interval in each supported/unsupported synchronization case. It asserts no additional frame construction and **zero captured terminal bytes** for unchanged hover, then verifies middle insertion and a normal exit. A separate decoder test consumes 1,000 packets instantaneously without producing any text. Split and late capability responses are covered in the decoder and terminal-policy tests, including preserving response-shaped pasted text.

An initial unpaced ConPTY firehose lost SGR prefix bytes before they reached KeyboardDecoder. Fixture-only instrumentation established the missing prefixes and was removed from production source. The transport test therefore uses the sustained 1,000 Hz schedule; the instantaneous test directly covers the decoder. Arbitrarily unpaced OS pipe delivery is not a guaranteed lossless input channel. No assertion was relaxed to accept text leakage or redraws.

Presentation tests verify complete buffered commits, balanced synchronization, no-op output, correct final cursor coordinates, hidden popup cursors and cleanup after draw/position/partial-write/flush errors. Cursor policy tests cover supported, unsupported, unanswered and late replies and restoration without forcing an originally steady cursor to blink.

## Visual and package consistency

All **275** common exported SVG buffers are byte-for-byte identical to the preceding agent-symbol release. [Comparison](../target/render-refactor/capture-comparison.json). Reviewed [conversation](../target/render-refactor/conversation-120x40.png) and [tall editor](../target/render-refactor/long-draft-120x100.png) are actual Ratatui buffers rendered with a local monospace font, not physical-terminal screenshots.

All four active distribution directories contain identical **8,209,408-byte** executables and matching checksum/build manifests. The bytes match the tested GNU release exactly:

~~~text
64d48a82db889561d1aaea20b45df585f994bfcf71f5a38061804e757dda0587
~~~

The PE gate permits only Windows system DLL imports, including OS UCRT. [Package checks](../target/render-refactor/distribution-sync.json). Previous packages and their own manifests are preserved under dist/legacy-rendering/46a64f7c8cef, retaining original locations. [Archive index](../target/render-refactor/archives.json).

All 26 protected web UI files match their prior hashes. Source/test/doc snapshots are in target/render-refactor/before; [combined machine-readable evidence](../target/render-refactor/verification.json).

## Not verified

Physical Alacritty and double-clicked Windows console appearance, other terminal fonts, clean virtual machines and other Windows versions were not inspected. Cursor-control bytes and ConPTY coordinates/visibility are verified; these are not a physical blinking/display-latency measurement. Cold reflow and the unpaced ConPTY transport limit remain as described above. The app remains a local UI preview without provider or agent execution.
