# Polaris web interface

A dependency-free HTML/CSS design preview. Open [index.html](index.html) directly, or serve it locally:

    python -m http.server 4173 --bind 127.0.0.1 --directory webui

Then open http://127.0.0.1:4173/. Fonts and assets are bundled; the delivered pages need no JavaScript, npm install, CDN, or external service.

## Screens

| Screen | Purpose |
| --- | --- |
| [Welcome](index.html) | One heading, compact prompt, living green atmosphere, dense irregular stars |
| [Chat](chat.html) | Open responses, outlined prompts, compact file rows, and a two-row black composer |
| [History](history.html) | Linked sample conversations in divided rows |
| [Models](models.html) | Provider, model rows, and five effort levels |
| [Settings](settings.html) | Appearance, interaction, and collapsed session preferences |
| [Usage](usage.html) | Overview/Models tabs, calendar, chart, and explicit demo metrics |
| [Interface states](states.html) | Seven initially collapsed groups retaining every design example |

The shared desktop navigation is a 68px icon rail. Labels appear on hover and keyboard focus; every navigation link has an accessible name. Below 768px a native disclosure opens the mobile navigation. Context is closed initially and opens beside content on wide screens, or in flow on smaller screens. The navigation's small Design preview disclosure explains the prototype boundaries.

## Visual contract

The canvas and shell are pure black. Typography remains Space Grotesk / Inter / JetBrains Mono. The welcome shows only “What’s on your mind for Polaris?” and a compact composer. Its broad gray-green mesh, cloud, and aurora atmosphere fades into black.

The atmosphere combines a fluid mesh base, drifting nebula clouds, and two soft aurora ribbons. Four large layers move independently over 31–48 seconds using transform and opacity. Color stops and SVG blur filters remain static. The light spans the welcome and fades into black at the viewport edges; it has no circular boundary.

Three seeded SVG assets contain 2,400 unique one-CSS-pixel stars. Rectangular sampling combines scattered points with uneven clusters, including close to the prompt. There is no elliptical sampler or large text exclusion rectangle. A small feathered dark backing and text shadow protect the heading; the opaque composer covers stars behind it. A locally generated 128px grayscale PNG adds low-opacity grain to the light. Star layers twinkle over 13, 19, and 23 seconds without directional travel.

Only the welcome has this atmosphere. Reduced motion freezes every moving light and star layer; forced colors removes the decoration. Reduced transparency removes the light and grain. No runtime JavaScript, live fullscreen noise filter, or external service is required.

The composer is approximately 60px tall when empty and grows with multiline text up to a bounded scrolling textarea. Browsers without field-sizing use the native resizable fallback. The shared model menu is a compact 300px-wide opaque-black panel with a neutral rim, trailing selection checks, and separate hover/focus tones. Native radio groups retain model, mode, and low / medium / high / xhigh / max choices. Model selection updates the trigger label locally through CSS; mobile keeps that label accessible while showing only the chevron. Attachment and send actions are disabled because services are not connected.

Composers and textarea interiors are opaque #000000 in normal, focused, multiline, and fallback states, without backdrop blur. A neutral border provides separation. Forced colors may substitute system colors. Chat and file rows have black interiors and fine neutral outlines. Glass remains only on supporting examples elsewhere. The model menu is fully black and does not use backdrop blur. Other pages use open black space, divided rows, concise headings, and disclosures instead of nested panels. CLI accent meanings remain green for the brand, gold for plan/files, blue for goals/settings, and magenta for auto.

The chat is a separate composition: a centered 760px reading column, compact right-aligned outlined prompts, open responses, gold tool disclosures, black code blocks, and compact downloadable file rows. Its stacked composer places native text editing above the actions, with a right-aligned sample goal status and explicit preview note. Chat controls use 32px capsule outlines, #181818 hover and #242424 pressed fills, and clear focus rings; coarse-pointer targets are at least 44px. The chat draft grows with added lines up to 37.5% of the viewport height before scrolling internally. There is no fixed pixel ceiling: resizing the window taller immediately gives a long draft more space, and shrinking it keeps the controls reachable. The model menu opens above the bottom action row, over the draft, so long text does not push it off-screen. Empty-draft size and the welcome composer remain unchanged. Code scrolls locally and line numbers are excluded from selection. Chat has no atmosphere, gradients, noise, or backdrop blur. The welcome markup, atmosphere assets, and shared model-menu behavior are preserved.

The menu opens with a 160ms fade, 4px movement, and .985-to-1 scale. Three model rows have a small 15ms stagger; the trigger chevron, checkmark, hover, and segmented selections transition independently. Reduced motion disables these effects. Closing retains native immediate disclosure behavior. Touch targets are at least 44px tall under a coarse pointer, and limited viewport space uses internal scrolling rather than shrinking controls.

## Available native behavior

- Links navigate to real preview pages, sections, and bundled downloads.
- Native details/summary opens tools, menus, Context, preview information, state groups, and attachment previews. Fragment links into collapsed groups reveal their enclosing disclosure and jump immediately to its content.
- Browser-owned textarea editing supports immediate insertion, selection, paste, and line breaks. Enter inserts a newline; there is no custom send keymap.
- Model/effort radios and usage tabs demonstrate local selection. They do not update an agent or persist settings. The model summary label reflects the local selection; navigation does not save or transfer that choice.
- Settings contrast, larger text, and reduced motion change the current page through CSS. Session fields are local design controls and do not enable services.
- Pasted-content preview and download use the same 684-character fixture. Code gutters are excluded from selection.

Inference, sending/stopping, uploads, clipboard buttons, search/filtering, live completion or syntax highlighting, timers, authentication, provider connections, and persistence are not implemented. Unavailable actions are disabled or clearly labeled as static examples. Conversation and goal fixtures are marked sample; usage is labeled DEMO DATA. Downloading the sample patch does not apply it.

## References

- [NVIDIA Elements design contract](https://github.com/NVIDIA/elements/blob/main/DESIGN.md) and [semantic tokens](https://nvidia.github.io/elements/docs/foundations/themes/tokens/): consistent roles and hierarchy.
- [Google Material 3 Expressive research](https://design.google/library/expressive-material-design-google-research?pubDate=20250521): purposeful shape, scale, and grouping.
- User references: a spacious black welcome with atmospheric light, and matte artifact download cards.

These inform the Polaris design; no framework dependency or formal design-system compliance is claimed.

## Fonts and licenses

Original font files are self-hosted under assets/fonts/ with their SIL Open Font License notices:

- Space Grotesk: Google Fonts, ofl/spacegrotesk/SpaceGrotesk[wght].ttf.
- Inter: reused from the repository's NVIDIA Elements reference, with its Inter license.
- JetBrains Mono: the official JetBrains/JetBrainsMono variable font.

See assets/fonts/sources.json for provenance and checksums. The reference checkout is not modified and is not needed to open the built pages.

## Maintain and verify

Shared Python markup generates all seven checked-in HTML pages and three star assets. There is no runtime build step.

    python scripts/build_webui.py
    python scripts/build_webui.py --check
    python scripts/test_webui.py
    python -m unittest discover -s scripts -p test_*.py

Ten static checks cover generated output, landmarks, labels, links, assets/downloads, fonts/licenses, fixture totals, the native black chat replacement and real downloads, complete native menu controls with independent names/IDs, and deterministic star/noise/ribbon assets and irregular distribution.

    python scripts/test_webui.py --browser PATH_TO_CHROMIUM_HEADLESS_SHELL --output target/webui-chat-review/verified

The isolated Chromium harness explicitly selects ANGLE/SwiftShader in-process software rendering, with threaded animation enabled. Earlier default/disabled-raster configurations produced intermittent GPU mailbox failures; raw probe outputs remain in the review directory. The harness checks evolving rendered transforms and explicit animation phases. Hardware-accelerated smoothness remains a separate verification.

The harness checks seven pages at 1440×900, 1024×768, 390×844, and 320×760. It exercises overflow, native disclosures/focus, menu bounds, middle insertion, bounded long drafts, model/effort choices, settings, and usage tabs. Chat checks also cover real sample downloads, long filenames, local code scrolling, sticky-composer clearance, source-color contrast, tool expansion, short viewports, reduced motion, forced colors, and coarse-pointer capsule targets. Additional scenes check reduced motion, forced colors, native fragment reveal, and the mobile model menu. The menu is also checked with local model-label updates, every mode/effort choice, opening/selection states, short viewports, multiline drafts, reduced motion, forced colors, and an emulated coarse pointer. Coarse-pointer emulation does not establish physical touch behavior. The atmosphere is captured at 0, 12, and 24 seconds; assertions verify evolving transforms, fixed prompt geometry, stars close to its border, and opaque-black composer/textarea surfaces. Screenshots include normal layouts, expanded sections, downloads, long drafts, solid-surface fallback, and a 2× scale approximation.

Current chat redesign evidence is kept in [browser results](../target/webui-chat-review/browser-results.json), [raw browser command output](../target/webui-chat-review/browser-command-short-window.json), and [verification summary](../target/webui-chat-review/verification.json). Earlier design evidence remains in target/webui-review/, target/webui-minimal-review/, and target/webui-atmosphere-review/. Test JavaScript is confined to an in-memory loopback harness and is never shipped in webui/.

Expanded-draft sizing and live viewport-resize checks are recorded in [editor verification](../target/webui-expanded-draft-review/verification.json). These exercise incremental line growth, 37.5% viewport sizing without a fixed pixel ceiling, and toolbar-anchored menus in short windows.

Physical keyboard/OS clipboard checks, manual browser zoom, non-Chromium engines, and hardware-accelerated blur remain unverified. The solid-surface scene forces a fallback appearance rather than running an old engine. The Rust CLI, its distribution, and the NVIDIA reference checkout are outside this redesign; no Rust test result is claimed for it.
