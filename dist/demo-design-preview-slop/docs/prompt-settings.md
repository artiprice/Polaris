# Dollar settings and goal display

These controls are implemented in the local UI preview. They store structured session settings; web search, subagent execution, retrieval, link fetching and a backend goal runner are not connected. Model names and retrieval source names are labels, not validated services.

## Syntax

| Setting | Examples |
| --- | --- |
| Web search | $websearch, $websearch=on, $websearch=[on], $websearch=off |
| Subagents | $subagents, $subagents=on, $subagents=off |
| Subagent models | $subagents-model=[model1(low-xhigh), model2(high-max)] |
| Retrieval sources | $rag=[docs, source] |
| Session skills | $skills=[lint, review] |
| Reference links | $links=[https://example.com, https://example.org] |

Bare toggle names mean on. Both [one,two] and [one, two] are accepted. A list replaces the previous list; =[] clears it. Model effort can be omitted, a single level such as model(high), or a lower-to-upper range such as model(low-xhigh) or model(low-to-xhigh). Supported levels are low, medium, high, xhigh, max, in that order. Reversed or unknown ranges are rejected.

Settings stay in memory for the current session until explicitly changed, including across /clear and preview scenario changes. Restarting the app resets them. Repeated assignments use the last value. Each submitted prompt receives a snapshot of the effective settings; changing settings during a response does not mutate the earlier submission.

A settings-only entry applies without starting a response. Exact bare toggle names apply with one Return. Settings can also be mixed with a prompt and /plan or /goal directives, including settings on the first line and prompt text on the next. Both the submitted prompt and later prompts receive the resulting session settings. The skills list applies to the session until replaced; $skills=[] clears it. Skill names are stored as selection metadata and are not loaded or executed by this UI preview. During a response, settings-only entries remain available; another ordinary prompt remains a draft until work stops.

Typing $ opens name suggestions. Partial names can be completed with Tab or Return; list names open an =[ template. Existing =value text is preserved when completing a name. The full-width menu supports mouse clicks, wheel navigation and the existing lighter hover tone. Complete known setting names use blue foreground; partial names stay gray. Dollar-menu keyboard focus uses the same #89B4FA blue, with lighter #B4D2FF hover (LightBlue/LightCyan without RGB). Saved values are shown in the suggestion details so multiline submissions visibly update the session. These accents preserve surrounding backgrounds.

List entries may be wrapped in single or double quotes; quoted commas are retained inside an entry. Quotes are wrappers, with no shell or JSON escape expansion. Setting names start with an ASCII letter or underscore and may contain letters, digits, underscores and hyphens. Additional $name, $name=value and $name=[items] entries are retained as custom metadata, with no execution behavior. Settings inside backtick code or attachment payloads are not applied; dollar signs embedded in URLs or escaped with a backslash are literal. Invalid known values or malformed lists leave both the live draft and all stored settings unchanged.

/status shows saved settings under an explicit preview heading. PgUp/PgDn scrolls that popup.

## Empty editor and effort

An exactly empty editor shows What's on your mind for Polaris? in #525252 dark gray. The placeholder disappears when any text is entered. It is drawn outside the source map, so dragging or copying cannot include it. It is never submitted or saved in history. No-color mode inherits the terminal foreground.

/model, the footer and Shift+Up/Down share the five effort levels, including xhigh between high and max. These remain per-model preview preferences.

## Goal activity

Submitting a prompt with /goal starts a local goal activity state. A blue (#89B4FA, LightBlue without RGB) goal active (time) or goal paused (time) line appears right-aligned immediately above the editor. Its time accumulates active duration, excluding pauses. Escape interruption or the end of the simulated response pauses it. A later ordinary prompt resumes it; /clear or loading a scenario removes it. Popups and settings-only entries preserve the state. The label is display-only and excluded from conversation and editor copying.

This tracks preview response activity; it does not create a background autonomous goal or claim actual agent progress.
