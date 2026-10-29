//! Typed UI command addition; no execution permission is implied.

use std::error::Error;

use polaris_kernel_directives::slash::{PanelTarget, SlashCommand};
use polaris_kernel_directives::{ParseLimits, parse};

fn command(input: &str) -> Result<Option<SlashCommand>, polaris_kernel_directives::DirectiveError> {
    Ok(parse(input, &ParseLimits::default())?.command)
}

#[test]
fn ui_navigation_commands_are_recognized() -> Result<(), Box<dyn Error>> {
    let cases = [
        ("/panel", SlashCommand::Panel(None)),
        (
            "/panel tasks",
            SlashCommand::Panel(Some(PanelTarget::Tasks)),
        ),
        (
            "/panel agents",
            SlashCommand::Panel(Some(PanelTarget::Agents)),
        ),
        (
            "/panel questions",
            SlashCommand::Panel(Some(PanelTarget::Questions)),
        ),
        (
            "/panel review",
            SlashCommand::Panel(Some(PanelTarget::Review)),
        ),
        (
            "/panel proof",
            SlashCommand::Panel(Some(PanelTarget::Proof)),
        ),
        (
            "/panel context",
            SlashCommand::Panel(Some(PanelTarget::Context)),
        ),
        (
            "/panel system",
            SlashCommand::Panel(Some(PanelTarget::System)),
        ),
        ("/panel off", SlashCommand::Panel(Some(PanelTarget::Off))),
        ("/agents", SlashCommand::Agents),
        ("/questions", SlashCommand::Questions),
    ];
    for (input, expected) in cases {
        assert_eq!(command(input)?, Some(expected), "{input}");
    }
    Ok(())
}

#[test]
fn panel_arguments_are_bounded_and_validated() -> Result<(), Box<dyn Error>> {
    for input in [
        "/panel unknown",
        "/panel tasks more",
        r#"/panel "tasks""#,
        "/agents extra",
        "/questions extra",
    ] {
        assert!(command(input).is_err(), "invalid command accepted: {input}");
    }
    Ok(())
}
