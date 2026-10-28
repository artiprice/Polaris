# Polaris — CLI UI phases and acceptance gates

> **Updated:** 2026-10-25 (continuation of 2026-10-24 planning) · **Status:** implementation in progress; phase gates remain open and all acceptance claims are partial or not started.
> **Design/contracts:** [UI_PLAN.md](UI_PLAN.md) · **Inventory:** [UI_MODULES.csv](UI_MODULES.csv)
> **System:** [PLAN.md](PLAN.md), [PHASES.md](PHASES.md), [PHASES_STATUS.md](PHASES_STATUS.md).

## 0. Rules and scheduling

These are UI work packages inside the main phases, not a replacement P0–P10 schedule. UI0–UI4
complete the deferred P1 CLI scope; UI5 is sliced by the backend's actual phase; UI6 contributes
to P10. Implementation has started. Existing source and newly added modules are reused only after review; their presence or focused tests do not grant a phase PASS.

Dependencies in tables are UI task IDs. An explicit backend prerequisite is equally binding:
start consumers only after its public contract freezes, and exit only after prerequisite acceptance
evidence exists (main PHASES §0). Key-dependent gates are BLOCKED-ON-ACCESS when access is absent;
do not mark keyless fixture tests as live integration. Independent completed backend work can be
consumed after audit without redoing its whole phase.

Each task is still expected to follow descriptor/spec → observed meaningful red → implementation → green + raw evidence. For existing work, record missing pre-implementation red evidence honestly; do not reconstruct it.
Extend existing descriptors where applicable; no duplicate fake adapters. Follow prompt.md, D10,
ADR-001/003/004/006 and UI_PLAN §0.2. No handwritten unsafe, lint-config additions, stub services,
discarded fallible results or weakened checks. Record unsupported manual/native/formal cases.

**Evidence destination:** append dated UI task/module references to the existing main evidence
workflow in PHASES_STATUS (and linked evidence files) when implementation actually happens. Do not
edit historical P1 non-UI PASS or count these proposed tasks as completed generated crate rows.
No separate UI status ledger is created in this planning task.


### 0.2 Historical E36 coverage and open gates (updated 2026-10-25; work began 2026-10-24)

| Area | Current evidence | Status / boundary |
|---|---|---|
| UI0 contracts and terminal feasibility | Safe Windows record decoder, bounded CSI/paste and single reader owner; observable cleanup and pure guard restoration-call assertions pass. Direct raw/mouse OS-mode inspection, physical keyboard/terminal matrix and Linux leg remain open. | **PARTIAL**; U-G0 remains open. |
| UI1 shared model | Current config/model/renderer command passed 263 tests combined. Composer selection/word editing and no-default/None questionnaire contracts have focused evidence. | **PARTIAL**; U-G1 remains open. |
| UI2 pure renderer | Shared textarea geometry, typed clipped hits, hover and single-entry transcript selection; actual interactive welcome/question/settings/editor exports reviewed. Cross-entry selection and full overlay/performance acceptance remain open. | **PARTIAL**; U-G2 remains open. |
| UI3 terminal shell | Final self-contained native ConPTY target: 15 passed. Shift+Enter, unbound Ctrl+Enter, effort/caret placement, dollar delivery, literal paste, idle output and restoration covered. Physical/Linux matrix and full terminal fault matrix remain open. | **PARTIAL**; U-G3 remains open. |
| UI4 offline app | Current selected app command: 87 passed (58 library, 2 RTK, 3 trace, 3 session, 6 startup, 15 terminal). Isolated demo, clipboard/index/settings workers and integrated controller tests added. Separate headless target: 2 passed, doctor readiness failed on missing prerequisites. Native clipboard and full settings error/restart gates remain open. | **IN PROGRESS**; no M1/UI acceptance verdict. |
| UI5 P2–P8/P3 human workflow | Scripted demo exercises three-question form and local cancellation only. No real human-request protocol/dispatch/service/persistence or requester interruption integration. | **PARTIAL only for questionnaire/controller state; backend slices NOT STARTED.** |
| UI6 security/performance/release | The continuation all-target safety Clippy pass for config/model/CLI/app exited 0 with the five selected unsafe-pattern lints; non-fatal style warnings remain. Cargo deny licenses/bans/sources passed; full advisory check could not acquire read-only cache lock. No cold-service allocation profile, performance baseline, physical-terminal report or release evidence. | **PARTIAL safety checks; remaining gates NOT STARTED.** |
| WEB-WELCOME / WEB-OPT / DESKTOP-LATER | Remain separately deferred. WEB-OPT has no measured baseline. | **DEFERRED**. |

Current raw command/output records and limitations: [continuation checkpoint](evidence/ui/continuation-verification.md), [machine-readable index](evidence/ui/continuation-verification.json), [native 15-test output](evidence/ui/continuation-final-native-selfcontained.stdout.txt). The 350 scoped passing tests do not include the separate failing doctor readiness assertion. Historical foundation [full package result](evidence/ui/final-tests.json), [active-only CAS meaningful red](evidence/ui/active-payload-red.json), and [service evidence/red gap](evidence/ui/continuation-services-verification.md) are retained. No UI phase or M1 acceptance is claimed.

Specific observed reds include controller frame clear (4,923 emitted bytes), queue pressure, resume race, help capability filtering, text-chip submission and edit overflow, terminal coalescing and Ratatui style-reset output, and the active-only CAS payload regression. The initial broad red gate is still missing across model/renderer/terminal work; model-initial-red.txt records a missing-API compile failure, not an observed behavioral assertion. Keep that gap explicit.

### 0.3 Reference-design repair (2026-10-25)

**VERIFIED:** the repair run passed 92 selected tests: 46 UI CLI and 46 app, including nine actual PTY scenarios plus a VT-parser regression. Welcome glyph/color comparison against saved demo buffers has zero differences outside the dynamic footer across five sizes. The native path verifies slash → completion → Help → Esc, RGB without COLORTERM, literal paste, signed receipts, idle output, cursor/newline and observable cleanup. [Raw command/output index](evidence/ui/repair-verification.md).

The first implementation snapshot above remains historical. Full UI phase acceptance is still **PARTIAL**: no model execution profile, native clipboard/selection, full settings persistence, performance release baseline, Linux or physical-terminal matrix is implied by this repair.

**Cold-service continuation (2026-10-25):** text clipboard, bounded workspace index and global preference persistence now have app-owned service/config code plus focused tests. The service tests do not touch the user's clipboard. Full settings-screen save/retry/restart flows and real OS clipboard behavior remain unverified; see [continuation evidence](evidence/ui/continuation-services-verification.md). UI4-3/UI4-4 remain partial and U-G4 remains open.

### 0.4 Reference parity continuation (2026-10-25; E37)

[Current evidence and remaining failures](evidence/ui/parity-verification.md) supersede E36 coverage where explicitly stated. UI2-3/UI2-4 now include rich source decorations, presentation-only tool cards, multi-entry Unicode selection and retained bounded wrap-cache APIs. UI3 adds key-up-safe VT assembly, exact Windows mouse-release tracking and direct balanced keyboard/mouse protocol checks. UI4 adds an isolated local screen gallery, source-derived bottom sheets, typed slash routing and the real questions/review form. Source fixture values never supply UI5 backend acceptance.

The current test commands, actual five-size app captures, source provenance and debug/release microbenchmark distributions are linked in the evidence. The physical Alacritty command is `cargo run --offline -p polaris-cli`; Alacritty 0.16.1 is installed, but physical keyboard/mouse operation is UNVERIFIED. No U-G gate, M1 or whole-workspace PASS is inferred.

### 0.5 Mouse and selection repair (2026-10-25; E38)

[Exact current commands and remaining limits](evidence/ui/mouse-repair-verification.md) record 277 model/CLI-renderer tests plus 145 selected app tests, 0 failures, with one opt-in CPU benchmark ignored in this run. UI0-2/UI3-1/UI3-2 now have actual child-console readbacks for mouse/window/extended/VT input, Quick Edit disabled, and exact original-mask restoration; failure retry and idempotent close/drop ordering have focused regressions. This supersedes the earlier lack of OS-mode readback, not the missing physical-terminal matrix.

UI1-3/UI2-2/UI2-4/UI3-4/UI4-2 cover shared character/word/line gestures, source-only shading/hit targets, selected Delete/Backspace and replacement/Undo, direct Demo profile-row activation, question answer/notes isolation and privacy-safe mouse diagnostics. Four same-path app buffer exports were reviewed. Five selected safety Clippy rules and formatting have current evidence; full strict Clippy still has recorded failures. No UI gate, live P3 workflow or M1 PASS follows; physical Alacritty/Linux, OS clipboard and wider formal/performance/release gates remain open.

### 0.1 Milestone map

| UI gate | Depends on | Main milestone / actual claim permitted |
|---|---|---|
| U-G0 contracts | UI0-1..3 | Design decisions/specs frozen; no product UI claim. |
| U-G1 shared state | U-G0 + UI1-1..5 | Reviewed reusable presentation model; no interactive TUI claim. |
| U-G2 deterministic renderer | U-G1 + UI2-1..6 | Pure/dump cell behavior under defined fixtures; no physical-terminal claim. |
| U-G3 terminal shell | U-G2 + UI3-1..4 | Safe terminal lifetime, input and buffered repaint evidence on tested transports. |
| U-G4 offline CLI | U-G3 + UI4-1..6 | Remaining UI portion of P1-X-2; M1 also requires the main P1 gate and evidence. |
| U-G5 execution | UI5-1 + accepted P2 prerequisites | CLI portion of M2; real sandbox execution, not a fixture demo. |
| U-G6 reasoning | UI5-2..4 + UI5-10 + accepted P3 prerequisites | CLI portions of M3 (local) and M4 (keyed, separately evidenced). |
| U-G7 MVP | UI5-5 + main P4/M5 prerequisites | CLI acceptance of PLAN §11.7, including visibly blocked falsified completion. |
| U-G8 extensions | UI5-6/7/8/9 slices | Only the corresponding accepted P5/P6/P7/P8 capability; no blanket advanced-UI PASS. |
| U-G9 release | UI6-1..4 + relevant main P10 gates | CLI evidence for M6; full submission still needs all main acceptance. |

```text
UI0 → UI1 → UI2 → UI3 → UI4 → M1 UI acceptance
                              ├─ UI5-1 (P2) → M2 UI acceptance
                              │    └─ UI5-2/3/4/10 (P3) → M3/M4 UI acceptance
                              │                   └─ UI5-5 (P4) → M5 UI acceptance
                              └─ UI5-6/7/8/9 (individual P5/P6/P7/P8 prerequisites)
Relevant completed slices + UI6 + P10 → CLI release acceptance
P9 later: WEB-WELCOME → WEB-OPT (separate evidence); DESKTOP-LATER
```

## UI0 — contracts, terminal feasibility and decisions (P0/P1 bridge)

**Entry:** planning is accepted for later implementation, current source/status and workspace policy
are inspected. **Exit U-G0:** no unresolved safety/ownership decision for the P1 shell; later backend
gaps are assigned to their real phases rather than hidden behind stubs.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI0-1 | Contract/API + bound audit: current SDK/schema/KernelApi/EventSource/CAS/config/telemetry; sequence scope, receipt reconciliation, availability, payload paging and image ownership. Audit/enforce UI_PLAN §8.1 numeric queue/byte/cache/undo/history/dump/shutdown limits; freeze overload policy and benchmark measurement workload. | — | Contract matrix with actual signatures/owner/version and available/missing classifications; interleaved-session and receipt failure descriptors; finite budgets with reasons. Missing P2/P3+ contracts become explicit owner tasks. |
| UI0-2 | Safe terminal capability spike, linked to S0-10. Audit crossterm/platform safe acquisition/restoration, enhanced keys, paste, resize, mouse, output synchronization and cursor state. | UI0-1 | Real-binary ConPTY plus Windows Terminal/Alacritty manual key table with versions; observed fallback chords; unsupported features visible. No unsafe Win32 copy. Manual gaps keep related claims UNVERIFIED. |
| UI0-3 | Freeze design drift and integration contracts: North Star/#141414 migration, keymap context/precedence, non-TTY behavior, CLI backward compatibility, config location/override semantics, descriptor/fixture provenance. | UI0-1 | Reviewed decisions + descriptors. Default non-TTY invocation returns an actionable error without escapes; explicit --dump and headless subcommands work without TTY. Preserve existing command syntax and output; no silent simulated default. |

## UI1 — shared model and safe action semantics (P1)

**Entry:** U-G0. **Exit U-G1:** model tests and new failure properties pass, existing tests are retained
or explicitly migrated for a changed requirement; no terminal/backend I/O leaks into the model.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI1-1 | Review existing tokens/logo; migrate current-demo North Star, prompt and selection surfaces/provenance. Preserve web token roles as needed; do not accidentally restyle deferred surfaces. | UI0-3 | Exact glyph/ASCII grids and palette/style assertions; old→new requirement record; shared token export regression. |
| UI1-2 | Context/focus/overlay/keymap state. Extend current keymap rather than duplicate it; help/footer generated, effective chords filtered by capability; prepare later roster contexts. | UI0-2, UI0-3 | No ambiguous chords per context; Esc/Ctrl+C priority, form/menu/tool/stats isolation, Ctrl+Z/Y, candidate-vs-applied properties. |
| UI1-3 | Review existing composer/history/chips; add transactional submission, actual payload ownership contract, bounded undo/history and rejection restore. | UI0-1, UI1-2 | Unicode/property/boundary tests; rejected/overloaded/ambiguous send preserves complete draft; no duplicate committed send; image bytes/geometry/lifetime/cap tests. |
| UI1-4 | Review reducer/view projection, sequence/session rules, bounded loaded history, explicit unavailable/stale metrics and recovery state; payload-dependent updates are atomic. | UI0-1 | Deterministic snapshots; gap/out-of-order/interleaved session, missing payload and stale-generation tests; counters distinguish unknown and known zero. No missing event silently treated as completion. |
| UI1-5 | Pure cockpit panel/navigation state: selected view, explicit pin, focus, independent scroll/filter and capability availability. No P3 questionnaire dependency for the offline shell. | UI1-2, UI1-4 | Default panel closed; focus never changes submission target; responsive transitions retain state; unavailable views show reasons and no synthetic domain data. |

## UI2 — deterministic renderer and dump (P1)

**Entry:** U-G1. **Exit U-G2:** binary-level dump uses the same render path as interactive mode;
synthetic scripts are visibly identified and do not open backend/network/clipboard services.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI2-1 | Shared responsive geometry/header/welcome; protected rows, min-terminal notice and size validation. | UI1-1, UI1-2 | Layout matrix at tiny/below-minimum/60×20 through 200×60; hero thresholds, optional account label, clipping and no overlap/underflow. |
| UI2-2 | Composer/footer/status painter and one shared Unicode wrap/caret geometry. | UI2-1, UI1-3 | Styled/text snapshots; 3/8 bound and protected space; first-visible-row pointer; history row guards, placeholder exclusion, mode/effort/draft integrity. |
| UI2-3 | Transcript/tool/code/diff rendering, semantic source map, safe output text boundary and generation cache. | UI2-1, UI1-4 | Exact semantic copy versus gutters; hostile control/bidi/style tests; thread/width/theme/content invalidation; append and composer-only cache reuse counters. |
| UI2-4 | Opaque completion/overlay framework, bottom sheets, selection/To latest, hit occlusion and current-state geometry refresh. | UI2-2, UI2-3 | Type→click same-batch, wrapped wide glyph/word drag, hidden target rejection, scroll anchoring and overlay restoration assertions. |
| UI2-5 | CLI args/default branch and --dump WxH/--scenario real render path; preserve existing subcommands; optional positional prompt pre-fills without auto-submit. | UI0-3, UI2-4, UI2-6 | Real help/version/invalid-options/dump tests; identical plain and styled checkpoints, bounded sizes and no escapes in plain output; non-TTY error; existing JSONL/doctor/rtk/sandbox contracts unchanged; key-trace create-new/no-overwrite/symlink/error and no-content-canary cases. |
| UI2-6 | Responsive optional panel and narrow sheet, shared geometry/occlusion, F4/F6 and /panel metadata. Tasks/Agents/Questions/Review/Proof/Context/System content is capability-gated. | UI2-1, UI1-5, UI2-4 | 120×28 breakpoint and 34-column panel; shrink/grow focus/scroll/draft preservation; no hidden hit targets or second composer; default quiet layout. |

## UI3 — safe terminal lifetime and demand-driven repaint (P1)

**Entry:** U-G2 and audited safe adapter coverage. **Exit U-G3:** normal/error/unwind paths attempt
all owned cleanup, failure reports remain honest, measured terminal bytes/cursor behavior satisfy
the presenter contract; physical/manual limitations are explicitly recorded.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI3-1 | App-owned terminal guard, fallible writer/error aggregator, explicit close and setup rollback; bounded background-worker ownership. | UI0-2, UI2-5 | Every acquisition step fault-injected; EOF, partial startup, writer failure, repeated close and injected unwind cleanup; no unconditional process::exit; terminal modes restored where transport permits. |
| UI3-2 | Safe input normalization and ordered scheduler; bounded queues, 4 ms/256-event batches, 60 fps ceiling, fair cancellation/event handling. | UI3-1, UI1-2 | Fake-time deadlines, no idle busy-loop, coalescing barriers, release/repeat handling, paste not submitted, floods with explicit overload/backpressure and bounded cancel latency. |
| UI3-3 | Owned buffered presenter with Ratatui as the sole cell diff, exclusive writer ownership and cursor staging, optional safe synchronized-output negotiation, bounded capability replies and fallback. | UI3-1, UI3-2, UI2-3 | Zero bytes for identical frames; cursor-only positioning; complete logical commit/final flush; split/late/paste-shaped replies; partial-write/flush cleanup followed by clean shutdown; no uncertain incremental retry. |
| UI3-4 | Interactive loop wired to renderer/input only, using genuine offline state and explicit unavailable backend actions; no simulated reply engine. | UI3-2, UI3-3, UI2-4 | Real ConPTY typing/middle edit/selection/resize/overlay/exit tests; current draft geometry before click; final insertion caret and visibility; terminal backend thread remains separate from kernel work. |

## UI4 — real offline session UI and preferences (P1 / M1 UI portion)

**Entry:** U-G3 plus current offline kernel/storage/config APIs. **Exit U-G4:** all P1 CLI actions
operate on real offline state, unavailable future features say so, sessions persist/resume and the
required M1 snapshots/smoke evidence exist. Full M1 also retains its other main PHASES requirements.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI4-1 | App bridge to real LocalBackends/KernelApi: session lifecycle, event subscription, bounded CAS queries, command receipts and recovery. Existing P1 owners must pass their scoped acceptance. | UI3-4, UI1-3, UI1-4 | Actual on-disk event/CAS session; restart/resume; receipt rejection/reconciliation; lag replay and atomic view swap; disk/CAS errors visible, no fake payload success. |
| UI4-2 | /sessions, /session resume, /status, /help, /clear, /quit plus directive-only input. Add availability metadata for future commands through existing grammar owner. | UI4-1, UI1-2 | Real command route/metadata contract tests; source layers in status; resume preserves D11; clear leaves audit durable; cancellation/shutdown joined and no accepted command silently lost. |
| UI4-3 | Completion snapshot worker, real workspace-ignore policy, safe clipboard/text/image adapters, text attachment preview, tool expansion and copy feedback. | UI4-1, UI2-4, UI1-3 | No render-time filesystem/clipboard calls; deny/junction/case/stale-index cases; real payload binding and error/timeout/cancel behavior; private test clipboard adapter, no user's clipboard mutation. |
| UI4-4 | /settings and preference adapter to project config owner; candidate/applied/save state, isolated launch overrides and unknown-field/version handling. | UI4-1, UI1-2 | Enter-only apply; per-field atomic persistence; restart; failed save leaves old file and marks active session choice unsaved; retry, corrupt/unsupported config and unrelated override preservation. No new demo %APPDATA% JSON store. |
| UI4-5 | /stats offline telemetry and reusable /usage presentation with real/empty/unavailable datasets. Strip/Ticks/Segments, Stacked/Lines/Trends only for available data. | UI4-1, UI2-4 | Same sums/buckets/denominators across styles; known zero vs absent, overflow, partial cache categories, timezone/date edges, exact-value inspector, no D/demo toggle in normal product. P3 usage collection remains gated. |
| UI4-6 | M1 UI acceptance: integrated offline interaction, deterministic dump checkpoints, source/style review and command backward compatibility. | UI4-2, UI4-3, UI4-4, UI4-5 | Real-binary snapshots at required sizes/fallbacks, session create/edit/directive/resume/quit; native + Linux check leg; manual supported-terminal table. Append P1-X-2 evidence without altering non-UI historical acceptance. |

**2026-10-25 interactive/service slice:** UI4-3 has text-only clipboard and bounded path-index workers, generation/revision-scoped paste, current-state mouse targets and single-entry selection. UI4-4 has typed CLI preferences, explicit per-field global persistence and a bounded save worker; untouched launch overrides and demo remain memory-only. Four integrated visual exports were reviewed. Full task rows remain open for OS clipboard success/failure, cross-entry selection, settings retry/restart/malformed config, native/Linux matrix and measured performance. The explicit labelled demo exercises interaction without granting backend acceptance.

## UI5 — backend-integrated CLI slices (P2–P8)

Do not make M1 depend on model keys or later phases. UI5-10 is not an M1/UI0–UI4 dependency. P3-X-5 contracts and per-frame concurrency are real P3 work,
not the future P7 scheduler and not a fake adapter. Each slice starts after U-G4 and its backend
contract audit; each exits only when its backend acceptance and required access are evidenced.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI5-1 | **P2 / M2:** sandbox lifecycle, WSL/OpenShell doctor/onboarding, sync progress, diff/blast radius, approval decisions and proof gate. Requires platform/OpenShell/sync/vcs/leases/guards/kernel contracts. | UI4-6 | Real sandbox create/exec/test/destroy and cleanup; command/token/scope/hash/expiry-bound approval, stale/replayed decision rejected; failure/denial surfaced; no host-shell fallback; fixtures cannot supply M2 PASS. |
| UI5-2 | **P3 / M3–M4:** real streaming model/tool transcript, provider/effort availability, pending/429/retry/cancel, plan/goal task inspection and read-only session replay, lossless folded-reference expansion. Requires models/token-factory/frames/context/kernel contracts. | UI5-1, UI4-6 | Local endpoint loop first; keyed services separate; no duplicate work after receipt reconciliation, safe secret refs, stream-gap recovery; historical mode visibly read-only; plan/goal updates acknowledge and replay deterministically. |
| UI5-3 | **P3 base / P7 extension:** /agents + F3 grammar/keymap, actual frame roster/thread routing. Requires real frame id/lifecycle/attribution and child projection APIs; missing APIs are owner work. | UI5-2, UI1-2 | Main remains submission/cancel target; accepted send returns Main; rejected send preserves inspected child/draft; per-thread scroll/tool state, seven lifecycle states and unavailable metrics; scoped wheel/keys; no timer-generated children. |
| UI5-4 | **P3 / M4:** /model /usage /quota and HUD/status from real model/telemetry/ratelimit data; initial HUD lives in /status to protect footer space. | UI5-2, UI4-5 | Actual source provenance, optional usage/cost/cache/quotas, no invented capacity/reset/prices; child/session totals reconcile; live and synthetic evidence separate; real keyed usage remains BLOCKED-ON-ACCESS without access. |
| UI5-5 | **P4 / M5:** S1/PII/worker status and main MVP script (PLAN §11.7), with human-question integration exercised where the script requires it; the baseline M5 script does not gain an unconditional UI5-10 dependency. | UI5-2, UI5-3, UI5-4 | Real sandbox proof on Rust/Python/CUDA-flavored tasks; falsified completion visibly Blocked; Completed only from proof event; replay checkpoints; GPU/nvcc skips explicit; S1 timing measured; no sensitive diagnostic payload. |
| UI5-6 | **P5 knowledge/context/ingest:** UI_PLAN §6.1 coverage for context map/expand, citations/search/NVIDIA verification, document/image/OCR ingest, cache stats/purge and storage scope. | UI4-6, UI5-2 | Each owner contract and P5-X-1..4 as relevant; source/page/sheet attribution, exact reference expansion, cancellable bounded ingest and real errors; purge scope/confirmation; keyed evidence separate. |
| UI5-7 | **P6 cloud/budget/branches:** branch tree/snapshot ids, remote tier/env, quota/budget sources, explicit opt-in Data Lab export/fine-tune and speculative-mode state. | UI5-2, UI5-4 | P6-X-1/2/3 and specific owner acceptance; real branch/diff/proof, export consent and redaction, job errors/cancel; missing access remains BLOCKED-ON-ACCESS. |
| UI5-8 | **P7 advanced agentics:** research goal/iterations/evidence/report/resume, DAG/pools/leases, hacker findings, browser/a11y/screenshot/ui-diff artifacts, MCP/skills trust and enabled state. | UI5-2, UI5-3 | Independent owner/P7-X-1/2/3 gates; restart resumes actual checkpoint, graph/list fallback and source-backed evidence; safe explicit artifact actions; no permission expansion by UI. |
| UI5-9 | **P8 DevEnv/CI/T0:** environment/image/flake inspection, real CI steps/results/cache evidence and Needle availability. | UI4-6 | Specific P8 owner contracts; authorized environment/CI actions with failure/cancel/replay; generated flake reviewed; no host execution shortcut or invented local-model measurements. |
| UI5-10 | **P3 human ask/answer:** SDK human request/answer types and versioned codecs; real ask_user host/tool service, durable request/answer/interruption transitions, per-frame waiting/cancel registry; pure form/model/popup and application adapter per UI_PLAN §3.4. Main prerequisite P3-X-5. | UI5-2, UI5-3 | Real request → atomic answer → requester continuation; siblings run; no defaults, None exclusivity, optional notes, required validation and exact Tab/Enter/arrow/Esc scope; duplicate/stale/cross-client answers, cancel race, over-limit payloads and restart with answered/pending/interrupted requests; main draft intact, approval separate. |

## UI6 — security, performance and release evidence (P10, incremental regression earlier)

**Entry:** relevant UI slices implemented; run cheap scoped regressions throughout, not just here.
**Exit U-G9:** CLI contribution to P10 passes with all material skips named; full M6 remains a main
system verdict. Do not treat unimplemented P5–P8 or deferred P9 UI as required for the M1 shell.

| Task | Work / backend prerequisite | Depends on | Required exit evidence |
|---|---|---|---|
| UI6-1 | Security/policy/formal review of completed CLI scope; Miri/Kani/fuzz and cross-crate authorization/redaction/terminal injection cases. | UI4-6 | Raw scoped reports incl. bounds/time/toolchain; zero own unsafe; dependencies vetted; unsupported native/formal cases explicit. Run the same review for later UI5 slices as they land. |
| UI6-2 | Scheduler/cache/memory/latency/load budgets (UI_PLAN §8); slow output, long transcripts, paging, input/event floods and repeated session/thread/resize churn. | UI6-1 | <8 ms frame target at 200×60 under frozen measurement definition, percentile tables and conditions; queue/cache/undo high-water caps hold; idle/identical bytes = 0; no silent event loss or cancel starvation. Failures remain FAIL. |
| UI6-3 | Manual terminal and portability matrix, final design/accessibility/source attribution review. | UI6-2 | Actual Windows Terminal/Alacritty keys, glyphs, mouse, cursor/mode restoration; ConPTY and WSL/Linux records separate; color/no-color/ASCII/reduced-motion review; no unverified universal keyboard claim. |
| UI6-4 | CLI dist packaging/docs/fixtures/evidence closure with main P10-4/5; rerun changed feature integration gates. | UI6-3 | Actual release artifact SHA256, startup/help/dump/offline smoke of packaged binary; available M2–M5 slices rechecked as required; backward compatibility; UI task/module/main status/docs agree; skipped live access clearly flagged. |

## Later — declared, deferred surfaces

These tasks are **DEFERRED**, not authorization or implementation-complete placeholders.

| Task | Scope / prerequisite | Depends on | Future acceptance |
|---|---|---|---|
| WEB-WELCOME | P9-X-1; later welcome-only detailed plan under Leptos SSR/Axum; inspect only demo index + welcome CSS/assets. Shared tokens/API changes follow their real contracts. | UI0-3 | Responsive/a11y/visual welcome comparison, real capabilities or explicit unavailable controls, no sample context/dead links. Full web application plan remains separate later work. |
| WEB-OPT | Separately gated follow-on, UI_PLAN §10.2; baseline then welcome-only asset/CSS/fonts/atmosphere/hydration/public-asset cache work. Broader application/PWA cache design remains deferred. | WEB-WELCOME | Before/after bytes, cold/warm timings and CPU/GPU/background behavior under recorded conditions, numeric budgets frozen after baseline; no-JS/reduced-motion/forced-colors/no visual regression. Cannot be closed by a screenshot. |
| DESKTOP-LATER | IN PROGRESS: desktop welcome + offline session view (real local kernel session via `polaris-ui-session`, composer v2, config-driven model list, no model replies); ADR-014 licence resolved (royalty-free); model execution (P3), P9 RPC/daemon `--connect`, attachments, speech input, search indexing and Models/Usage/Settings pages still open. | — | Evidence in `@prime/evidence/ui/desktop/desktop-verification.txt` (being updated) and live-1..4 PNGs; polaris-ui-gui 36 and polaris-desktop 17 tests; remaining desktop screens and the daemon bridge keep their own implementation/verification gates. |

## Planning review checklist

Checked items below mean document/source consistency only, not product implementation acceptance.

- [ ] Later implementer confirms scope/versions/contracts, then records statuses through the existing evidence workflow.
- [x] Every CSV module resolves to tasks above and real package ownership; every task has module coverage (56 modules / 41 tasks; strict CSV/path/owner/dependency checks).
- [x] Future domain API/event propagation is assigned to SDK/proto/kernel/runtime owners, codecs/tests/docs and UI consumers; no API implementation or layer inversion is claimed.
- [x] Historical P1 non-UI and backend evidence is not promoted to UI acceptance; this update does not alter main phase verdicts.
- [x] UI inventory IDs, task/dependency references, existing paths and plan anchors were validated; workspace check-docs passed.
- [x] python -I @prime/tools/workspace.py render-docs --check reports PHASES.md up to date (11 blocks) and PHASES_STATUS.md up to date (10 blocks) at this snapshot.
- [ ] Product performance, safety Clippy, manual keyboard/OS restoration matrix and complete phase acceptance remain open.
