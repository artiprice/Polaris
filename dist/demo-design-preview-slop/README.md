# Polaris

Polaris is a Rust/Ratatui coding-agent interface preview with a full-screen conversation and bottom popups.

> Preview boundary: inference, authentication, provider connections, shell commands, file edits, account lookup, and persistent sessions are not implemented. Provider/model settings exist in memory only. UI appearance preferences save locally. Sample activity, tokens, sessions, and patches are labeled as demo data.

## Web interface design preview

Open [the Polaris web preview](webui/index.html) for the HTML/CSS coding workspace: a black canvas, compact prompt, layered living green atmosphere and dense stars, a spacious black chat with capsule controls and compact file rows, and seven screens with quiet icon navigation. Fonts and assets are local. This is a design prototype; inference and persistence are not connected. See [preview behavior and verification](webui/README.md).

## Standalone Windows executable

Use `dist/polaris.exe` directly:

```powershell
.\dist\polaris.exe --preview --interactive
```

Rust and MinGW support libraries are linked statically. Windows system DLLs, including the built-in UCRT, remain required; no separate Visual C++ or GCC runtime DLLs are shipped. Rebuild and verify offline with `.\scripts\build_verified.ps1`, which runs the release suite serially before packaging. For uncached dependencies use `python scripts/build_static.py --test`. See [distribution details](docs/distribution.md).

## Run

```powershell
cargo run -- --preview --interactive
```

If a running Windows instance locks the default executable, build separately:

```powershell
cargo build --offline --target-dir target/keyboard-fix
.\target\keyboard-fix\debug\polaris.exe --preview --interactive
```

Close the old instance before trying the new binary. Changes cannot update an already running process.

Without terminal input/output the default prints a static 80×24 preview; --interactive requires a TTY. Use --dump WIDTHxHEIGHT for a deterministic text render.

```powershell
cargo run -- --scenario models --dump 80x24
cargo run -- --preview --theme terminal --ascii --reduce-motion
```

## Layout

The CLI is trying a full-width layout on black, with two-cell outer margins and no 100-cell cap. A small North Star app bar shows regular-weight Polaris, a gray regular-weight v0.1.0alpha, a clipped workspace path and an optional local account label. The welcome body centers an unfilled North Star and greeting; conversation keeps only the app bar. The star has a #C0E58C center, #76B900 rays and gray tips, with ASCII/no-color fallbacks. See [header geometry and verification](docs/header-logo.md) and [branding history](docs/branding/README.md).

The bottom composer has thin top/bottom rules and open sides and grows to at most 37.5% of the terminal height, with additional room reserved for history and status. Its empty placeholder is dark gray and excluded from copying. Goal status sits right-aligned above it in blue. After one blank row, the footer offers compact [ mode on ], [ model ] and [ effort ] controls. Auto remains magenta, plan gold, and metadata gray. Source-colored selection retains the 30% tint; completion hover remains foreground-only. Shared controls have bounded dark focus/hover surfaces. Windows VT sessions use RGB automatically, including double-click launches; explicit color options and terminal-native styling remain available. See [color policy](docs/color-policy.md).

## Keymaps

| Key | Action |
| --- | --- |
| Enter / Return in composer | Submit |
| Shift+Enter | Insert a newline |
| Up / Down at the first / last visual composer row | Browse older / newer prompts; Down past newest restores the draft |
| Shift+Tab | Toggle auto / plan |
| Shift+Up / Shift+Down in composer | Increase / decrease active model effort immediately |
| Alt+Enter, Ctrl+Enter, Ctrl+J | Additional newline bindings |
| Esc during work | Interrupt the response after dismissing any active popup, menu, preview or selection; preserve the draft |
| Esc in any popup | Return to the previous view without applying a selection |
| Enter / Return in information popups | Close status, usage, stats, diff, transcript, or help |
| Enter / Return in a selection menu | Confirm the selected item; /model applies the model and closes |
| ? with an empty draft | Open shortcuts |
| PageUp / PageDown | Scroll conversation or popup content |
| Mouse wheel over `/`, `@`, or `$` suggestions | Navigate and scroll the list without moving the conversation |
| F2 | Open status details |
| F3 / /agents | Open or focus the agent roster; Enter views a conversation |
| Ctrl+O | Expand/collapse all inline tool details (click a title to toggle one) |
| Ctrl+C | Interrupt demo work, close a popup, clear a draft, or confirm exit with two presses when empty |
| Tab / Shift+Tab in provider form | Next / previous field |
| Ctrl+S in provider form | Save settings in memory |
| P / C in /model | Choose / configure provider |
| Left / Right in /model | Adjust low / medium / high / xhigh / max effort |
| Tab or Left / Right in /stats | Switch Overview / Models |
| R in /stats | Cycle All time / Last 7 days / Last 30 days |
| C in /stats → Models | Preview Stacked / Lines / Trends |
| Ctrl+O in /stats | Expand / collapse token detail |
| [ / ] in /stats | Inspect earlier / later day or bucket |
| D in /stats or /usage overview | Toggle labeled demo data / empty state |
| Ctrl+O in /usage | Expand cache and subagent details |
| Tab / Shift+Tab in /settings | Switch category / control focus |
| Arrows / click in /settings | Highlight choices; Left at the first variant returns to categories |
| Enter in /settings | Apply the highlighted choice and autosave; Space does nothing |

On Windows, the session enables VT input so ConPTY preserves enhanced key sequences. A native record reader preserves Escape, Return, and Tab even when their virtual key code is zero. The decoder handles CSI-u, reverse-tab, navigation keys, and bracketed paste. Paste is inserted as text without submitting embedded newlines. The original console mode is restored on exit; terminal configuration is not changed.

If a legacy Enter/Tab record loses Shift, the fallback samples only the currently held Shift state for that received key. This can miss Shift after release; CSI-u provides an explicit modifier and is the preferred path. The app requests enhanced keyboard reporting and pops that request on exit. Physical key reporting in the user's Alacritty window remains a manual check.

For a local diagnostic capture:

```powershell
cargo run -- --preview --interactive --key-trace target/keys.jsonl
```

The trace path must be new. It records key kinds, modifiers, TTY detection, and application state; printable draft characters are replaced with `Text`. It does not log draft contents. Use a different filename for each run.

## Models and providers

NVIDIA NIM appears first, followed by Anthropic, OpenAI, and Local. Three static NVIDIA reference models appear immediately: Nemotron Super 49B v1.5, Nemotron Nano 9B v2, and Nemotron Nano 8B v1. Source references: [Super 49B](https://build.nvidia.com/nvidia/llama-3_3-nemotron-super-49b-v1_5), [Nano 9B](https://build.nvidia.com/nvidia/nvidia-nemotron-nano-9b-v2), and [NVIDIA catalog](https://build.nvidia.com/models?label=Instruction+Following&nim=hosted). Names do not establish endpoint availability.

Each model appears once with a current marker. Wide popups include short descriptions. A white Provider · Change control sits below the models. Effort preferences are remembered per model in memory and labeled as preview preferences; backend support for these levels is not claimed. Provider form groups have a blank row between them; credential fields accept references/environment variable names rather than requesting API keys.

## Statistics

The rebuilt [Stats sheet](docs/stats.md) has an activity calendar, session highlights and three model visualizations: Stacked, Lines and Trends. All three use the same data and date filters. C previews chart styles inside /stats → Models; Settings → Statistics saves the default only after Enter. Ctrl+O reveals token details, and mouse hover/click or [ / ] inspects dates and intervals. Charts and calendars wrap without dropping dates on small terminals. Live collection remains unconnected; sample data is explicitly labeled.

## Options and commands

Options include --scenario NAME, --color-depth auto|truecolor|ansi, --theme dark|terminal, --ascii, --reduce-motion, --color auto|always|never, --account-email EMAIL, --dump WIDTHxHEIGHT, --interactive, --key-trace PATH, --help, and --version. NO_COLOR is respected in automatic color mode. Below 60×20 the app asks for a larger terminal while preserving its draft and navigation.

Completed responses show a muted worked-duration and local completion clock. Code blocks have numbered gutters and lexical syntax colors; diffs have full-width green/red change surfaces. Ordinary messages stay neutral, warnings use amber, and errors use muted #AC4242 red with explicit type labels and regular weight. The `/preview` gallery includes a labeled messages demonstration.

The [Usage sheet](docs/usage.md) provides session/context/limit summaries, model rows and expandable cache/subagent detail, with Strip bars by default. Highlight Strip, Ticks or Segments in [Settings](docs/settings.md), then press Enter to apply and save. Navigation and clicks only highlight choices. All sample metrics are labeled; real usage sources remain unconnected.

Commands: /agents, /preview, /settings, /model, /sessions, /status, /usage, /stats, /diff, /help, /providers, /onboarding, /clear, /quit. /preview lists the available scenarios.

See [the agent work view](docs/agents.md) for the bottom roster, conversation switching and labeled preview scenarios. The composer always targets Main.

## Dollar settings

Use `$websearch` or `$subagents` to enable a session setting; add `=off` to disable it. Lists accept either comma-spacing style: `$subagents-model=[model1(low-xhigh), model2(high-max)]`, `$rag=[docs,source]`, `$skills=[lint,review]`, `$links=[https://example.com, https://example.org]`. Values stay in memory until changed, including when mixed into multiline prompts, and appear in `/status` and suggestion details. Dollar suggestions use blue selection and lighter blue hover. A settings-only entry starts no reply. These are preview settings; services are not connected. See [the syntax and goal guide](docs/prompt-settings.md).

## Composer

The composer uses a steady cursor and commits complete changed frames, preserving its insertion position during middle edits. Idle and identical frames emit no terminal output. See [rendering behavior](docs/rendering.md) for scheduling, caching and terminal fallbacks. It supports text and image clipboard paste, attachment previews, mouse selection across conversation messages with copy on release, double-click word selection and word-by-word dragging while holding the second click, and workspace-file completion. Long pastes use `[Pasted Content M chars]` labels. Sent prompts use a #141414 surface without a role heading; trailing blank rows are removed while interior paragraphs remain. Up/Down browses session prompt history at the first/last visual editor row and restores the unsent draft after the newest entry. Assistant replies also omit their role heading. Completion hover uses a lighter foreground tone than keyboard focus. Colored text selection preserves each source color with a faint matching background tint; spaces use a neutral tint, ANSI/terminal modes use neutral index 237, and no-color mode uses bold. A centered `To latest` button returns scrolled conversation history to the bottom. Its dark-gray background brightens on hover. See [the composer guide](docs/composer.md) for key bindings, limits, completion behavior, and current preview boundaries. The [design guide](docs/ui-design.md) describes the surrounding interface.

## Verification

The repository’s local check commands are:

```powershell
cargo test --offline
cargo clippy --offline --all-targets -- -D warnings
cargo fmt --all -- --check
```

Physical key reporting in the user’s Alacritty window, live resizing, and terminal-specific font appearance require manual verification. See [the design guide](docs/ui-design.md) and [keyboard verification evidence](docs/keyboard-verification.md).
