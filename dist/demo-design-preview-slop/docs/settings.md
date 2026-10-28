# Settings

/settings opens a responsive black sheet with Appearance, Usage, Statistics, Interaction, Accessibility and Providers categories. At terminal widths of at least 100 columns, a 20-column sidebar sits beside the controls. Smaller terminals use wrapping category tabs. The title, save status, category navigation and closing hints stay visible while content scrolls.

Tab/Shift+Tab switch category/control focus. Up/Down navigate the focused region. Right or Enter moves from categories into controls, initially highlighting the applied value. Left/Right highlights choices without applying them; Right stops at the last variant, and Left from the first returns to categories. Enter applies exactly the highlighted choice; Space does nothing. Clicking an option only highlights it, and Enter confirms mouse choices too. Leaving a row or controls discards its unconfirmed candidate. Esc closes Settings, discards unconfirmed choices and restores the conversation draft and scroll; confirmed changes remain active. Categories and options support clicks, distinct gray hover, mouse-wheel scrolling and page navigation.

## Saved UI preferences

Changes apply and autosave immediately after Enter confirms each field. The applied marker remains separate from candidate focus, and a pending choice shows Enter to apply. Saved fields are bar style (default Strip), theme (Dark), color policy (Auto), glyphs (Unicode), and motion (Normal). Interaction mode remains session-only and also requires Enter. Manage providers opens the local Providers screen only on Enter. Provider/model configuration, model effort, dollar settings and conversation history remain session-only. No provider connection is made.

Windows stores preferences at %APPDATA%/Polaris/settings.json. Other systems use $XDG_CONFIG_HOME/polaris/settings.json, falling back to ~/.config/polaris/settings.json. POLARIS_CONFIG_DIR overrides the directory for portable launches and isolated tests. The path appears at the bottom of Settings. App instances constructed directly by library tests use memory only unless a store is explicitly attached.

Loading precedence is defaults → saved preferences → explicit launch options. --theme, --ascii, --reduce-motion and --color override that launch without writing the file. Settings labels affected rows as launch overrides. Explicitly changing a row saves that field and removes its override; changing another field does not save unrelated launch overrides. Color depth remains a terminal capability / launch option, not a saved setting. Auto color respects NO_COLOR and TERM=dumb; Always and Never are explicit policies.

Preferences use versioned JSON and preserve unknown fields. Missing files use defaults without creating a file. Invalid or unsupported files produce a visible warning and remain unchanged until an explicit UI preference change. Saves replace the file atomically; failures show a regular-weight red error and leave the confirmed session choice active while preserving the last good file. Enter retries a failed save, including on the already applied value. Enter on an active launch override saves that field and clears its override; unrelated overrides stay transient. Reconfirming an ordinary applied value does not write. The pinned status reports Saved or Save failed; each field is confirmed separately rather than staged in a multi-field dialog.

## Usage styles

Usage contains three rows with the same labeled 42% preview: Strip, Ticks and Segments. Their painters are shared with /usage, so previews follow the applied theme, color policy and glyph setting. Up/Down or a click highlights a row; Enter applies it. The [x] marker stays on the applied style until confirmation. Left from any Usage row returns to categories. The old usage-bars scenario now opens this category; the separate gallery and Usage style shortcuts are retired.

## Statistics chart styles

Statistics sits after Usage and previews Stacked, Lines and Trends through the actual Stats painters on labeled demo data. Navigation/clicks highlight; Enter applies and saves stats_chart_style. Left returns to categories. The default is Stacked. Switching charts in /stats is a sheet-local preview and never writes preferences; reopening uses this saved choice. Existing configuration files without the new field keep their prior settings and default to Stacked. See [Stats](stats.md).
