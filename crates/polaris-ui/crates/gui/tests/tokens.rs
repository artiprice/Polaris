//! GUI Tokens (descriptor: specs/descriptors/ui_gui_tokens.json).

use std::collections::HashSet;
use std::error::Error;
use std::path::Path;

use polaris_ui_gui::palette::WELCOME_ROLES;
use polaris_ui_model::tokens::{FONTS, RADIUS_PX, ROLES, SPEED_MS};

type TestResult = Result<(), Box<dyn Error>>;

const GENERATED: &str = include_str!(concat!(env!("OUT_DIR"), "/slint-tokens/tokens.slint"));

#[test]
fn slint_tokens_match_model_roles() -> TestResult {
    for role in ROLES.iter().chain(WELCOME_ROLES.iter()) {
        let line = format!("out property <color> {}: {};", role.name, role.hex);
        assert!(
            GENERATED.contains(&line),
            "missing or wrong token line: {line}"
        );
    }
    let colour_lines = GENERATED.matches("out property <color>").count();
    assert_eq!(
        colour_lines,
        ROLES.len() + WELCOME_ROLES.len(),
        "unexpected colour properties"
    );
    Ok(())
}

#[test]
fn welcome_roles_do_not_shadow_shared_roles() -> TestResult {
    let shared: HashSet<&str> = ROLES.iter().map(|role| role.name).collect();
    let mut local = HashSet::new();
    for role in WELCOME_ROLES {
        assert!(
            !shared.contains(role.name),
            "{} shadows a shared role",
            role.name
        );
        assert!(local.insert(role.name), "{} is listed twice", role.name);
    }
    Ok(())
}

/// Finds `#` followed by 3..=8 hex digits that is not part of an identifier or a path command.
fn colour_literals(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for (at, _) in text.match_indices('#') {
        let Some(rest) = text.get(at + 1..) else {
            continue;
        };
        let digits: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
        let next = rest.chars().nth(digits.len());
        let boundary = next.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_');
        if (3..=8).contains(&digits.len()) && boundary {
            found.push(format!("#{digits}"));
        }
    }
    found
}

#[test]
fn every_color_in_slint_comes_from_tokens() -> TestResult {
    let ui = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut checked = 0_usize;
    for entry in std::fs::read_dir(&ui)? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "slint") {
            let text = std::fs::read_to_string(&path)?;
            let literals = colour_literals(&text);
            assert!(
                literals.is_empty(),
                "{} has colour literals {literals:?}",
                path.display()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 3,
        "expected the ui/*.slint files, found {checked}"
    );
    Ok(())
}

#[test]
fn radius_and_speed_match_model_constants() -> TestResult {
    let radius = format!("out property <length> radius: {RADIUS_PX}px;");
    let speed = format!("out property <duration> speed: {SPEED_MS}ms;");
    assert!(GENERATED.contains(&radius), "missing {radius}");
    assert!(GENERATED.contains(&speed), "missing {speed}");
    for (suffix, family) in FONTS {
        let line = format!("out property <string> font-{suffix}: \"{family}\";");
        assert!(GENERATED.contains(&line), "missing {line}");
    }
    Ok(())
}
