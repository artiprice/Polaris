# Polaris — CLI UI implementation plan

> **Updated:** 2026-10-25 (continuation of 2026-10-24 planning) · **Status:** implementation in progress; coverage is partial and no UI phase acceptance is claimed.
> **Companions:** [UI_PHASES.md](UI_PHASES.md) (execution order and gates), [UI_MODULES.csv](UI_MODULES.csv) (module inventory),
> [PLAN.md](PLAN.md) (system architecture), [PHASES.md](PHASES.md) (system delivery), [prompt.md](prompt.md) (engineering rules).

## 0. Scope, confidence and definition of done

This document specifies a production CLI/TUI derived from dist/demo-design-preview-slop. CLI implementation is underway in model/renderer/terminal modules and the offline app integration; the code present so far covers only slices of the planned modules. No UI phase or M1 acceptance is complete. Web and desktop remain deferred. Only the web **welcome page** contributes web design ideas here; its future optimization is a distinct deferred workstream (§10).

Planning is complete when design provenance, safe implementation boundaries, module ownership,
main-phase dependencies, acceptance criteria, failure handling and deferred work agree across these
three UI artifacts and the two main planning documents. Implementation completion will require the
gates in UI_PHASES; writing this plan satisfies none of them.

**Evidence vocabulary:** the original design-analysis sections use `VERIFIED` for inspected source/document state. Runtime claims are confined to dated implementation evidence with exact commands/output. `LIKELY` is an inference; `UNVERIFIED` names missing evidence.
All prescriptions, module paths not present today, resource limits and budgets are **proposed
requirements**, not claims of implementation or measured performance. The CSV status records observed UI implementation coverage; acceptance remains open until its full criteria pass. Existing source is a reuse candidate, not an automatic PASS.

### 0.1 Failure modes and entry assumptions

| Failure mode | Required response / condition for proceeding |
|---|---|
| Copying unsafe demo transport or panic-prone frame code | Reimplement behavior with safe adapters; no handwritten Win32 FFI. Prove the fallback input path before claiming keyboard support. |
| Pretty UI invents backend progress, account access or completion | Display committed kernel facts or explicit pending/unavailable state. Model prose cannot mark work Completed. |
| Missing events/payload APIs make a screen impossible | Freeze an actual typed contract with its owner first. Keep that capability unavailable; do not fabricate a production adapter. |
| Slow terminal or transcript rebuild blocks cancellation/kernel work | Bounded queues, fair scheduling, paging, independent kernel ownership and cancellation; enforce measured budgets. |
| Malicious output executes terminal controls or spoofs an approval | Escape untrusted controls before painting; authoritative approval surfaces have separate typed state and identity. |
| Old model tokens and current demo disagree | Deliberate migration with new assertions and reviewed snapshots (§2.2); no silent edit of an existing check merely to get green. |
| Terminal setup partially succeeds or output fails mid-frame | Track acquired modes, explicitly close with errors, use best-effort guard cleanup for unwind; never promise restoration after hard kill. |

**VERIFIED premise:** local design sources and real shared UI-model source exist. This is sufficient
to plan and begin a CLI. Terminal behavior on physical terminals, full safe-wrapper coverage, end-to-end payload resolution and performance remain **UNVERIFIED**; UI0 gates remain open for those claims.

### 0.2 Binding rules and precedence

Follow prompt.md's HA > HFT > DOD > metaprogramming > HPC order and its no-stub/API-registry/test-first
requirements. Preserve the main plan's explicit decisions:

- **D10:** add no lint attributes or lint configuration. Apply the prompt's semantic prohibitions
  through design, review and the repository's existing on-demand checks; do not paste its example
  `#![forbid(...)]` blocks into source or manifests.
- **ADR-001:** use the established stack and dependency policy. New dependencies/features/edges go
  through `tools/workspace.toml`, vet/deny/audit and layer checks, never ad hoc manifest drift.
- **ADR-003:** UI layout is outside the closed zero-allocation core set. UI allocations must be
  bounded and profiled; rendering must never add allocation/blocking work to bus/SSE/log hot paths.
- **ADR-004:** this CLI plan selects **zero own unsafe**, including tests and terminal helpers.
  Do not take the optional console exception. If safe wrappers cannot preserve a feature, retain
  documented fallbacks or flag an unmet requirement for a later explicit architecture decision.
  Vetted dependencies' internal unsafe is not a claim that the entire dependency tree is unsafe-free.
- **D3 / ADR-006:** retain unwind supervision and process isolation. Write no deliberate production
  panic, `unwrap`, `expect`, unchecked indexing/cast/arithmetic, `todo!` or ignored fallible result.
  Use checked conversions, bounded iteration, explicit Option branches and typed thiserror errors.
- **D11:** `$` settings are session-scoped and persist via kernel events. UI preferences are a
  different concern. Neither plan/auto nor a mouse click can bypass capability verification.
- Diagnostics use the redacting tracing pipeline. Terminal output uses an owned fallible writer,
  not debug-print macros; JSONL headless output stays machine readable and separate from logs.

Current implementation status is recorded below and in UI_PHASES/UI_MODULES. The historical P1 non-UI verdict stays intact; it is not UI acceptance.

## 1. Inspected sources and current implementation snapshot

Paths in this table are repository-relative; the demo remains read-only reference material.

| Source | VERIFIED observation / use |
|---|---|
| `@prime/prompt.md`; PLAN §§2, 6.4–6.5, 6.16–6.17, 7, 9, 11–13; PHASES P1/P2/P3/P4/P9/P10 | Safety policy, CQRS boundaries, crate owners, command grammar, phase and acceptance anchors. |
| `@prime/P1_LOGIC_STATUS.md`; `@prime/PHASES_STATUS.md` | Historical non-UI acceptance explicitly excludes full M1/TUI. Status evidence must not be rewritten as UI acceptance. |
| `crates/polaris-ui/crates/model/src/{composer,completion,classify,keymap,reduce,view,tokens,logo,cockpit,questionnaire,session}.rs` and sibling tests/descriptors | Shared model provides bounded composer/history, completion, session projection, panel/questionnaire state, tokens and keymap. Fresh model tests are linked in §1.1; full UI acceptance remains broader. |
| `crates/polaris-ui/crates/cli/src/{lib,layout,screen,text,input,scheduler,presenter,terminal}.rs` | Pure layout/screen/text geometry plus safe input, scheduler, buffered presenter and terminal guard modules are implemented. The app crate owns terminal-loop composition; module code and crate tests alone do not prove the complete interactive behavior. |
| `apps/polaris-cli/src/{app,tui,session,key_trace,doctor,rtk,sandbox}.rs` | Default TUI/dump dispatch, real local session/directive commit-replay, controller and key trace are now present alongside headless operations. Preserve existing command/output contracts; evidence and remaining limits are in §1.1. |
| `crates/polaris-sdk/src/contracts.rs`; model `reduce.rs` | Current events include approval/blast-radius/proof outcomes. The reducer currently ignores some execution/approval events; complete roster, streaming text and payload materialization must not be assumed available. |
| Demo `README.md`, `docs/ui-design.md`, `docs/header-logo.md`, `docs/branding/north-star-palette.json`, `docs/color-policy.md` | Active minimal North Star, full width, restrained colors and portable modes. Older branding files are historical. |
| Demo `docs/composer.md`, `docs/rendering.md`; `src/frame.rs`, `src/presentation.rs`, `src/ui/cache.rs`, input/output adapters | Editing/selection semantics, invalidation, caching, cursor staging and transactional presentation ideas. Source also contains unsafe Win32, unwrap/fallbacks, unchecked time arithmetic and discarded cleanup errors; do not copy it. |
| Demo `docs/{agents,settings,usage,stats}.md` | Roster routing, candidate-versus-applied settings, missing-data semantics, responsive sheets and exact-value inspection. Demo evidence reports are historical assertions, not evidence for Polaris. |
| Demo `webui/index.html`, welcome-related rules in `webui/styles.css`, referenced local assets | Welcome composition, font families, black composer, green atmosphere and star layers. No other web page is a design requirement for this plan. |

The design reference was inspected from source and saved buffers. Current native ConPTY evidence is recorded in §1.3, including observable output cleanup; pure guard tests check restoration calls. Direct Windows raw/mouse OS-mode inspection, physical-terminal appearance, a manual keyboard matrix and the Linux terminal leg remain unverified. Do not inherit the demo's claimed test counts, provider list, clocks or quota values.


### 1.1 Historical foundation snapshot (2026-10-24 to 2026-10-25; superseded by §1.3)

**VERIFIED tests and source:** the final offline package run covered the UI model, UI CLI, kernel directives and app. It exited 101 because `doctor_installs_aws_lc_provider` requires setup that is missing in this environment; every other target passed. The app library had 18 passing tests, key trace 3, sessions 3, startup 5, ConPTY 7, RTK 2, and headless 2 passed with the one doctor test failing. UI CLI had 37 passing tests; model had 143; directives had 46 passed and one ignored throughput workload. The active-only CAS materialization regression now passes; its pre-fix behavioral failure is preserved in [active-payload-red.json](evidence/ui/active-payload-red.json). The CAS payload-poll cancellation and failed-owner cleanup cases also passed. Raw full-run evidence: [final-tests.json](evidence/ui/final-tests.json), [stdout](evidence/ui/final-tests.stdout.txt), [stderr](evidence/ui/final-tests.stderr.txt). No UI phase or M1 acceptance is implied by these focused package results.

- [Final strict safety check](evidence/ui/final-safety.json) exited 0 for UI model, UI CLI and app, all targets, with `--no-deps` and the five requested lints. Its raw output is [stdout](evidence/ui/final-safety.stdout.txt) and [stderr](evidence/ui/final-safety.stderr.txt); non-fatal style warnings remain. This is not a whole-repository or dependency-tree safety pass.
- [Dependency-policy subset](evidence/ui/dependency-policy.json) passed licenses, bans and sources. The full advisory check could not acquire its read-only cache lock; see the final verification index for the exact raw attempt.
- [Current verification index](evidence/ui/verification.txt) is the root-maintained summary. Do not treat it or this snapshot as a complete UI phase verdict.

**Test-first evidence and remaining gaps:** meaningful reds were observed for controller frame clear (4,923 emitted bytes), queue pressure, resume race, help capability filtering, text-chip submission and edit overflow; terminal input coalescing and Ratatui style-reset output also had observed reds before fixes. The active-only CAS test’s observed failure is recorded above. These do not fill the missing initial broad red gate for model/renderer/terminal work. [model-initial-red.txt](evidence/ui/model-initial-red.txt) is a missing-API compile failure, not a behavioral assertion red. Keep that limitation explicit.

**Not verified or not implemented:** the doctor readiness failure remains an environment prerequisite failure. Windows ConPTY does not directly inspect native raw-mode/mouse-capture state; manual keyboard matrix and Linux terminal leg remain absent. No performance baseline/benchmark or measured cold-service allocation profile exists. P3 has only pure questionnaire state: no human-request SDK/kernel/frame backend, durable answer/cancel transition or requester-interruption integration. The text clipboard process adapter, bounded workspace snapshot worker and per-field global preference owner now exist; their focused tests avoid native clipboard access. Real OS clipboard success/timeout/unsupported behavior, selection, complete settings restart/corrupt-config screen handling and visual integration remain unverified. Provider/model execution remains incomplete. WEB-WELCOME, separately deferred WEB-OPT and desktop remain outside CLI scope; WEB-OPT has no measured baseline.

### 1.2 Cold-service continuation (2026-10-25)

**VERIFIED scoped implementation:** app-owned workers provide text-only clipboard read/write requests capped at 1 MiB, bounded ignore-aware workspace snapshots using `polaris-tools-builtin`, and a settings helper with runtime overrides plus explicit one-field persistence to the global config owner. Clipboard and index results are generation-scoped; queue submission is nonblocking, stale work/results are retired, index shutdown is bounded, and settings saves run on an owned worker with a bounded queue. Demo/render fixtures use `MemoryOnly` settings and cannot call through to global persistence.

Focused evidence: [continuation-services-verification.md](evidence/ui/continuation-services-verification.md), [service descriptor](evidence/ui/continuation-services-descriptor.json), [config preference descriptor](../specs/descriptors/config_cli_preferences.json), [demo runtime descriptor](../specs/descriptors/ui_cli_demo_runtime.json), [editor controller descriptor](../specs/descriptors/ui_cli_editor_controller.json), and [cursor repaint descriptor](../specs/descriptors/ui_cursor_repaint_integrity.json). The tests cover config key preservation/atomic failure, memory-only settings, background settings results, bounded index output and ignore/secret-path behavior; they do not mutate the user's clipboard. The service-specific pre-implementation red transcript was not retained, so no red claim is made for this continuation.

**Still partial:** the full UI4 integration gate is open. Native Windows/Linux/macOS clipboard operations and their failure/timeout paths need platform evidence; path-junction/native filesystem edges need focused verification; settings screen save-failure/retry, restart and malformed-config behavior need integrated tests. The subsequent integrated visual exports and controller evidence are recorded in §1.3. No phase status changes follow from this scoped work.

### 1.3 Current interactive continuation checkpoint (2026-10-25)

**VERIFIED scoped behavior:** the composer now shares Unicode row/source geometry for painting, caret, selection, click/drag and vertical navigation. Shift+Enter inserts a newline; Ctrl+Enter is intentionally unbound. Word/line movement, selection replacement, chip preview and sticky vertical column have controller regressions. The presenter restores the physical editor caret after repaint, including Shift+Up/Down effort changes. Typed hit maps support footer/completion/tool hover, click, editor drag and canonical single-entry transcript selection, with overlay clipping and current-state geometry.

The explicit `--demo` profile (also selected through the footer or F5) owns isolated drafts, transcript and scripted streaming/cancel/replay state. It does not open durable offline state, call KernelApi/providers or save preferences. Entries are labelled `[DEMO]`. Its three-question form exercises single/multiple/free-text answers, exclusive **None of the above**, no defaults, Tab notes and atomic Enter submission. This is display/controller evidence, not P3 human-request backend acceptance.

**VERIFIED evidence:** [current checkpoint](evidence/ui/continuation-verification.md) and [machine-readable index](evidence/ui/continuation-verification.json) record **350 passing scoped tests** (263 config/model/renderer plus 87 selected app tests). The final self-contained native test target passed **15/15**, including key modifiers, `$` Alt-code delivery, literal paste, idle output, observable terminal cleanup and demo isolation. Safety Clippy, formatting and dependency-policy subset commands are recorded separately. [Actual interactive buffer exports](evidence/ui/continuation-after/) cover welcome, questionnaire, settings and selected multiline input. Meaningful behavioral red evidence is retained for cursor staging, editor/controller, pointer handling and preference override persistence; the earlier broad/service red gaps remain historical limitations.

**Open gates:** headless doctor still fails on missing setup prerequisites; advisory audit cannot lock its read-only cache. Native OS clipboard success, physical-terminal/Linux matrix, cross-entry transcript selection/word drag, interactive demo tool/diff cards, full settings error/restart flows, production human-request/model integration, transcript/layout caches and measured allocation/performance/release budgets remain open. Text clipboard/index/settings workers are implemented but do not close those acceptance gates. Web welcome, separate WEB-OPT and desktop stay deferred. No UI phase or M1 completion is claimed.

## 2. Design extraction and intentional differences

### 2.1 Retain, adapt or exclude

| Idea / provenance | CLI treatment | Acceptance owner |
|---|---|---|
| Minimal North Star + quiet app bar (`header-logo.md`) | Regular-weight title/version; actual package version; clipped workspace; account label only if explicitly configured, never a sign-in claim. | UI2-1 |
| Black, full-width composition (`ui-design.md`) | Two-cell side margins, two top rows, no arbitrary content-width cap; preserve hierarchy through responsive layout. | UI2-1 |
| Composer rules, open sides, growth (`composer.md`) | Shared wrapping; maximum visible draft height floor(h × 3/8), bounded further by protected controls/history. Draft survives resize. | UI2-2 |
| Thin footer controls, gold plan / pink auto / blue directives and goals | Generate available hints from the shared keymap; capability-dependent chords cannot be advertised as universal. | UI1-2 / UI2-2 |
| No role headings; compact tools and worked summary | Keep visual restraint but preserve actor attribution in data, tool/source details and child-thread banner. Display durations/clocks only when known. | UI2-3 |
| Ctrl+O inline tools, numbered code and full-row diff surfaces | Shared renderer; semantic copy excludes chrome/gutters and preserves indentation/diff markers. Plain fallback for unknown languages. | UI2-3 / UI4-3 |
| Opaque completion/bottom sheets | Clear covered cells and hit targets, retain draft/scroll, pin title/hints. One overlay stack owns focus. | UI2-4 / UI4-3 |
| Main + child roster (`agents.md`) | Retain pinned Main, separate viewed thread from submission target, per-thread scroll/expansion, honest lifecycle and unknown metrics. Requires real frame contracts in P3; extended DAG views follow P7. | UI5-3 |
| Enter-to-apply settings (`settings.md`) | Candidate is separate from applied state; Enter persists only that field via project config owner. Failure remains visible and retryable. | UI4-4 |
| Usage Strip/Ticks/Segments; Stats Stacked/Lines/Trends | Same source/denominator across styles, exact-value inspector, accessible text fallbacks. Defer unsupported data categories rather than synthesize them. | UI4-5 / UI5-4 |
| Demand-driven updates (`rendering.md`) | Preserve dirty scheduling, bounded semantic batches, layout caches, staged cursor and zero bytes for identical frames (§5). | UI3-2 / UI3-3 / UI6-2 |
| Demo provider/model names, simulated activity, sample stats, D toggle, `/preview`, `--preview` | No implicit sample data in offline/live profiles. The user-authorized explicit `--demo` profile is isolated, bounded and labelled DEMO; deterministic dump fixtures remain labelled SYNTHETIC. | UI0-2 / UI4-5 |
| Web welcome's green atmosphere and pill composer | Record as future web welcome reference; CLI keeps static cell artwork, no background animation or raster emulation. | WEB-WELCOME / WEB-OPT |

**VERIFIED reference repair (2026-10-25):** welcome glyphs and foreground colors outside the footer match the saved demo buffers at 60×20, 80×24, 100×30, 120×40 and 160×60; see [comparison](evidence/ui/repair-reference-comparison.json) and [repaired buffer preview](evidence/ui/repair-after/welcome-80x24.png). Footer model/effort values remain actual local state. Black rule-bounded editor, wrapped growth, per-glyph North Star colors, neutral conversation, opaque selection-following completion and black sheets are covered by the new reference suite. The final repair command passed 92 selected tests (46 UI CLI, 46 app), including nine native PTY scenarios and one VT-parser regression; this is separate from the older full-package doctor failure. See [repair verification](evidence/ui/repair-verification.md). No complete UI phase is claimed.

### 2.2 Resolve visual drift before implementation

**VERIFIED correction (2026-10-25):** the active demo specifies a minimal North Star, a black live editor and `#141414` submitted-message surfaces. The initial plan incorrectly assigned the submitted-message surface to the composer. This error propagated into renderer assertions and is corrected with an explicit [source-to-assertion migration](evidence/ui/repair-assertion-migration.md). The current reference takes precedence over historical PLAN sketches.

UI1-1 must migrate the relevant model tokens/logo grids, their provenance and assertions together;
UI2 must regenerate reviewed product snapshots. Preserve older assertions until their requirement
change is explicitly recorded; never weaken unrelated safety/geometry tests. Main PLAN §13 visual
sketches are historical where they conflict with this section. Main architecture and capability
gates continue to govern.

| Role / geometry | Proposed authoritative CLI value |
|---|---|
| Canvas / normal text / muted | `#000000` / `#E8E8E8` / `#858585` |
| North Star center / rays / tips | `#C0E58C` / `#76B900` / `#6B6B6B`; no filled package |
| Welcome / compact / app-bar mark | 9×5 / 5×3 / one-cell `✦`; same-size ASCII alternatives |
| Hero visibility | Empty transcript, no active work: 9+ body rows → 9×5; 5–8 → 5×3; otherwise app bar only. Typing alone does not hide it. |
| Live editor / submitted prompt / composer rule | `#000000` / `#141414` / `#343434`; open sides, complete top/bottom rules |
| Control hover / focus | `#181818` / `#191919` |
| Plan/tools/mentions / auto / directives/goals | `#F9E2AF` / `#F5C2E7` / `#89B4FA` |
| Command focus / hover; file hover | `#B6E35B` / `#DBF5B0`; `#FFFDE6` |
| Warning / error | `#E5B454` / regular-weight `#AC4242`; always text labels |
| Diff add / remove | Existing PLAN `#91CD66` on `#07230C` / `#F59696` on `#320709`; `+`/`-` survive no-color mode |
| Selection | Preserve source foreground/emphasis/link underline; 30% source tint over actual background, neutral whitespace tint; white on `#141414` → `#545454`. ANSI/terminal index 237, no-color bold. |
| To latest | `#262626`, hover `#373737`, whole underlying glyph-cell occlusion |

Font and glyph rendering belongs to the user's terminal. ASCII replaces application decoration,
not user Unicode. Truecolor, ANSI, inherited terminal theme, `NO_COLOR`, `TERM=dumb` and reduced-motion
must retain readable labels and focus. App options override auto detection only as documented.

### 2.3 Layout and state behavior

At ≥60×20 use one geometry result for header, history, status, draft, footer, roster and overlays.
Smaller live terminals show a compact resize notice and retain the complete draft/navigation;
cancel/quit remains possible. Zero/tiny dimensions must return a bounded render/typed validation
result rather than underflow. Bound dump dimensions/cell count before allocating (UI0-1 freezes
the supported maxima with the test harness).

The composer has a `>` on its first visible row, display-only placeholder, one blank row before
the footer and no hidden fixed six-line limit. Status/goal and approval controls take priority over
decorative hero space. Hide To latest under an occluding overlay. Below 28 rows compress approval
descriptions, with an explicit way to inspect full scope; never hide decision identity or risk.

Sheets scroll their bodies, not pinned controls. Settings uses a 20-column category sidebar from
100 columns and wrapping tabs below it. Usage uses two columns from 104 inner cells, otherwise
stacks; preferred 30 rows / expanded 40, bounded by available space. Stats prefers 32 rows.
Roster targets up to four child rows at 32 terminal rows, two at 24, one below that; the rest scroll.
These breakpoints are design targets subject to the protected-region checks, not permission to clip.

### 2.4 Balanced cockpit and responsive optional panels

The conversation and composer remain the default surface. Add one optional inspection panel, not a
second execution owner. It is closed on first launch; only an explicit saved pin preference may open
it on later launches. F4 toggles the selected panel; F6 cycles main/panel focus when both are visible.
These bindings are unavailable inside the human-question modal. The shared keymap generates hints.

At **120 columns × 28 rows or larger**, an opened panel occupies **34 columns including its divider**;
outer margins remain two cells and the conversation keeps the remaining width. Below either threshold,
opening a panel uses an opaque sheet. On shrink, an unfocused panel collapses to a named indicator;
a focused panel becomes its sheet. On growth, a pinned panel returns without taking focus. Preserve
panel choice/filter/scroll, source selection anchors, inspected thread and the main draft across reflow.
At 60–79 columns keep optional details out of the status ribbon; below 60×20 use §2.3's resize notice.
Never allocate a second composer or change the submission target merely because panel focus moved.

| Panel | Real data and controls | Main gate |
|---|---|---|
| Tasks | Goal, plan steps, dependencies, current stage and blocked reason; inspect supporting evidence. | P3 base; P7 DAG depth |
| Agents | Main and children, frame lifecycle, viewed-thread marker, pending questions and scoped interruption. | P3; P7 scheduling detail |
| Questions | Outstanding requests grouped by requester; open one without changing the main draft. | P3-X-5 / UI5-10 |
| Review | Exact command/diff, workspace/sandbox, blast report, immutable approval identity and decision state. | P2 |
| Proof | Required checks, measured results, missing evidence and latest proof outcome; prose never marks Completed. | P2 base; P4 MVP |
| Context | Folded references, provenance, expansion and ingestion state; unavailable content remains explicit. | P3 references; P5 full view |
| System | Provider availability, quotas/budgets, workers, queues, retries and failures with provenance. | P1 observed status; P3/P6/P7 detail |

Use Prepare → Run → Review → Verify as navigation labels, not a fabricated linear percentage: work
may return to a stage, have parallel branches or be blocked. Distinguish execution succeeded, answer
ready and proof accepted. Show no elapsed/count/health assertion until its owner supplies that fact.

Add navigation metadata for `/panel [tasks|agents|questions|review|proof|context|system|off]`,
`/questions` and `/agents`. These open views locally; answer/cancel/approval actions use domain APIs.
Canonical grammar/help/availability metadata must agree; no unknown directive is executed by the UI.

## 3. Architecture, ownership and contracts

Use existing packages; CSV rows describe **logical modules, not new workspace members**.

```text
safe terminal input ─► app-owned ordered queue ─► keymap/actions ─► composer / transient UI state
                                                     │
                                                     ▼
                       typed kernel command + receipt / rejection (capability checks in kernel)
                                                     │
                       committed EventRecord stream + bounded payload queries
                                                     ▼
                         session-aware model projection ─► layout/cache ─► cell diff + cursor
                                                                               │
                                                            safe fallible terminal writer
```

| Owner | Responsibilities / forbidden dependencies |
|---|---|
| `polaris-sdk` + existing API registry (L0) | Reuse command/event/newtypes and typed service seams. Add a shared UI-required contract here only if it is domain-level and missing; version the schema and update proto/codecs/replay tests. No dependency on UI. |
| `polaris-ui-model` (L6) | Reuse composer/completion/classification/keymap/tokens/reducer. Add pure presentation state, explicit availability and bounded projection as needed. No terminal, filesystem, clipboard, runtime or kernel I/O. |
| `polaris-ui-cli` (L6) | Pure Ratatui drawing/layout, semantic text maps, terminal-event normalization, generic scheduler/presenter logic behind narrow safe traits. No in-process kernel ownership or execution. |
| `apps/polaris-cli` (L7) | Sole terminal-session/loop/composition owner; input and background workers, event/payload adapters, receipts, preferences, typed errors, shutdown. Preserve current headless/doctor/rtk/sandbox paths. |
| Existing kernel/storage/config/security/telemetry/platform owners | Own authorization, sessions, CAS, indexing, settings, metrics and cleanup. UI calls real APIs; never duplicates a business rule to make a screen appear functional. |
| `polaris-tests` (L8) and crate-local tests | Cross-crate/real-binary acceptance versus pure logic/layout tests. ConPTY via vetted portable-pty, no handwritten unsafe harness. |

Third-party APIs sit behind focused internal adapter traits in the owning module (prompt §6.2).
Use generics/enum dispatch; cold-path dynamic dispatch needs a reason. Prefer exclusive writer and
buffer ownership over `Rc<RefCell<...>>` runtime borrow panics. `FrameContext` explicitly carries one
captured monotonic time, terminal area and generation; no implicit thread-local clock.

### 3.1 Contract audit required in UI0-1

Inventory actual API signatures, event schema, sequence scope, payload limits and permission checks;
freeze descriptors before writing consumers. Required capabilities:

1. Open/resume/list session; submit authorized input/directive/control; bounded command admission,
   receipt/commit/rejection, cancellation and joined shutdown. Reuse current LocalBackends.
2. Subscribe/replay from a cursor; detect lag/sequence gaps; materialize bounded CAS payloads for
   conversation, tool output, diff, settings and proof evidence. Confirm whether sequence scope is
   global or per-session: filtering sessions must not produce false gaps. Test interleaved sessions.
3. Domain-backed pending approval identity/scope/diff hash/expiry; decision acknowledgement. Never
   construct an authority witness solely from a displayed row or treat the screen as authorization.
4. Provider/model capabilities and availability, real telemetry snapshots with optional measurements,
   frame identities/lifecycle and per-frame attribution. P3/P4/P5+ dependencies remain gated.
5. Config ownership, writable preference scope, canonical directive metadata and workspace path index.

**UNVERIFIED:** full streaming transcript/frame and terminal capability APIs meet all these needs.
Current model events alone do not establish that. Missing contracts are work in their existing
owner/main phase, including codecs/schema version and contract tests, not hypothetical names called
as though they existed. `UI_MODULES.csv` identifies both consumer and prerequisite ownership.

### 3.2 Commands and authoritative state

Transient UI state (selection, candidate preference, viewed thread, scroll, layout) can change
immediately. A domain effect is Pending until an acknowledgement/committed event; mark errors and
retain retry context. Never show a sent prompt as committed solely because Enter was pressed.

**VERIFIED:** current Composer exposes a mutating `submit`. UI1-3 must design prepare/commit/restore
semantics or snapshot-before-submit so rejected admission preserves text, cursor, chips, selection,
undo and prompt-history draft exactly. Use a stable command id; after ambiguous disconnect reconcile
its receipt before retrying, so reconnect cannot duplicate a tool run or submitted prompt.

Parse `$ / @` through the established grammar/metadata; shared completion coloring is presentation,
not authorization. Settings-only input commits directives without starting a model response. UI
commands may open while work runs; a second ordinary prompt stays draft until accepted by a backend
that explicitly supports queueing. Auto mode never means unrestricted host execution. `/clear` clears
the visible conversation/history as specified without deleting the durable audit journal; the exact
resume/clear projection contract is an explicit UI4-2 case.

Model/effort selection distinguishes highlighted candidate, locally requested preference and confirmed
backend capability. Unsupported effort levels show a reason or remain unavailable. No hard-coded
demo model catalog, token prices or provider connection success. `/agents` and F3 are a planned
addition to the main command/keymap contract in UI5-3, not assumed existing grammar support.

### 3.3 Projection, recovery and truth of metrics

Keep durable domain events separate from presentation actions. Validate ordering before mutation;
commit the sequence cursor only after required payload application succeeds. Async responses carry
session/frame/entry/generation identity; stale results cannot overwrite current content.

On lag/gap, mark the projection recovering, freeze risky actions, obtain a consistent snapshot and
replay tail through the kernel owner, then atomically swap the view and resume. Do not skip missing
records, reset last_seq to hide failure or infer Completed from text. Failure to recover is visible
and retryable; kernel execution must not block on a stalled renderer.

The complete event log/CAS remains authoritative. Keep only a bounded loaded transcript window and
derived layout caches; page older content on demand with a loading/error marker and stable anchors.
Do not evict a live selection's source silently. UI5 must reconcile resume/replay to the same view
hash under fixed presentation time and availability policy.

Known zero, unknown, not reported and stale differ. Existing count-only fields need availability
metadata before the UI can claim provider cache hit, cost, quota or context capacity. Counts can be
shown without a percentage when capacity is unknown. Use checked integer arithmetic for micro-USD,
tokens, totals and ratios; overflow becomes a visible unavailable/error result. Specify whether
cached input is a subset before summing; never count it twice. Shared usage/stats datasets reconcile
per-model/per-frame/session totals with deliberate child inclusion; Main roster metrics do not
silently aggregate children. Missing dates/timezones and unknown categories stay explicit.

### 3.4 Human ask/answer protocol and popup (P3, not an M1 dependency)

**VERIFIED source boundary:** SDK `system_one::QuestionKind/Answer` describes model classifier
schemas, not human interaction. Current `CommandBody` has no answer-request command; an existing
`CancelFrame` variant does not establish targeted kernel dispatch. This requires actual SDK/kernel/
runtime-frame work under main **P3-X-5** and **UI5-10**. Pure form fixtures prove only presentation.

Keep domain ownership separate from UI state. Add a dedicated versioned human-input payload with a
stable request id, session, requesting FrameId, origin step/turn, immutable revision/digest, question
ids, stable option ids and required flags. Support SingleChoice, MultiChoice and FreeText. Requests
and answers are bounded CAS objects; durable events remain small Copy records with ids/CIDs. Extend
the next event schema and propagate command/event conversions, durable codecs, replay compatibility
and RPC definitions as applicable; historical schema v1–v4 records remain readable. Do not reuse S1
question ids or infer a request from model prose.

The runtime exposes a real `ask_user` tool/host service through the existing ToolRegistry/API seams.
Validate its frame capability and payload before persisting a requested transition. Permit one
outstanding request per frame and 32 per session; reject overflow explicitly rather than silently
lose questions. Wait only inside the requesting frame task; siblings, event persistence, control
handling and the UI keep running. This P3 concurrency behavior uses owned per-frame cancellation
tokens and must not wait for the P7 DAG implementation. A pending request survives pause/restart and
is rehydrated before its frame resumes; an interrupted request cannot reappear as pending.

Add typed `AnswerHumanRequest { request_id, revision, answer: Cid }` command semantics through the
existing unverified-command/verifier/receipt pipeline. An answer is one atomic batch of stable question
and option ids, explicit NoneOfTheAbove selections, free-text answers and optional notes. The kernel
checks session/frame ownership, capability, revision, pending state and all required answers; reject
incomplete, stale, cancelled, conflicting or unknown-id answers without applying any partial answer.
Persist the answer before acknowledging resolution and waking the frame. Repeated same command id
reconciles its original receipt/result; conflicting duplicate or second-client answers are rejected.
An admission receipt alone is not evidence of the completed answer. On ambiguous delivery keep the
form Pending and reconcile, rather than submit a new command id.

Wire `CancelFrame` to the specific runtime frame token/registry and commit a request-interrupted
transition. On answer/cancel races the kernel serializes terminal transitions: exactly one wins;
the loser gets the existing outcome and cannot restart work. Esc never maps to kernel shutdown or
cancellation of Main merely because the user inspected a child. Closing the session explicitly is
a separate operation; reusing an expired frame identity is rejected. No deadline auto-selects an answer.

The UI maintains a pending-request list, with one active request form. Preserve answers/notes while
navigating or inspecting another queued request. Automatically present a new request only when the
main draft is empty, no other modal/paste/drag is active and no typing occurred for 750 ms; otherwise
show a persistent Questions indicator. Never replace a visible approval or question underneath input.
A request resolving elsewhere becomes a read-only outcome, not a new form reusing the old focus.
The pending list can reopen/inspect a request; postponing it leaves only its requester waiting.

Popup geometry is centered, up to 88 columns wide with two-cell outside clearance, bounded by the
available body. Title, requester/turn, question index, unanswered count and key hints remain pinned;
long question/choice content scrolls. Notes use the shared Unicode editor with at most four visible
rows, preserving their full bounded text. Layout prioritizes origin, content and controls over hero
art. All styles/labels are sanitized as untrusted text. Main composer, history draft, attachments,
cursor, selection, undo and inspected-thread state are untouched by answer editing.

| Key in the human-question context | Exact behavior |
|---|---|
| Left / Right outside notes | Previous/next question, clamped; preserve every question's state. |
| Up / Down outside notes | Move the choice highlight; highlighting alone never answers. |
| Space outside notes | Explicitly select the highlighted single option or toggle a multi-select option. |
| Tab | Add/focus optional notes; when notes are focused return to that question's options/navigation. |
| Shift+Tab | Return from notes to options/navigation; never toggle plan/auto. |
| Arrows inside notes | Edit/move within notes normally, without changing question or option. |
| Ctrl+J inside notes | Insert newline; Shift+Enter/Alt+Enter only when the terminal capability is verified. |
| Enter, including inside notes | Validate all mandatory questions and submit the whole batch once. No second confirmation keystroke; errors focus the first incomplete question. |
| Esc | Interrupt this request's frame; show Pending until the kernel confirms. Retain form state on rejection. |
| Ctrl+C in this modal | Same requester-scoped interruption as Esc; no global clear/exit fall-through. |
| Ctrl+B | Defer this form and return to conversation with its answers/notes retained; requester remains waiting and /questions reopens it. This is not interruption. |

The footer wraps as needed: **Tab notes | Enter submit all | ←/→ navigate questions | Esc interrupt**,
plus Up/Down/Space guidance where choices are present. No generic popup Enter-close, completion,
Ctrl+O or main-composer binding can consume a questionnaire key.

Every choice question includes a visible **None of the above** option, represented as a typed
selection, not an ordinary provider-supplied option id. Nothing is preselected. Show a compact answered/unanswered summary across all questions before
submission without introducing another confirmation keystroke. For MultiChoice,
selecting None clears named options; selecting a named option clears None. Explicit None satisfies
a required choice question even without notes. Notes are optional for all choice answers; free-text
questions instead require nonempty answer text when marked required. No hidden Other choice is
needed: None + notes permits a custom explanation. Encode every optional unanswered question with an explicit Unanswered state in the answer batch,
never omit it or silently map it to None. Limits are in §8.

Clarification answers grant no execution permission and construct no approval witness. A prompt that
asks for credentials routes to the secret-management/onboarding workflow instead of persisting a key
in notes or sending it to the model. Redact payloads from diagnostics; do not put draft answers in
key traces, command-line arguments or telemetry. Durable answer artifacts follow the existing
session-content/PII policy, not a new unredacted debug transcript.

## 4. Safe input, editing, clipboard and selection

Human-question keys have the explicit precedence and requester scope in §3.4; the general Esc/Ctrl+C
rules below apply to other contexts.

### 4.1 Terminal capability and keymap contract

Use vetted safe crossterm/platform interfaces, with acquisition/restoration documented in UI0-2.
S0-10 manual status is a prerequisite to advertising terminal-specific chords, not evidence supplied
by the demo. Normalize available BackTab/Shift+Tab, Return/Escape/control characters, enhanced keys
and supported mouse/resize events. Ignore key releases as actions; preserve intentional repeat.

Enter submits; Shift+Enter is a newline only where verified. Alt+Enter / Ctrl+J are first fallback
candidates and need real terminal checks too. Ctrl+Enter is intentionally unbound for newline. Never sample Shift using unsafe
Win32 calls or claim a decoder can recover modifiers the OS discarded. Capability reply parsing, if
needed, is bounded, timeout-driven and separate from text; bracketed paste is always literal text.

One context-aware keymap generates footer/help. Extend existing Ctrl+Z/Ctrl+Y undo/redo rather than
duplicating it. Esc priority: overlay/menu/preview/selection → roster focus → inspected child back to
Main → cancel Main work. Ctrl+C retains current context-sensitive interrupt/close/clear/two-press
empty-draft exit semantics, with a bounded confirmation window frozen in UI0. Tab in forms cannot
toggle mode; Ctrl+O in stats/usage cannot expand transcript tools. Menus own their wheel region and
clamp, rather than wrap, one item per notch.

### 4.2 Payload and source limits

Preserve existing model constraints: 1 MiB normalized text; long paste chip above five logical lines
or 500 Unicode scalars; 64 MiB decoded RGBA per image; 128 MiB session attachments; completion max
100 results. Validate actual bytes and checked width × height × 4 before accepting image data; an
ImageRef's length alone is not proof of a retained, decodable payload. Tie payload ownership to chip
lifetime, undo and session persistence; release/cancel deterministically, avoiding undo-history copies
that multiply the attachment cap. Bound undo and history by bytes as well as entry count.

Clipboard and file-index tasks are cancellable cold workers with deadlines and size limits; their
results enter the ordered queue with generation ids. Slow native clipboard APIs must not block the
UI executor; a cancellable request may still require worker isolation/join policy if the native call
cannot stop. Unsupported image/model/ingest capabilities show a clear reason and retain the draft;
do not pretend attaching a chip means the backend can process it (P3/P5).

Path completion is a snapshot query, never a filesystem traversal during render. Use the existing
trusted workspace/code-intel/ignore policy rather than copy the demo walker. Exclude deny-listed
secret paths, do not follow symlink escapes, handle junctions/case aliases and quoted Unicode paths;
final access is revalidated by the backend. A stale index may suggest a deleted file but cannot grant
access. Earlier scan generations cannot replace newer ones.

### 4.3 Shared text geometry and hostile content

One wrapping/width policy powers paint, byte↔grapheme↔cell mapping, caret, history row guards,
hit tests, selection and scrolling. Test tabs, combining marks, ZWJ emoji, wide glyphs, empty rows,
CRLF normalization and terminal-dependent width fallback. Chips are atomic. Ordinary drag selects
source graphemes and real spaces; double-click selects a word using the composer’s shared token
classification, with held drag extending by complete words. Triple-click selects a logical source
line, including its existing newline, across soft wraps. The click sequence uses a 450 ms interval
and one-cell tolerance. Decorative padding cannot start a word/line selection; drag boundaries
clamp to visible source rows. Exactly one editor/transcript surface owns the active gesture;
release preserves the editable range for Delete/Backspace, replacement and Undo. Typing immediately
before a click refreshes geometry **in memory**, without a pre-click frame. Overlay occlusion removes hidden targets.

Escape untrusted C0/C1/ESC/OSC/DCS sequences and make bidi/control spoofing visible at the display
boundary. Never feed tool ANSI directly to stdout or trust ansi-to-tui as a sandbox; if color parsing
is retained, allowlist SGR styles through a bounded parser and strip other actions. No payload-driven
OSC52 clipboard write, title change or auto-open URL. Link activation requires an explicit action
and approved scheme; underline alone implies neither reachability nor trust.

Semantic copying excludes placeholder, app chrome, line numbers and hidden attachment labels;
preserve meaningful source text/indentation. Define control-character-safe copy separately from
byte-exact raw CAS export so invisible controls cannot silently enter a pasted shell command. Keep
raw evidence lossless in storage, never in the terminal transport. Copy-on-release requires an
application selection; show success only after the clipboard adapter confirms it, else an error.

The optional --key-trace adapter creates a new private file with create-new/no-overwrite semantics,
rejecting existing files/symlink targets. Record only bounded event categories, approved modifier/
nontext-key metadata and counts, plus mouse kind/button/coordinates/modifiers; never printable key values, raw input bytes, paste, paths, drafts,
answers/notes, image data or secrets. Trace I/O failures are explicit. Verify privacy with canaries
across success and error paths; traces are diagnostics, never transcript capture.

## 5. Screen updates, cache and terminal lifetime

### 5.1 Scheduling and fair batching

Demand-driven rendering with a **60 fps maximum**, no continuous redraw loop. Dirty causes are
semantic edits, visible event changes, width/theme/thread generations, focus/hover/selection changes,
visible timer boundaries and worker results affecting a visible surface. Partial escape input,
releases, unchanged hover, hidden index refresh and identical frames do not dirty the screen.

Drain at most **256 semantic events or 4 ms** per turn (design starting limits), then service kernel
events, cancellation, payload completions and any due frame fairly. Coalesce only adjacent plain
mouse moves and adjacent resizes; preserve keys, paste, wheel, press/drag/release boundaries and all
domain records. Coalescing never crosses a semantic barrier. A continuously dirty stream cannot
starve a frame or cancellation; no busy-loop when nothing changes.

Freeze queue capacities/byte budgets, backpressure and a reserved cancellation path in UI0-1.
Bound queued paste bytes and output packets, not just item count. At saturation pause the producer
or reject with explicit overload; never silently drop accepted keys, commands or durable events.
Broadcast lag triggers §3.3 recovery. Time arithmetic is checked; one captured monotonic time per
frame drives all durations. Idle waits select input/events/deadlines/cancel. Visible elapsed labels
invalidate at displayed second boundaries; reduced-motion disables cosmetic animation.

### 5.2 Cache and geometry generations

Cache entry layouts by stable entry id + revision + width + theme/glyph generation + disclosure
state. Append only new/changed entries. Composer-only edits reuse history pane cells/source map;
selection/hover repaint the affected pane without reparsing unchanged source. Share immutable source
handles; do not clone the whole transcript to paint a frame.

Only visible rows need cell hit maps. Retain semantic source separately for cross-message copy and
page it explicitly if unloaded. Width/theme/thread changes release obsolete derived generations.
Cold resize/reflow may be proportional to loaded text: measure it separately and page/chunk work so
the terminal remains cancellable. Never claim constant-time full-history resize.

The app refreshes stale hit geometry before dispatching a click with the same layout engine used
to paint. Tests must distinguish current state geometry from the last emitted terminal frame.

### 5.3 Buffered presentation and cursor

Ratatui owns the sole previous/current cell-buffer diff; do not add a second application cell diff.
A generic backend/writer adapter stages commands and cursor intent with exclusive mutable ownership,
without the demo's Rc<RefCell> aliases. Stage the entire changed-cell command sequence and final cursor intent in an owned bounded buffer.
Commit with `write_all` and one final flush per changed frame; partial OS writes remain possible, so
this is not a physically atomic screen update. An identical cell grid **and** identical cursor
position/visibility emits zero application bytes. Cursor-only moves position once without a hide/show
pair. Info sheets hide the caret; dismissal restores the exact draft insertion point.

Optional DEC 2026 synchronization must be positively negotiated through a safe supported input
path. Bound probe/reply size and proposed timeout (100 ms); split/late/unknown replies are handled
without turning them into draft keys. Unsupported/unavailable probing selects buffered hide-paint-
position-reveal immediately. Do not build a new unsafe reader just to enable the optimization.
Restore reported blink/shape state when safely observable; otherwise restore terminal default and
document that limit. No user terminal-config edits.

On write/flush failure, attempt to end synchronization and reveal/restore cursor, record both primary
and cleanup errors, invalidate the presenter state and stop normal painting. Never retry an uncertain
partial frame as if it were fresh. The initial implementation chooses clean shutdown, not automatic retry. Any later recovery design
must invalidate Ratatui's prior buffer and perform a full resync before incremental paint resumes.

### 5.4 Terminal session and shutdown

The app's session guard records only successfully acquired modes: raw input, alternate screen,
mouse capture, bracketed paste, enhanced keyboard request, cursor visibility/blink and console modes
the safe adapter can manage. Startup rollback unwinds acquisition in reverse order. Prefer explicit
`close() -> Result<..., CleanupReport>`; Drop is a best-effort no-panic fallback whose failures go to
a nonterminal redacted sink if available. Do not silently discard Result in either path.

Normal quit, Ctrl+C, EOF, kernel failure, setup error and output failure share a bounded shutdown:
stop admission → cancel owned workers/domain work as appropriate → persist accepted session state →
join/reap → restore terminal → flush diagnostics. Closing a sheet cancels only sheet queries. On
unwind use the guard without recursive panic; test unwind with an external/injected failure harness
consistent with zero-panic source policy. Abort, process kill, power loss and a broken terminal can
prevent restoration: record that physical limit instead of promising otherwise.

## 6. Feature delivery linked to the system

| System anchor | CLI requirement / gate | Scheduling constraint |
|---|---|---|
| P1-X-2; M1; PLAN §13 ui-model/ui-cli/cli | UI0–UI4: real offline sessions/directives/read controls, composer/completion, safe interactive shell, help/status/settings/stats and deterministic dump. | Preserve P1 non-UI evidence; full M1 closes only with its remaining system gates too. |
| P2-X-2; M2; PLAN §6.6 and guards | UI5-1: real doctor/onboarding status, sandbox lifecycle, sync, diff, approval/blast-radius and proof outcomes. | Actual P2 contracts and sandbox evidence; no host-exec fallback. |
| P3-X-1..5; M3/M4 | UI5-2..4 and UI5-10: streaming model/tool transcript, model/provider/effort, usage, child frames, replay and human ask/answer. | Local endpoint first. Keyed Token Factory/Contree evidence remains separate BLOCKED-ON-ACCESS when credentials are absent. |
| P4-X-3; M5; PLAN §11.7 | UI5-5: S1/PII status and scripted Rust/Python/CUDA-flavored demo; falsified completion shown Blocked until ProofOfDone passes. | No fabricated latency, model completion or GPU verification. |
| P5/P6/P7/P8 | UI5-6/7/8/9, independently: knowledge/context, branch/budget, research/DAG/hacker/browser, DevEnv views from actual owner data. | Incremental extensions at their own phase; not dependencies of M1 or an excuse to fake commands. |
| P9-X-1; ADR-012/014 | WEB-WELCOME, WEB-OPT and DESKTOP-LATER in §10. | Deferred; welcome may be pulled forward only by later authorization. Full daemon attachment is not required for local CLI. |
| P10-1..5; M6 | UI6: security, terminal/manual matrix, load/performance, packaging and reproducible evidence. | CLI regression subset accompanies every relevant change; full submission uses main hardening gates. |

No unreachable feature masquerades as an active button. Completion/help can list a later command
with a visible availability reason and phase; invoking it returns an explicit unsupported/unavailable
result without changing authoritative state. Avoid blank placeholder panels for unscheduled features.

### 6.1 Capability-to-UX coverage and independent backend gates

All rows require loading/empty/unavailable/running/error/retry/cancel states where applicable,
source-backed results and reconnect/replay behavior. A navigation view can exist before a capability,
but must show its actual unavailability; a synthetic dump is not backend acceptance. The matrix
specifies coverage, not permission to introduce independent UI business rules.

| Capability / canonical entry | CLI interaction and trustworthy result | UI task / main prerequisite |
|---|---|---|
| /sessions; /session resume; /clear | Search/select sessions with workspace identity; explicit resume; clear only the visible projection watermark, keep audit durable and replayable. | UI4-2 / P1-X-2 |
| /settings; /status; /help; /quit | Candidate versus applied/saved fields; source layers, availability reasons, generated keys; joined teardown. | UI4-2/4 / P1 |
| /plan; /goal | Task tree with stage/dependency/blocked state and evidence links; authoritative goal updates acknowledge before applied state. No completion inferred from prose. | UI5-2 / P3 runtime/kernel |
| /session replay [step] | Read-only historical checkpoint with clear REPLAY cursor; disable domain mutations until explicitly returning to live state. Stable transcript/artifact references. | UI5-2 / P3 eventlog/runtime-context |
| /diff; /rollback; /undo | Inspect paths/hunks and scope; show dirty-workspace conflicts and exact proposed target; explicit confirmation for destructive changes with owner revalidation. Composer Ctrl+Z is separate. | UI5-1 / P2 vcs/kernel |
| /proof-of-done; execution approval | Separate immutable scope/digest and decision state; missing/failed checks remain visible; Completed requires actual proof outcome for that frame/claim. | UI5-1/5 / P2 guards; P4 MVP |
| /model; /providers; /onboarding; /usage; /quota | Real capability/catalog and setup checks; secret-entry workflow; cost/quota/source/reset unknowns explicit, retry/429 state honest. | UI5-1/2/4 / P2/P3 owners |
| /agents; /questions; /panel | Frame-attributed inspection, Main submission target, request queue, exact ask/answer keys and requester-only cancellation. | UI5-3/10 / P3-X-5 and frame composition |
| $directives; @file; path completion | Grammar-aware token/chip editing, canonical metadata, real ignore/access policy and directive-only acknowledgements. Parsing never implies later service availability. | UI4-2/3 / P1; execution gated by owner phase |
| /context-map; /context-expand; folded §ref | Inspect budgets, immutable source and fold provenance; page exact expansion separately from safe terminal display; replay cannot change source content. | UI5-2 base / P3; UI5-6 full / P5 |
| $rag; $links; $websearch; /search-local; $nvidia; /nv-profile; /nv-verify | Query/sync/ingest states, citations/source dates and verification coverage, per-provider failures; CUDA checks never assert hardware execution without evidence. | UI5-6 / P5-X-1..4 and knowledge owners |
| @docx/@pdf/@xlsx/@img/@scans | Real payload preview/type/size, ingest/OCR progress/cancellation, page/sheet/source attribution and unsupported modality reason. | UI5-6 / P5 ingest; image model path P3 |
| /cache stats; /cache purge; $storage-level | Accurate tier/scope inventory; confirmation names exactly what is purged; preserve eventlog/CAS audit unless their owner explicitly defines otherwise. | UI5-6 / P5 storage owners |
| /nebius-status; /contree-branches; $nebius-branching; remote gateway | Real branch/snapshot ids, parent relations, status/diff/proof and environment/tier; switching or running uses authorized backend calls. | UI5-7 / P6-X-1/2 |
| $budget; /datalab-sync; /datalab-finetune; $nebius-datalab/speculative | Budget provenance/warnings, opt-in export scope/PII review/consent, actual export/job outcomes and cancellation; no guessed API completion. | UI5-7 / P6-X-3; telemetry/models |
| /research status/report/resume | Goal, iteration/evidence ledger and checkpoint; resume is distinct from starting again; inspect contradictory/failed findings. | UI5-8 / P7-X-1 |
| /parallel-dag; /concurrency-stats; $concurrency | Graph/list fallback, pools, leases, blocked edges and verified frame cancellation; no untrusted arbitrary resource edits. | UI5-8 / P7 runtime-dag/leases |
| /hack; /redteam; /screenshot; /browser; /ui-diff; $browser | Findings with source/evidence; bounded browser/a11y/artifact inspection; open/save only by explicit safe action, no automatic terminal escape/image execution. | UI5-8 / P7-X-2/3 |
| $mcp; $skills | Trusted source/capability inventory, connection/mount failures and actual enabled state; never silently expand permissions. | UI5-8 / P7 ext owners |
| /ci; /ci-run; /nix-flake; $env; $ci; $needle | Environment/image/CI step/cache evidence, generated-flake review and real results; all execution stays in approved execution tiers. | UI5-9 / P8 DevEnv/nano |

## 7. Verification design and evidence requirements

Every nontrivial unit starts with a JSON descriptor under `specs/descriptors/ui_<logic>.json`, using
prompt §6.3 fields, or extends an existing `uimodel_*.json`/CLI descriptor. Generate/maintain actual
tests, explicitly execute any red/ignored-red case to observe failure for the intended reason, then
implement and record green. Ignored or missing-environment tests cannot supply PASS. An unavailable
scaffolder is a tooling prerequisite to resolve, not permission to silently skip descriptors.

| Layer | Required cases / evidence |
|---|---|
| Pure model | Questionnaire None exclusivity/no defaults, optional/required answers, per-question notes, scoped keymap, all-at-once validation, focus/panel resize preservation; existing suite regression; candidate/apply, submit rejection restore, interleaved sessions/seq gaps, recovery atomicity, bounded paging, unknown metrics, overflow and clock rollback. Properties for Unicode edits, chip lifetime, exact copy and deterministic replay. |
| Layout/dump | Text and styled-cell snapshots at 60×20, 80×24, 100×30, 120×40, 160×60 and 200×60; below-minimum and zero/tiny inputs; dark/terminal/ANSI/no-color/ASCII/reduced-motion. Welcome, draft, conversation, completion, every sheet, approval, roster and error/recovery. No hidden clickable cells. |
| Scheduler/cache | FakeClock, flood and event-order assertions; idle/identical output bytes = 0 after setup; cursor-only minimal output; final pending dirty frame is not lost; cache hit/miss counters and generation eviction. |
| Presenter/lifecycle | Recording writer and injected short-write/Interrupted/WriteZero/flush errors; balanced sync controls, correct final caret; acquisition failure at each step, repeated close, EOF/cancel/unwind. Assert cleanup attempts and errors; cannot assert physical success when transport is dead. |
| Real binary/ConPTY | Default launch, valid/invalid options, dump determinism, actual key/mouse/paste stream, mid-edit cursor, Unicode wrapped selection, overlays, rapid type→click, resize storms, cancellation, clean terminal teardown. Test adapters never modify the user's clipboard/terminal settings. |
| Contract/security | Key-trace create-new/no-overwrite/symlink/error cases and canary exclusion for typed draft, paste, question answers/notes and provider secrets; grammar/help availability drift, typed receipts, approval binding/expiry/replay, CAS failure and secret redaction, untrusted controls/OSC/bidi, malicious paths, stale async results and schema compatibility. Hostile fixtures generated at test time. |
| System | Real ask_user → answer → frame continuation, requester-only interrupt with siblings running, answer/cancel race, cross-client/revision rejection, duplicate Enter, and answered/pending/interrupted replay; real session create/resume across process restart, safe shell execution only inside the sandbox, blocked false completion, replay/view hash; existing headless JSONL/exit/RTK/doctor contracts preserved. Synthetic fixtures prove rendering only. |
| Formal/fuzz | Miri on compatible pure logic; Kani bounded arithmetic/queue/geometry and decoder-state harnesses; WSL fuzz for input, safe style parser, text maps/completion and projection payloads. Record bounds and unsupported native dependencies. Loom only for new concurrent primitives; prefer existing channels. |
| Manual | Windows Terminal + Alacritty actual keys, paste, selection, resize, glyph/font/color/cursor restoration; Linux/WSL compile and PTY leg. Record exact versions and physical conditions, separate from ConPTY cell evidence. |

Future implementers must inspect current runner/profile support first; the following are intended
verification commands, not claims that every future suite or xtask subcommand already exists:

```powershell
cargo test -p polaris-ui-model -p polaris-ui-cli -p polaris-cli
cargo xtask check-members
cargo xtask check-layers
python -I @prime/tools/workspace.py check-docs
python -I @prime/tools/workspace.py render-docs --check
```

Add targeted polaris-tests, ConPTY/WSL/fuzz/Miri/Kani, on-demand policy and dependency checks as their
cases land, using the established main test strategy. Do not run a nonexistent future command and
describe it as passed. Do not update PHASES_STATUS to PASS until raw commands/output/conditions exist.
Attach evidence to main task IDs plus UI task/module IDs without rewriting historical records.

## 8. Resource and performance acceptance

**All numbers here are design targets / initial measurement workloads, UNVERIFIED at planning time.**
PLAN §11.4's **<8 ms TUI frame render at 200×60** remains binding. UI0-1 freezes workload, percentile
and measurement boundary before optimization: report p50/p95/p99/max layout+diff CPU separately from
writer/flush and physical display latency. A buffer benchmark cannot establish screen latency.

- 60 fps is a ceiling; idle emits zero application frame bytes after capability setup/timeouts.
  Measure idle wakes/CPU rather than invent a zero CPU claim.
- Record keystroke→state, keystroke→commit, kernel-event→visible-state and cancel→ack latency under
  simultaneous 100k-event replay, token bursts, mouse moves, resize storms and slow/failing output.
- Measure 1k/10k/100k logical-entry transcripts with paging, cold/warm width/theme changes, repeated
  composer edits and tool expansion. Report loaded bytes, cache size, high-water RSS and allocations.
- Enforce §8.1's finite input/output/transcript/cache/undo/history/worker/shutdown/dump limits;
  UI0-1 records feasibility and measurement boundaries before implementing the relevant buffer.
  Existing text/image/attachment caps in §4.2 are ceilings, not excuses to duplicate payloads.
- Benchmark on the documented reference PC with recorded current toolchain/terminal/dependency
  versions; do not inherit PLAN's older machine observations as fresh measurements. Add a slow
  terminal writer workload; bound retries and cancellation, not merely the fast TestBackend case.
- A failed budget is FAIL with measured evidence. Reduce work or defer an optional visual effect;
  changing a target requires a documented requirement decision, never a quiet threshold increase.

### 8.1 Initial finite resource limits

These are **proposed requirements**, not measurements. UI0-1 audits implementation feasibility and
records the exact benchmark reference; changing a ceiling needs a recorded requirement decision,
not a weakened test. Enforce byte and count ceilings before allocation, including checked arithmetic.

| Resource | Initial ceiling / behavior |
|---|---|
| Semantic input queue | 1,024 records and 2 MiB retained payload bytes; one normalized paste still obeys the 1 MiB text cap. Backpressure/reject visibly; cancellation has a separate reserved path. |
| UI domain intake | 1,024 fixed-size records; preserve durable ordering and recover broadcast lag through replay. Do not change the kernel bus's existing 1,024-command/4,096-event capacities merely for the UI. |
| Loaded transcript | 1,024 entries or 16 MiB resolved source, whichever is reached first. Page long sources in bounded chunks; do not silently evict selected/pinned source. |
| Derived layout/pane/source-map caches | 32 MiB combined; evict obsolete generations/LRU unpinned entries, retain authoritative CAS references. Active display-cell maps are bounded by the viewport. |
| Composer undo / prompt-history windows | Each at most 256 entries and 8 MiB metadata/text deltas. Share attachment payload ownership; no full decoded-image duplication in snapshots. |
| Encoded output frame | 8 MiB; retain at most one staged frame and one in-flight write. Backpressure presentation rather than accumulating frames; never discard domain records to reduce paint load. |
| Live/dump viewport | At most 400×200 and 80,000 cells. Validate before Ratatui buffer allocation and on resize. Oversized live terminals use a bounded viewport/size notice; invalid dump sizes return typed errors. Tiny pure-layout cases remain tested. |
| Cold service workers | Four concurrently, at most one native clipboard operation. Supersede obsolete queries with generation ids; isolate uncancellable native calls in a killable helper rather than promise a thread can be cancelled. |
| Human request | One pending per frame; 32 per session; up to eight questions and 16 named options per choice question plus None. Request CAS payload at most 256 KiB. Over-limit tool requests receive a real validation error. |
| Human answer | At most 16 KiB free text/notes per question; the aggregate **128 KiB serialized payload cap is authoritative**, including ids/envelope/encoding overhead. Per-field ceilings are subordinate and need not all fit simultaneously. Show remaining aggregate capacity and reject an over-limit batch without losing edits or truncating; validate both sides. |
| Timing defaults | 450 ms word double-click; 1,000 ms empty-draft Ctrl+C exit confirmation; 100 ms optional capability reply window; 750 ms idle before automatic question attention (§3.4). |
| Shutdown | Five-second cooperative grace for owned cancellable work before reaping killable helpers. Preserve/report unfinished persistence or teardown; this is not a guarantee about an uninterruptible OS write or physically broken terminal. |

Blocking terminal output and uncancellable native calls must not occupy the kernel/control executor.
A slow writer can delay visible feedback even after cancellation is accepted: measure and report that
separately from cancel admission/ack. No unconditional physical screen or teardown latency claim.

## 9. Implementation sequencing and module inventory contract

[UI_PHASES.md](UI_PHASES.md) is the executable dependency/gate map; [UI_MODULES.csv](UI_MODULES.csv)
maps logical modules to it. Existing `workspace.toml` remains authoritative for packages and main
generated crate tables. Do not generate packages from the CSV.

CSV schema: `module_id,surface,module,owner,path,path_state,ui_tasks,main_phases,depends_on_modules,
plan_anchor,demo_sources,disposition,safety_contract,acceptance,status`. Multi-values use `;` inside
quoted CSV fields. `path_state` distinguishes `EXISTING`, `PROPOSED` and `REFERENCE_ONLY`; paths name
one owning file/directory, not a claim every future file exists. Main phase values use P0–P10;
dependencies refer to module IDs, while backend prerequisites are spelled out in the phase tasks.
`status` tracks UI implementation coverage (`NOT STARTED`, `IN PROGRESS`, `PARTIAL`, or `DEFERRED`); it does not overwrite crate status or imply phase acceptance. Use `PARTIAL` when only a subset of a logical module exists or its acceptance evidence is incomplete.

## 10. Deferred web welcome and desktop

### 10.1 WEB-WELCOME — future welcome-page design, no full web application plan

Reference only `webui/index.html` and its welcome CSS/assets: black canvas, centered restrained
heading “What’s on your mind for Polaris?”, locally served display/body/mono fonts, rounded black
composer, model/mode/effort popover, sparse navigation silhouette and green aurora/stars behind the
content. Keep atmosphere aria-hidden and input contrast independent from it. Responsive composition,
keyboard focus, labelled controls and motion/transparency/forced-color fallbacks are requirements.

Do not import sample workspace files, goals, account data, dead demo links or hard-coded model names.
Do not analyze/adopt chat, history, usage, settings, models or states web pages for this request.
Future real controls need actual capabilities; static welcome must omit unavailable actions or label
them visibly. Later detailed planning retains Axum + Leptos SSR/PWA (D6/ADR-012); no Tauri and no
new general handwritten JS beyond the already declared service worker policy.

### 10.2 WEB-OPT — separately gated deferred welcome optimization follow-on

This follows WEB-WELCOME and has its own verdict/evidence; visual acceptance cannot satisfy it by
copying demo CSS. Restrict hydration, delivery and cache changes to the welcome shell and its public
assets. Broader application/PWA/session caching and other web pages remain later planning.

1. **Baseline first:** inventory welcome-only CSS/assets/fonts and transferred bytes; measure cold/warm
   TTFB, visible shell/LCP, layout shift, interaction readiness, animation frame time, CPU/GPU and
   idle/background-tab activity under recorded desktop/mobile viewport, cache/network and device
   conditions. No chatlog “<2 ms” or other unmeasured speed claim.
2. **Asset work:** split welcome CSS from other-page rules; remove unused resources, subset/license
   fonts, compare WOFF2 with original TTF, use font-display/layout-stable fallbacks and preload only
   the critical face. Compress/cache hashed assets; do not preload the entire application.
3. **Atmosphere work:** compare the demo's multiple oversized animated gradients, repeated stars,
   grain blending and masks against fewer layers/static alternatives. Keep animation on measured
   inexpensive properties, cap raster/compositor size and pause offscreen/background work. Current
   composer rules override generic glass blur: inspect computed styles before optimizing a blur
   that is already disabled there. Static reduced-motion/transparency mode is first-class.
4. **Delivery work:** SSR the useful welcome shell; establish which controls need hydration/islands
   and avoid shipping unrelated application WASM for decoration. Preserve CSP and accessibility;
   assess actual emitted requirements rather than broadening CSP speculatively. PWA caching must
   not cache secrets/session data or serve stale unsafe behavior; full PWA design remains later.
5. **Gate:** after baseline, freeze numeric transfer/runtime budgets and compare before/after on the
   same conditions. Require no visual/accessibility regression at mobile/desktop breakpoints,
   reduced motion, no-JS static shell and forced colors. Save measurements separately from visual
   screenshots. WEB-WELCOME may pass visually while WEB-OPT remains NOT STARTED/FAIL.

### 10.3 DESKTOP-LATER and full web/daemon UI

Declare only the future seam: shared domain events/tokens/presentation concepts, Slint desktop,
P9 authenticated RPC/daemon and ADR-014 licence gate. No desktop screens, Slint renderer work,
full web interaction design, SSE/terminal web implementation or mobile work is scheduled here.
Existing P9 backend progress neither authorizes this work nor removes its future UI gates.

## 11. Documentation and implementation verification boundary

Check this document and companions for valid links, CSV parsing, unique module/task IDs, dependency
resolution/acyclicity, complete task coverage, main-phase mapping, source path existence and explicit
deferred status. Run existing `check-docs` and `render-docs --check`; preserve all generated blocks.
The original planning refresh updated only UI_PLAN.md, UI_PHASES.md and UI_MODULES.csv; it added no runtime evidence. Subsequent authorized CLI implementation is recorded in the dated checkpoints below. Runtime safety, end-to-end rendering, keyboard support, performance and full feature completion require the separate UI_PHASES gates; current partial implementation does not satisfy them.


## 12. Reference parity implementation checkpoint (2026-10-25)

The current authorized CLI repair is described in [reference parity evidence](evidence/ui/parity-verification.md) and PHASES_STATUS §E37. It adds typed local command screens, all 27 reference preview names plus questions/editor, a composer-replacing form and explicit Submit review, local approval controls, safe native input repairs, semantic multi-entry selection, rich source-mapped transcript styling and a persistent bounded row cache. Demo data is visibly labelled and has no provider/kernel approval authority.

The reference binary was dumped for all 27 original scenarios with isolated configuration; the config directory was not created. Actual app frames are captured at five viewport sizes through the same function used by the terminal. This is coverage evidence, not an assertion of pixel-perfect parity or physical Alacritty behavior. Questions are a new requirement, not a screen found in dist.

A frozen CPU-only 200×60/56-entry/64-sample microbenchmark records uncached, cold-cache and warm-cache distributions for debug and release separately. Cold reflow, full paging, representative 1k/10k/100k transcripts, allocation/RSS/slow-write and physical display budgets remain open. The renderer retains its existing bounded visible-source window; the cache does not establish complete 16 MiB transcript paging. Real P3 human/provider protocols, OS clipboard images, full settings error/restart flows and manual terminal/Linux acceptance remain independent gates. Web welcome, WEB-OPT and desktop are still deferred.

### 12.1 Mouse and selection repair checkpoint (2026-10-25; E38)

[Current repair evidence](evidence/ui/mouse-repair-verification.md) extends E37 with native mouse/window/extended/VT mode acquisition, Quick Edit suppression and exact fallible mode restoration. A successful input close is idempotent and cannot reapply raw mode after terminal cleanup; a failed close retains its snapshot for retry. The profile picker applies a clicked visible row through the same guarded action as Enter, so switching into Demo does not require a second confirmation key.

The app uses one gesture contract for composer, transcript and question answer/notes selection, with grapheme/word/logical-line granularity and source-only hit/paint geometry. Padding has no selectable source; it may position/clamp the caret without selecting the last word. Mouse-up preserves editor ranges for selected deletion, replacement and Undo. Tests cover direction reversal, Unicode, soft wrap, chips and late copy completion. Four actual app buffer exports were inspected; they are not physical Alacritty screenshots. The 422 scoped passing tests and native OS-mode readback are explicitly separate from physical terminal, OS clipboard, full strict lint/formal/performance and UI/M1 acceptance.
