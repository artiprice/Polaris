//! Directive Persistence (descriptor: specs/descriptors/directives_directive_persistence.json).

use std::error::Error;

use polaris_kernel_directives::directive::{Arg, SandboxMode, SessionDirective, Toggle};
use polaris_kernel_directives::render::render;
use polaris_kernel_directives::state::DirectiveState;
use polaris_kernel_directives::{ParseLimits, parse};

type TestResult = Result<(), Box<dyn Error>>;

#[test]
fn last_occurrence_wins() {
    let mut state = DirectiveState::new();
    assert_eq!(state.apply(SessionDirective::WebSearch(Toggle::On)), None);
    assert_eq!(
        state.apply(SessionDirective::WebSearch(Toggle::Off)),
        Some(SessionDirective::WebSearch(Toggle::On))
    );
    assert_eq!(
        state.get("websearch"),
        Some(&SessionDirective::WebSearch(Toggle::Off))
    );
    assert_eq!(state.len(), 1);
}

#[test]
fn fold_is_deterministic_for_same_event_order() {
    let events = [
        SessionDirective::Sandbox(SandboxMode::Openshell),
        SessionDirective::RateLimit {
            rpm: Some(60),
            tpm: None,
        },
        SessionDirective::Sandbox(SandboxMode::Contree),
    ];
    let fold = |order: &[SessionDirective]| {
        let mut s = DirectiveState::new();
        for d in order {
            s.apply(d.clone());
        }
        s
    };
    assert_eq!(fold(&events), fold(&events));
    let reversed: Vec<_> = events.iter().rev().cloned().collect();
    assert_ne!(fold(&events), fold(&reversed));
    assert_eq!(
        fold(&events).get("sandbox"),
        Some(&SessionDirective::Sandbox(SandboxMode::Contree))
    );
}

#[test]
fn new_session_starts_from_config_defaults() {
    let state = DirectiveState::with_defaults([
        SessionDirective::Sandbox(SandboxMode::Auto),
        SessionDirective::RateLimit {
            rpm: Some(600),
            tpm: Some(200_000),
        },
    ]);
    assert_eq!(state.len(), 2);
    assert_eq!(
        state.get("sandbox"),
        Some(&SessionDirective::Sandbox(SandboxMode::Auto))
    );
    assert!(state.get("websearch").is_none());
}

#[test]
fn custom_directive_survives_resume() -> TestResult {
    let mut live = DirectiveState::new();
    let parsed = parse(
        r#"$websearch $sandbox(remote) $my-tool(level=3, "two words", f(x)) $subagent-models(nano(1-2)) go"#,
        &ParseLimits::default(),
    )?;
    for d in parsed.directives {
        live.apply(d.value);
    }
    assert!(
        matches!(live.get("my-tool"), Some(SessionDirective::Custom { args, .. }) if args.len() == 3)
    );
    assert!(matches!(
        live.get("my-tool"),
        Some(SessionDirective::Custom { args, .. }) if args.first() == Some(&Arg::Kv { key: "level".into(), value: Box::new(Arg::Atom("3".into())) })
    ));

    // Persisted form (D11): the canonical text of each applied directive, replayed in order.
    let persisted: Vec<String> = live.iter().map(render).collect();
    let mut resumed = DirectiveState::new();
    for text in &persisted {
        for d in parse(text, &ParseLimits::default())?.directives {
            resumed.apply(d.value);
        }
    }
    assert_eq!(resumed, live, "persisted: {persisted:?}");
    Ok(())
}
