//! Design Tokens (descriptor: specs/descriptors/uimodel_design_tokens.json).

use std::error::Error;

use polaris_ui_model::tokens::{
    PLACEHOLDER_TEXT, ROLES, Rgb, SELECTION_TINTS, css_variables, role, selection_tint,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

/// The PLAN token table, transcribed independently of the implementation: (role, hex).
const PLAN_TABLE: &[(&str, &str)] = &[
    ("canvas", "#000000"),
    ("accent", "#76B900"),
    ("muted", "#858585"),
    ("muted-web", "#969C94"),
    ("mode-auto", "#F5C2E7"),
    ("mode-plan", "#F9E2AF"),
    ("plan-hover", "#FFFDE6"),
    ("tool-title", "#F9E2AF"),
    ("mention", "#F9E2AF"),
    ("command-focus", "#B6E35B"),
    ("command-hover", "#DBF5B0"),
    ("command-web", "#94CD40"),
    ("directive", "#89B4FA"),
    ("directive-hover", "#B4D2FF"),
    ("goal-line", "#89B4FA"),
    ("error", "#AC4242"),
    ("warning", "#E5B454"),
    ("prompt-surface", "#373737"),
    ("to-latest-bg", "#262626"),
    ("to-latest-hover", "#373737"),
    ("placeholder", "#525252"),
    ("logo-star", "#24D046"),
    ("logo-trace", "#76B900"),
    ("logo-mint", "#96D6AC"),
    ("logo-frame-light", "#B8B8B8"),
    ("logo-frame-dark", "#6B6B6B"),
    ("logo-package", "#1C1C1C"),
    ("surface", "#141414"),
    ("raised", "#1C1C1C"),
    ("text", "#E8E8E8"),
    ("green-light", "#C0E58C"),
    ("line", "#FFFFFF12"),
];

#[test]
fn every_plan_role_present_with_exact_hex() -> TestResult {
    for (name, hex) in PLAN_TABLE {
        let found = role(name).ok_or_else(|| format!("missing role {name}"))?;
        assert_eq!(found.hex, *hex, "role {name}");
    }
    assert_eq!(
        ROLES.len(),
        PLAN_TABLE.len(),
        "no role beyond the PLAN table"
    );
    Ok(())
}

#[test]
fn role_names_are_unique_and_hex_well_formed() -> TestResult {
    let mut seen = std::collections::BTreeSet::new();
    for r in ROLES {
        assert!(seen.insert(r.name), "duplicate role {}", r.name);
        assert!(
            r.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "role name {} is not kebab-case",
            r.name
        );
        let digits = r.hex.strip_prefix('#').ok_or("hex without #")?;
        assert!(
            digits.len() == 6 || digits.len() == 8,
            "hex length of {}",
            r.name
        );
        assert!(
            digits
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())
        );
        assert!(!r.source.is_empty(), "role {} has no source", r.name);
    }
    Ok(())
}

#[test]
fn error_role_has_ansi_red_fallback() -> TestResult {
    let error = role("error").ok_or("error role")?;
    assert_eq!(error.ansi, Some("Red"));
    let with_fallback: Vec<_> = ROLES
        .iter()
        .filter(|r| r.ansi.is_some())
        .map(|r| r.name)
        .collect();
    assert_eq!(
        with_fallback,
        ["error"],
        "only the error role states an ANSI fallback"
    );
    Ok(())
}

#[test]
fn css_variables_contain_every_role() {
    let css = css_variables();
    for (name, hex) in PLAN_TABLE {
        let line = format!("  --p-{name}: {hex};\n");
        assert!(css.contains(&line), "missing {line:?}");
    }
    assert!(css.contains("  --p-radius: 14px;\n"));
    assert!(css.contains("  --p-speed: 160ms;\n"));
    assert!(css.contains("  --p-font-display: \"Space Grotesk\";\n"));
    assert!(css.contains("  --p-font-body: \"Inter\";\n"));
    assert!(css.contains("  --p-font-mono: \"JetBrains Mono\";\n"));
}

#[test]
fn css_variables_block_shape() {
    let css = css_variables();
    assert!(css.starts_with(":root {\n"));
    assert!(css.ends_with("}\n"));
    assert_eq!(css.matches('{').count(), 1);
    assert_eq!(css.matches('}').count(), 1);
}

#[test]
fn selection_tints_match_blend_formula() -> TestResult {
    let expected = [
        ("text", "canvas", "#464646"),
        ("mode-plan", "canvas", "#4B4435"),
        ("accent", "canvas", "#233800"),
        ("directive", "canvas", "#29364B"),
        ("text", "prompt-surface", "#6C6C6C"),
    ];
    assert_eq!(SELECTION_TINTS.len(), expected.len());
    for (source, background, tint) in expected {
        let s = Rgb::from_hex(role(source).ok_or("source role")?.hex).ok_or("source hex")?;
        let b = Rgb::from_hex(role(background).ok_or("background role")?.hex).ok_or("bg hex")?;
        assert_eq!(
            selection_tint(s, b).to_hex(),
            tint,
            "{source} over {background}"
        );
        assert!(
            SELECTION_TINTS
                .iter()
                .any(|t| t.source == source && t.background == background && t.tint == tint),
            "table row {source}/{background}"
        );
    }
    Ok(())
}

#[test]
fn rgb_parsing_rejects_malformed_hex() {
    assert_eq!(Rgb::from_hex("#76B900"), Some(Rgb(0x76, 0xB9, 0x00)));
    assert_eq!(Rgb::from_hex("#76b900"), Some(Rgb(0x76, 0xB9, 0x00)));
    assert_eq!(Rgb::from_hex("76B900"), None);
    assert_eq!(Rgb::from_hex("#76B90"), None);
    assert_eq!(Rgb::from_hex("#76B9000"), None);
    assert_eq!(Rgb::from_hex("#GGGGGG"), None);
    assert_eq!(Rgb::from_hex("#é6B900"), None);
    assert_eq!(Rgb::from_hex(""), None);
}

#[test]
fn placeholder_text_exact() {
    assert_eq!(PLACEHOLDER_TEXT, "What's on your mind for Polaris?");
}

proptest! {
    #[test]
    fn prop_hex_round_trip(r in any::<u8>(), g in any::<u8>(), b in any::<u8>()) {
        let rgb = Rgb(r, g, b);
        prop_assert_eq!(Rgb::from_hex(&rgb.to_hex()), Some(rgb));
    }
}
