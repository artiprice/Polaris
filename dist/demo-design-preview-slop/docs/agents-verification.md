# Agent work view verification

VERIFIED against this checkout and its packaged Windows GNU release on 2026-10-21. This report covers the local UI preview, not a live subagent backend.

## Commands and raw evidence

~~~powershell
cargo test --offline --locked --test agents
~~~

~~~text
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
~~~

~~~powershell
cargo test --offline --locked --test conpty_smoke conpty_agents_keyboard_and_mouse_preserve_main_draft_and_route_focus -- --nocapture
~~~

~~~text
test conpty_agents_keyboard_and_mouse_preserve_main_draft_and_route_focus ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 25 filtered out
~~~

cargo fmt --all -- --check and cargo clippy --offline --locked --all-targets -- -D warnings exited 0. The earlier full development suite passed; the final static release reran the complete suite after the final source changes.

~~~powershell
.\scripts\build_verified.ps1
~~~

Exit 0. The final release log contains 287 passing top-level tests, zero failures and two ignored child entry points; their parent tests invoke them in isolated consoles. In particular:

~~~text
test result: ok. 19 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s
test result: ok. 25 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 14.50s
~~~

Raw complete output: [release.log](../target/agents-review/release.log). Build gate result: [release-result.json](../target/agents-review/release-result.json).

## Behavioral coverage

The focused suite covers automatic expansion, manual collapse, terminal retention, stable IDs, unknown metrics, independent timer updates after Main finishes, clearing state, draft/cursor/attachment/undo preservation, independent conversation scroll and tools, accepted/rejected Main submissions, overlays and Escape priority, scoped roster keyboard focus, paste focus, copy source isolation, inline and roster mouse targets, hover, wheel, overflow, resize, To latest, goal/approval layout and ASCII fallback.

The new ConPTY smoke test sends actual terminal byte sequences for F3, arrows, Enter, editing inside the preserved draft, Ctrl+O, F2 status, Escape, wheel and clicks. It checks both state diagnostics and terminal output. The complete release suite retains the existing keyboard, selection, clipboard, Settings and Usage regressions.

Ratatui layout tests cover 60x20, 80x24, 120x40 and 160x60. SVG captures were rendered with rsvg-convert and visually inspected for Main, active agents, mixed/failed statuses, a narrow inspected conversation and the smallest goal/approval fallback. Images: [Main roster](../target/agents-review/agents-main-120x40.png), [active roster](../target/agents-review/agents-working-120x40.png), [mixed statuses](../target/agents-review/agents-mixed-roster-120x40.png), [narrow thread](../target/agents-review/agents-many-thread-60x20.png), [short approval](../target/agents-review/agents-approval-60x20.png).

## Distribution and consistency

The exact executable built and exercised by the release gate is packaged as dist/polaris.exe. PE import verification accepts only Windows system DLLs and OS UCRT contracts.

~~~text
bytes: 8058880
sha256: d7c037263ad427e159ce0e7dae25b7539f8c6d74e69fd98aaa8c48a6cd32be24
~~~

The three active sibling distributions contain identical executable bytes, checksums and manifests. [Copy hashes](../target/agents-review/distribution-sync.json). Previous packages and matching manifests were preserved under dist/legacy-agents/5081032d89de, grouped by original location; [archive record](../target/agents-review/archives.json).

The exact packaged executable was copied to a directory containing only polaris.exe and run with only Windows System32 on PATH. Version/help, welcome and all four agent scenarios were checked at four sizes, plus ASCII, no-color and terminal-native cases. Output:

~~~text
25 standalone checks passed; no preferences written
~~~

[Standalone commands and captured output](../target/agents-review/standalone-results.json).

README, command/help listings, preview scenarios, UI design and distribution documentation were updated. All 26 pre-existing webui files have the same SHA-256 as before this task. Other uncommitted user changes were retained.

## Not verified / deliberately outside scope

No physical keyboard/mouse session in the user's Alacritty window was exercised. Font rendering and terminal-specific presentation remain manual checks. There is no live agent execution, provider telemetry, persisted agent session history or task-control backend. Demo tasks and optional metrics are visibly labeled; enabling $subagents alone starts nothing. No approval or execution controls were added to agent conversations.
