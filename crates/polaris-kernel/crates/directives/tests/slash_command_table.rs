//! Slash Command Table (descriptor: specs/descriptors/directives_slash_command_table.json).

use std::error::Error;

use polaris_kernel_directives::slash::{BrowserCmd, ResearchCmd, SlashCommand};
use polaris_kernel_directives::{DirectiveError, ParseLimits, parse};
use polaris_sdk::ids::{SessionId, ShortRef};

type TestResult = Result<(), Box<dyn Error>>;

fn command(input: &str) -> Result<Option<SlashCommand>, DirectiveError> {
    Ok(parse(input, &ParseLimits::default())?.command)
}

const ID: &str = "01920000-0000-7000-8000-0000000000a1";

#[test]
fn exit_aliases_quit() -> TestResult {
    assert_eq!(command("/quit")?, Some(SlashCommand::Quit));
    assert_eq!(command("  /exit  ")?, Some(SlashCommand::Quit));
    Ok(())
}

#[test]
fn research_subwords_are_reserved() -> TestResult {
    assert_eq!(
        command("/research status")?,
        Some(SlashCommand::Research(ResearchCmd::Status))
    );
    assert_eq!(
        command("/research report")?,
        Some(SlashCommand::Research(ResearchCmd::Report))
    );
    assert_eq!(
        command("/research resume")?,
        Some(SlashCommand::Research(ResearchCmd::Resume))
    );
    assert_eq!(
        command("/research find flaky tests")?,
        Some(SlashCommand::Research(ResearchCmd::Start(
            "find flaky tests".into()
        )))
    );
    assert!(matches!(
        command("/research"),
        Err(DirectiveError::BadArg { .. })
    ));
    Ok(())
}

#[test]
fn quoted_goal_equal_to_status_starts_research() -> TestResult {
    assert_eq!(
        command("/research \"status\"")?,
        Some(SlashCommand::Research(ResearchCmd::Start("status".into())))
    );
    Ok(())
}

#[test]
fn session_resume_requires_id() -> TestResult {
    let id: SessionId = ID.parse()?;
    assert_eq!(
        command(&format!("/session resume {ID}"))?,
        Some(SlashCommand::SessionResume(id))
    );
    assert_eq!(
        command(&format!("/session replay {ID} 5"))?,
        Some(SlashCommand::SessionReplay { id, step: Some(5) })
    );
    assert_eq!(
        command(&format!("/session replay {ID}"))?,
        Some(SlashCommand::SessionReplay { id, step: None })
    );
    assert!(matches!(
        command("/session resume"),
        Err(DirectiveError::BadArg { .. })
    ));
    assert!(matches!(
        command("/session resume not-a-uuid"),
        Err(DirectiveError::BadArg { .. })
    ));
    assert!(matches!(
        command("/session"),
        Err(DirectiveError::BadArg { .. })
    ));
    Ok(())
}

#[test]
fn unknown_command_is_an_error() {
    assert_eq!(
        command("/frobnicate"),
        Err(DirectiveError::UnknownCommand("frobnicate".into()))
    );
    assert_eq!(
        command("/frobnicate now"),
        Err(DirectiveError::UnknownCommand("frobnicate".into()))
    );
}

#[test]
fn path_is_not_a_command() -> TestResult {
    let parsed = parse("/usr/bin is broken", &ParseLimits::default())?;
    assert!(parsed.command.is_none());
    assert_eq!(parsed.prose, "/usr/bin is broken");
    assert!(
        parse("text then /quit", &ParseLimits::default())?
            .command
            .is_none()
    );
    Ok(())
}

#[test]
fn catalog_commands_parse() -> TestResult {
    let reference: ShortRef = "0123456789ab".parse()?;
    let cases: Vec<(&str, SlashCommand)> = vec![
        ("/model", SlashCommand::Model(None)),
        (
            "/model nvidia/Nemotron-3_5-Lightning",
            SlashCommand::Model(Some("nvidia/Nemotron-3_5-Lightning".into())),
        ),
        ("/usage", SlashCommand::Usage),
        ("/stats", SlashCommand::Stats),
        ("/status", SlashCommand::Status),
        ("/plan", SlashCommand::Plan),
        ("/goal", SlashCommand::Goal(None)),
        (
            "/goal fix the build",
            SlashCommand::Goal(Some("fix the build".into())),
        ),
        ("/diff", SlashCommand::Diff),
        ("/rollback", SlashCommand::Rollback(None)),
        ("/rollback 3", SlashCommand::Rollback(Some(3))),
        ("/undo", SlashCommand::Undo),
        ("/sessions", SlashCommand::Sessions),
        ("/settings", SlashCommand::Settings),
        ("/help", SlashCommand::Help(None)),
        (
            "/help directives",
            SlashCommand::Help(Some("directives".into())),
        ),
        ("/clear", SlashCommand::Clear),
        ("/providers", SlashCommand::Providers),
        ("/onboarding", SlashCommand::Onboarding),
        ("/nv-profile", SlashCommand::NvProfile),
        ("/nv-verify", SlashCommand::NvVerify),
        ("/nebius-status", SlashCommand::NebiusStatus),
        ("/contree-branches", SlashCommand::ContreeBranches),
        ("/datalab-sync", SlashCommand::DatalabSync),
        ("/datalab-finetune", SlashCommand::DatalabFinetune),
        ("/concurrency-stats", SlashCommand::ConcurrencyStats),
        ("/cache stats", SlashCommand::CacheStats),
        ("/cache purge", SlashCommand::CachePurge),
        ("/quota", SlashCommand::Quota),
        ("/context-map", SlashCommand::ContextMap),
        (
            "/context-expand §ref:0123456789ab",
            SlashCommand::ContextExpand(reference),
        ),
        ("/parallel-dag", SlashCommand::ParallelDag),
        ("/screenshot", SlashCommand::Screenshot),
        (
            "/browser goto https://example.com/a?b=1",
            SlashCommand::Browser(BrowserCmd::Goto("https://example.com/a?b=1".into())),
        ),
        ("/browser tree", SlashCommand::Browser(BrowserCmd::Tree)),
        ("/ui-diff", SlashCommand::UiDiff),
        ("/hack", SlashCommand::Hack),
        ("/redteam", SlashCommand::Redteam),
        (
            "/search-local error handling",
            SlashCommand::SearchLocal("error handling".into()),
        ),
        ("/ci", SlashCommand::Ci),
        ("/ci-run", SlashCommand::CiRun),
        ("/nix-flake", SlashCommand::NixFlake),
        ("/proof-of-done", SlashCommand::ProofOfDone),
    ];
    for (input, want) in cases {
        assert_eq!(command(input)?, Some(want), "{input}");
    }
    for bad in [
        "/quit now",
        "/cache",
        "/cache flush",
        "/rollback x",
        "/context-expand zz",
        "/browser",
        "/search-local",
    ] {
        assert!(
            matches!(command(bad), Err(DirectiveError::BadArg { .. })),
            "{bad} must be BadArg"
        );
    }
    Ok(())
}

/// Names of the top-level `match name {` arms (8-space indent) of `fn_marker`'s match in `file`.
fn parser_arm_names(file: &str, match_marker: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join(file);
    let source = std::fs::read_to_string(path)?;
    let start = source.find(match_marker).ok_or("match marker not found")?;
    let body = source.get(start..).ok_or("bad offset")?;
    let mut names = Vec::new();
    for line in body.lines().skip(1) {
        if line.starts_with("        other =>")
            || line.starts_with("        _ =>")
            || line == "    }"
            || line == "    };"
        {
            break;
        }
        let Some(arm) = line.strip_prefix("        \"") else {
            continue;
        };
        let Some((pattern, _)) = arm.split_once("=>") else {
            continue;
        };
        names.extend(
            pattern
                .split('|')
                .map(|p| p.trim().trim_matches('"').to_owned()),
        );
    }
    names.sort();
    Ok(names)
}

#[test]
fn command_name_table_matches_parser_arms() -> TestResult {
    let mut table: Vec<String> = polaris_kernel_directives::names::SLASH_COMMANDS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    table.sort();
    assert_eq!(table, parser_arm_names("slash.rs", "    match name {")?);
    Ok(())
}

#[test]
fn every_listed_command_is_recognised() {
    for name in polaris_kernel_directives::names::SLASH_COMMANDS {
        let result = command(&format!("/{name}"));
        assert!(
            !matches!(result, Err(DirectiveError::UnknownCommand(_)))
                && !matches!(result, Ok(None)),
            "/{name} is not recognised: {result:?}"
        );
    }
}

#[test]
fn directive_name_table_matches_parser_arms() -> TestResult {
    let mut table: Vec<String> = polaris_kernel_directives::names::DIRECTIVES
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    table.sort();
    assert_eq!(
        table,
        parser_arm_names("directive.rs", "let d = match name {")?
    );
    Ok(())
}

#[test]
fn every_listed_directive_is_typed() {
    use polaris_kernel_directives::directive::SessionDirective;
    for name in polaris_kernel_directives::names::DIRECTIVES {
        // Bare `$name` either types (toggle-style) or fails with an argument error; never Custom.
        if let Ok(parsed) = parse(&format!("${name}"), &ParseLimits::default()) {
            for d in &parsed.directives {
                assert!(
                    !matches!(d.value, SessionDirective::Custom { .. }),
                    "${name} parsed as Custom"
                );
            }
        }
    }
}
