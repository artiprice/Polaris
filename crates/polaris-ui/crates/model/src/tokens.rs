//! Design tokens: every colour is a *role* (a name plus a hex value), and frontends choose
//! truecolor, ANSI or inherited terminal colours for it.
//!
//! The values come from the PLAN token table (read from the demo's `ui-design.md`,
//! `color-policy.md`, `hybrid-palette.json` and `webui/styles.css`). Their rendering was never
//! re-checked visually. The web UI generates its `:root` block from [`css_variables`], so the TUI
//! and the web share one source.

use core::fmt::Write as _;

/// One colour role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColorRole {
    /// Kebab-case role name; also the CSS variable suffix (`--p-<name>`).
    pub name: &'static str,
    /// `#RRGGBB`, or `#RRGGBBAA` for translucent roles.
    pub hex: &'static str,
    /// ANSI fallback colour name where the source states one.
    pub ansi: Option<&'static str>,
    /// Where the value comes from.
    pub source: &'static str,
}

const fn role_of(name: &'static str, hex: &'static str, source: &'static str) -> ColorRole {
    ColorRole {
        name,
        hex,
        ansi: None,
        source,
    }
}

/// Every colour role of the design, in the order of the PLAN table.
pub const ROLES: &[ColorRole] = &[
    role_of("canvas", "#000000", "ui-design.md, palette.json"),
    role_of(
        "accent",
        "#76B900",
        "ui-design.md (NVIDIA-inspired, not an official token)",
    ),
    role_of("muted", "#858585", "ui-design.md (web --p-dim)"),
    role_of("muted-web", "#969C94", "styles.css (web --p-muted)"),
    role_of("mode-auto", "#F5C2E7", "ui-design.md (auto mode, pink)"),
    role_of("mode-plan", "#F9E2AF", "ui-design.md (plan mode)"),
    role_of("plan-hover", "#FFFDE6", "ui-design.md (plan hover)"),
    role_of("tool-title", "#F9E2AF", "ui-design.md (tool titles)"),
    role_of("mention", "#F9E2AF", "composer.md (@file mentions)"),
    role_of("command-focus", "#B6E35B", "composer.md (command focus)"),
    role_of("command-hover", "#DBF5B0", "composer.md (command hover)"),
    role_of("command-web", "#94CD40", "styles.css (token-command)"),
    role_of(
        "directive",
        "#89B4FA",
        "prompt-settings.md (dollar settings)",
    ),
    role_of(
        "directive-hover",
        "#B4D2FF",
        "prompt-settings.md (dollar hover)",
    ),
    role_of("goal-line", "#89B4FA", "prompt-settings.md (goal line)"),
    ColorRole {
        name: "error",
        hex: "#AC4242",
        ansi: Some("Red"),
        source: "ui-design.md (error title/body, regular weight)",
    },
    role_of(
        "warning",
        "#E5B454",
        "styles.css (web --p-warning; TUI value not stated)",
    ),
    role_of(
        "prompt-surface",
        "#141414",
        "ui-design.md (submitted prompt; live composer uses canvas)",
    ),
    role_of("to-latest-bg", "#262626", "ui-design.md (To latest)"),
    role_of(
        "to-latest-hover",
        "#373737",
        "ui-design.md (To latest hover)",
    ),
    role_of(
        "placeholder",
        "#525252",
        "prompt-settings.md (empty-editor placeholder)",
    ),
    role_of("logo-star", "#C0E58C", "UI_PLAN current-demo North Star"),
    role_of("logo-trace", "#76B900", "UI_PLAN current-demo rays"),
    role_of("logo-tips", "#6B6B6B", "UI_PLAN current-demo gray tips"),
    role_of("surface", "#141414", "styles.css (web surface)"),
    role_of("raised", "#1C1C1C", "styles.css (web raised surface)"),
    role_of("text", "#E8E8E8", "styles.css (web text)"),
    role_of("green-light", "#C0E58C", "styles.css"),
    role_of("line", "#FFFFFF12", "styles.css (hairline)"),
];

/// Looks a role up by name.
#[must_use]
pub fn role(name: &str) -> Option<&'static ColorRole> {
    ROLES.iter().find(|r| r.name == name)
}

/// Text shown in the empty editor (drawn in the `placeholder` role, outside the source text).
pub const PLACEHOLDER_TEXT: &str = "What's on your mind for Polaris?";

/// Chart series colours by name. The PLAN gives the names only; `green` is the `accent` role and
/// `gold` is `mode-plan`, while `cyan` has no stated hex yet.
pub const CHART_SERIES: [&str; 3] = ["green", "cyan", "gold"];

/// Selection colour in a terminal that only has the 256-colour palette (neutral grey).
pub const SELECTION_ANSI_INDEX: u8 = 237;

/// Strength of the selection tint: a 30% share of the source colour over the background. With no
/// colour at all, a selection is shown bold.
pub const SELECTION_PERCENT: u8 = 30;

/// Corner radius of web surfaces in pixels.
pub const RADIUS_PX: u32 = 14;

/// Web transition speed in milliseconds.
pub const SPEED_MS: u32 = 160;

/// Web font stacks: (CSS variable suffix, family). OFL-licensed files are bundled by the web UI.
pub const FONTS: &[(&str, &str)] = &[
    ("display", "Space Grotesk"),
    ("body", "Inter"),
    ("mono", "JetBrains Mono"),
];

/// An opaque colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Parses `#RRGGBB` (digits in either case). Anything else, including `#RRGGBBAA`, is `None`.
    #[must_use]
    pub fn from_hex(text: &str) -> Option<Self> {
        let digits = text.strip_prefix('#')?;
        if digits.len() != 6 || !digits.is_ascii() {
            return None;
        }
        let channel = |from: usize| {
            digits
                .get(from..from.saturating_add(2))
                .and_then(|pair| u8::from_str_radix(pair, 16).ok())
        };
        Some(Self(channel(0)?, channel(2)?, channel(4)?))
    }

    /// Formats as `#RRGGBB` with upper-case digits.
    #[must_use]
    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.0, self.1, self.2)
    }

    /// `percent`% of `self` over `background`, rounded to nearest per channel.
    #[must_use]
    pub fn blend_over(self, background: Self, percent: u8) -> Self {
        let share = u32::from(percent.min(100));
        let mix = |source: u8, behind: u8| {
            let numerator = u32::from(source)
                .saturating_mul(share)
                .saturating_add(u32::from(behind).saturating_mul(100u32.saturating_sub(share)))
                .saturating_add(50);
            let value = match numerator.checked_div(100) {
                Some(value) => value,
                None => 0,
            };
            match u8::try_from(value) {
                Ok(channel) => channel,
                Err(_) => u8::MAX,
            }
        };
        Self(
            mix(self.0, background.0),
            mix(self.1, background.1),
            mix(self.2, background.2),
        )
    }
}

/// The selection colour for text of colour `source` drawn on `background`: a
/// [`SELECTION_PERCENT`]% tint of the source over the background.
#[must_use]
pub fn selection_tint(source: Rgb, background: Rgb) -> Rgb {
    source.blend_over(background, SELECTION_PERCENT)
}

/// One row of the PLAN selection table (roles refer to [`ROLES`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionTint {
    /// Role of the text colour being selected ("white" in the PLAN is the `text` role).
    pub source: &'static str,
    /// Role of the background underneath.
    pub background: &'static str,
    /// The resulting tint, as listed in the PLAN.
    pub tint: &'static str,
}

/// The selection tints listed in the PLAN table; [`selection_tint`] reproduces each of them.
pub const SELECTION_TINTS: &[SelectionTint] = &[
    SelectionTint {
        source: "text",
        background: "canvas",
        tint: "#464646",
    },
    SelectionTint {
        source: "mode-plan",
        background: "canvas",
        tint: "#4B4435",
    },
    SelectionTint {
        source: "accent",
        background: "canvas",
        tint: "#233800",
    },
    SelectionTint {
        source: "directive",
        background: "canvas",
        tint: "#29364B",
    },
    SelectionTint {
        source: "text",
        background: "prompt-surface",
        tint: "#545454",
    },
];

/// A `:root { --p-<role>: <hex>; ... }` block for the web UI, generated from [`ROLES`],
/// [`RADIUS_PX`], [`SPEED_MS`] and [`FONTS`].
#[must_use]
pub fn css_variables() -> String {
    let mut css = String::from(":root {\n");
    for r in ROLES {
        // Writing to a String cannot fail.
        let _ = writeln!(css, "  --p-{}: {};", r.name, r.hex);
    }
    let _ = writeln!(css, "  --p-radius: {RADIUS_PX}px;");
    let _ = writeln!(css, "  --p-speed: {SPEED_MS}ms;");
    for (suffix, family) in FONTS {
        let _ = writeln!(css, "  --p-font-{suffix}: \"{family}\";");
    }
    css.push_str("}\n");
    css
}
