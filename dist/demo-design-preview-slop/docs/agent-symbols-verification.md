# Agent symbol verification

VERIFIED: the roster uses ● for Main and ◯ for working subagents. Only these two renderer literals changed. Layout, names, colors, remaining status markers, ASCII fallbacks and the header logo are unchanged.

The focused regression failed against the previous Main star, then the complete agent suite passed:

~~~text
cargo test --offline --locked --test agents
test result: ok. 20 passed; 0 failed; 0 ignored
~~~

The regression checks all seven statuses, Main working/idle, ASCII/Unicode, exact symbol width and adjacent name columns at 60x20, 80x24, 120x40 and 160x60. Existing agent interaction tests also passed. Formatting check exited 0.

~~~powershell
cargo fmt --all -- --check
.\scripts\build_verified.ps1
python target/agent-symbols/standalone.py
~~~

The static release wrapper exited 0: 310 top-level tests passed, 0 failed. Two ignored child entry points were executed through their parent tests. [Raw release output](../target/agent-symbols/release.log), [parent-suite totals](../target/agent-symbols/test-summary.json).

Standalone raw output:

~~~text
10 standalone checks passed; Main/worker symbols and ASCII output verified; no preferences written
~~~

These checks used the exact distributed executable in a directory containing only polaris.exe and Windows System32 on PATH. They cover working/mixed scenarios, all four viewport sizes and ASCII. [Raw results](../target/agent-symbols/standalone-results.json).

[Reviewed roster buffer](../target/agent-symbols/agents-main-120x40.png): both circle symbols align with the existing rows. Physical Alacritty appearance and other terminal fonts remain unverified. This is still a UI preview without agent execution.

All four active distribution copies match the tested release and their manifests:

~~~text
46a64f7c8cef701188f46f39d07c1958da640583ef7571647d9fb6e85bc71b2a
~~~

[Package hashes](../target/agent-symbols/distribution-sync.json). Previous executable bytes and matching manifests are preserved under dist/legacy-agent-symbols/d9738bd21edf, retaining original locations; [archive index](../target/agent-symbols/archives.json). The PE gate passed with only Windows system DLL dependencies, including OS UCRT.
