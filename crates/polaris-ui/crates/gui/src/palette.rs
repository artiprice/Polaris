//! Desktop-only colour roles of the welcome screen.
//!
//! The shared roles live in [`polaris_ui_model::tokens::ROLES`]. The welcome page of the design
//! demo (`dist/demo-design-preview-slop/webui/styles.css`) also uses component colours that no
//! other frontend needs yet; they are listed here once. `build.rs` includes this file and writes
//! every role, shared and local, into the generated Slint `Tokens` global, so `.slint` files never
//! carry a colour literal of their own.

use polaris_ui_model::tokens::ColorRole;

const fn local(name: &'static str, hex: &'static str, source: &'static str) -> ColorRole {
    ColorRole {
        name,
        hex,
        ansi: None,
        source,
    }
}

/// Welcome-screen roles that are not part of the shared token table.
pub const WELCOME_ROLES: &[ColorRole] = &[
    local("heading", "#D8DBD5", "styles.css (.welcome h1)"),
    local(
        "heading-backing",
        "#000000BB",
        "styles.css (.welcome h1:before)",
    ),
    local("line-strong", "#FFFFFF22", "styles.css (--p-line-strong)"),
    local("nav-hover", "#FFFFFF0C", "styles.css (.nav-link:hover)"),
    local("tooltip-bg", "#202020", "styles.css (.nav-tooltip)"),
    local(
        "topbar-hover",
        "#FFFFFF08",
        "styles.css (.context-toggle summary:hover)",
    ),
    local("context-bg", "#11140F", "styles.css (.context-aside)"),
    local("context-file", "#CCD2C6", "styles.css (.context-file)"),
    local("composer-border", "#FFFFFF29", "styles.css (.composer)"),
    local(
        "composer-border-focus",
        "#FFFFFF55",
        "styles.css (.composer:focus-within)",
    ),
    local("attach-fg", "#C5CEC0", "styles.css (.attach-button)"),
    local(
        "placeholder-web",
        "#A1A69D",
        "styles.css (textarea::placeholder)",
    ),
    local("pill-fg", "#B6BDB0", "styles.css (.composer-pill)"),
    local(
        "pill-hover",
        "#FFFFFF07",
        "styles.css (.composer-pill:hover)",
    ),
    local(
        "pill-open-fg",
        "#E0E5DA",
        "styles.css (.composer-popover[open] .composer-pill)",
    ),
    local("send-bg", "#E0E6D9", "styles.css (.send-button)"),
    local("send-fg", "#20241C", "styles.css (.send-button)"),
    local("popover-border", "#FFFFFF26", "styles.css (.popover-body)"),
    local(
        "popover-shadow",
        "#00000099",
        "styles.css (.popover-body box-shadow)",
    ),
    local("row-fg", "#D7DBD2", "styles.css (.menu-model-row)"),
    local(
        "row-checked-bg",
        "#111111",
        "styles.css (.menu-model-row:has(:checked))",
    ),
    local(
        "row-checked-fg",
        "#F0F2ED",
        "styles.css (.menu-model-row:has(:checked))",
    ),
    local(
        "row-hover-bg",
        "#1B1B1B",
        "styles.css (.menu-model-row:hover)",
    ),
    local(
        "row-hover-fg",
        "#FFFFFF",
        "styles.css (.menu-model-row:hover)",
    ),
    local("check", "#A8CE7B", "styles.css (.selection-check)"),
    local("segment-border", "#FFFFFF17", "styles.css (.mode-options)"),
    local(
        "segment-selected-bg",
        "#191919",
        "styles.css (.mode-options :checked)",
    ),
    local(
        "segment-selected-ring",
        "#FFFFFF16",
        "styles.css (.mode-options :checked)",
    ),
    local(
        "segment-hover-bg",
        "#242424",
        "styles.css (.mode-options label:hover)",
    ),
    local("footer-link", "#C0C9B6", "styles.css (.menu-footer a)"),
    local(
        "panel-shadow",
        "#000000BB",
        "styles.css (.preview-description box-shadow)",
    ),
    local(
        "scrim",
        "#00000000",
        "desktop: transparent end stop of gradients",
    ),
    local(
        "selection",
        "#464646",
        "tokens::SELECTION_TINTS (text over canvas)",
    ),
    local(
        "tray-bg",
        "#0B0D0A",
        "desktop composer v2: tray behind the input box",
    ),
    local(
        "mode-plan-line",
        "#F9E2AF55",
        "desktop composer v2: plan chip outline (mode-plan at 33%)",
    ),
    local(
        "mode-auto-line",
        "#F5C2E755",
        "desktop composer v2: auto chip outline (mode-auto at 33%)",
    ),
    local(
        "send-idle-bg",
        "#1C1F1A",
        "desktop composer v2: send button without a draft",
    ),
    local(
        "session-sidebar-bg",
        "#090A08",
        "desktop session view: sidebar surface",
    ),
    local(
        "session-row-active",
        "#141812",
        "desktop session view: selected session row",
    ),
    local(
        "bubble-bg",
        "#141613",
        "desktop session view: user message bubble",
    ),
    local("field-bg", "#0F110E", "desktop session view: search field"),
];
