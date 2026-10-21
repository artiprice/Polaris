# S0-10 — crossterm 0.29 key chords on Windows terminals

**RESULT: IN PROGRESS — probe built, manual key test pending (needs a human at the keyboard).**
Labels: VERIFIED / LIKELY / UNVERIFIED

| | |
|---|---|
| Question | crossterm 0.29 on Windows Terminal / ConPTY: Shift+Enter, Shift+Tab, bracketed paste, Alt/Ctrl chords |
| Kill criterion | Shift+Enter unreliable |
| Fallback | Alt+Enter / Ctrl+J for newline; an unsafe exception crate only if those are rejected (ADR-004) |

## Why manual

Whether a chord reaches the app depends on the terminal's translation layer (Windows Terminal, conhost,
VS Code, SSH/WSL). Injecting synthetic input (`WriteConsoleInput`/`SendInput`) would bypass exactly that layer,
so it cannot answer the question. Verified so far: the probe builds (crossterm 0.29.0, 10 s incremental) and
starts, enables raw mode and prompts (VERIFIED in a non-interactive harness, where it timed out as expected).

## How to run (each team member, once per terminal they use)

```powershell
cd @prime\spikes\s0-10-crossterm
$env:CARGO_TARGET_DIR = "$PWD\..\target"; cargo build --release
..\target\release\s0-10-crossterm.exe          # in Windows Terminal, then conhost, then VS Code's terminal
```

Follow the prompts (Esc skips a step; 30 s timeout per step). The probe asks for Enter, Shift+Enter, Alt+Enter,
Ctrl+J, Shift+Tab and a two-line paste, records the raw crossterm events and appends PASS/FAIL lines to
`s0-10-report.txt` in the current directory — keep that file (it is the raw evidence) and copy the verdicts into
the table below.

## Results

| Terminal | Enter | Shift+Enter | Alt+Enter | Ctrl+J | Shift+Tab | Paste (2 lines) | CSI-u push | Reporter |
|---|---|---|---|---|---|---|---|---|
| Windows Terminal | — | — | — | — | — | — | — | pending |
| conhost | — | — | — | — | — | — | — | pending |
| VS Code terminal | — | — | — | — | — | — | — | pending |

Decision rule: Shift+Enter PASS in Windows Terminal → keep it as the newline chord; otherwise Alt+Enter or
Ctrl+J (whichever passes everywhere) becomes the documented newline chord and no unsafe code is added.
