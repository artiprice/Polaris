//! Slash commands. A command is recognised only when `/name` is the first non-whitespace text of the
//! whole input and the name is followed by whitespace or the end (`/usr/bin …` stays prose).

use polaris_sdk::ids::{SessionId, ShortRef};

use crate::args::quoted;
use crate::cursor::Cursor;
use crate::error::DirectiveError;

/// `/research` sub-command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ResearchCmd {
    /// Start a research loop with this goal.
    Start(String),
    /// Show progress.
    Status,
    /// Print the report.
    Report,
    /// Resume a paused loop.
    Resume,
}

/// `/browser` sub-command.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum BrowserCmd {
    /// Navigate to a URL.
    Goto(String),
    /// Print the accessibility tree.
    Tree,
}

/// A slash command (PLAN command catalog).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SlashCommand {
    /// `/model [id]`.
    Model(Option<String>),
    /// `/usage`.
    Usage,
    /// `/stats`.
    Stats,
    /// `/status`.
    Status,
    /// `/quit` (alias `/exit`).
    Quit,
    /// `/plan`.
    Plan,
    /// `/goal [text]`.
    Goal(Option<String>),
    /// `/diff`.
    Diff,
    /// `/rollback [step]`.
    Rollback(Option<u32>),
    /// `/undo`.
    Undo,
    /// `/sessions`.
    Sessions,
    /// `/session resume <id>`.
    SessionResume(SessionId),
    /// `/session replay <id> [step]`.
    SessionReplay {
        /// Session to replay.
        id: SessionId,
        /// Stop at this event step.
        step: Option<u64>,
    },
    /// `/settings`.
    Settings,
    /// `/help [topic]`.
    Help(Option<String>),
    /// `/clear`.
    Clear,
    /// `/providers`.
    Providers,
    /// `/onboarding`.
    Onboarding,
    /// `/nv-profile`.
    NvProfile,
    /// `/nv-verify`.
    NvVerify,
    /// `/nebius-status`.
    NebiusStatus,
    /// `/contree-branches`.
    ContreeBranches,
    /// `/datalab-sync`.
    DatalabSync,
    /// `/datalab-finetune`.
    DatalabFinetune,
    /// `/concurrency-stats`.
    ConcurrencyStats,
    /// `/cache stats`.
    CacheStats,
    /// `/cache purge`.
    CachePurge,
    /// `/quota`.
    Quota,
    /// `/research …`.
    Research(ResearchCmd),
    /// `/context-map`.
    ContextMap,
    /// `/context-expand <ref>`.
    ContextExpand(ShortRef),
    /// `/parallel-dag`.
    ParallelDag,
    /// `/screenshot`.
    Screenshot,
    /// `/browser goto <url>` | `/browser tree`.
    Browser(BrowserCmd),
    /// `/ui-diff`.
    UiDiff,
    /// `/hack`.
    Hack,
    /// `/redteam`.
    Redteam,
    /// `/search-local <query>`.
    SearchLocal(String),
    /// `/ci`.
    Ci,
    /// `/ci-run`.
    CiRun,
    /// `/nix-flake`.
    NixFlake,
    /// `/proof-of-done`.
    ProofOfDone,
}

/// One whitespace-separated command argument.
struct Word {
    text: String,
    quoted: bool,
}

/// Command arguments plus the raw remainder (for free-text commands).
struct CmdArgs<'a> {
    words: Vec<Word>,
    raw: &'a str,
}

impl CmdArgs<'_> {
    /// Free text: the content of a single quoted word, otherwise the trimmed raw remainder.
    fn free_text(&self) -> Option<String> {
        match self.words.as_slice() {
            [] => None,
            [only] if only.quoted => Some(only.text.clone()),
            _ => Some(self.raw.trim().to_owned()),
        }
    }

    fn word(&self, i: usize) -> Option<&Word> {
        self.words.get(i)
    }
}

fn split_args(rest: &str, base: usize) -> Result<CmdArgs<'_>, DirectiveError> {
    let mut cur = Cursor::new(rest, base);
    let mut words = Vec::new();
    loop {
        cur.skip_ws();
        match cur.peek() {
            None => break,
            Some('"' | '\'') => words.push(Word {
                text: quoted(&mut cur)?,
                quoted: true,
            }),
            Some(_) => {
                let text = cur
                    .take_while(|c| !(c.is_whitespace() || matches!(c, '"' | '\'')))
                    .to_owned();
                words.push(Word {
                    text,
                    quoted: false,
                });
            }
        }
    }
    Ok(CmdArgs { words, raw: rest })
}

/// Recognises `/name` at the start of `input`; returns the name, the argument text and its offset.
pub(crate) fn recognise(input: &str) -> Option<(&str, &str, usize)> {
    let mut cur = Cursor::new(input, 0);
    cur.skip_ws();
    if !cur.eat('/') {
        return None;
    }
    let name = cur.take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        return None;
    }
    match cur.peek() {
        None => Some((name, "", cur.offset())),
        Some(c) if c.is_whitespace() => Some((name, cur.rest(), cur.offset())),
        Some(_) => None,
    }
}

/// Parses the command `name` with its argument text.
pub(crate) fn parse_command(
    name: &str,
    rest: &str,
    at: usize,
) -> Result<SlashCommand, DirectiveError> {
    let args = split_args(rest, at)?;
    let bad = |reason| DirectiveError::BadArg {
        directive: format!("/{name}"),
        at,
        reason,
    };
    let no_args = |cmd: SlashCommand| {
        if args.words.is_empty() {
            Ok(cmd)
        } else {
            Err(bad("takes no arguments"))
        }
    };
    let single = || match args.words.as_slice() {
        [] => Ok(None),
        [only] => Ok(Some(only.text.clone())),
        _ => Err(bad("takes at most one argument")),
    };
    match name {
        "model" => single().map(SlashCommand::Model),
        "usage" => no_args(SlashCommand::Usage),
        "stats" => no_args(SlashCommand::Stats),
        "status" => no_args(SlashCommand::Status),
        "quit" | "exit" => no_args(SlashCommand::Quit),
        "plan" => no_args(SlashCommand::Plan),
        "goal" => Ok(SlashCommand::Goal(args.free_text())),
        "diff" => no_args(SlashCommand::Diff),
        "rollback" => match single()? {
            None => Ok(SlashCommand::Rollback(None)),
            Some(step) => step
                .parse()
                .map(|s| SlashCommand::Rollback(Some(s)))
                .map_err(|_| bad("step must be a number")),
        },
        "undo" => no_args(SlashCommand::Undo),
        "sessions" => no_args(SlashCommand::Sessions),
        "session" => {
            let id = |i: usize| -> Result<SessionId, DirectiveError> {
                args.word(i)
                    .ok_or_else(|| bad("missing session id"))?
                    .text
                    .parse()
                    .map_err(|_| bad("invalid session id"))
            };
            match args.word(0).map(|w| w.text.as_str()) {
                Some("resume") if args.words.len() == 2 => Ok(SlashCommand::SessionResume(id(1)?)),
                Some("replay") if (2..=3).contains(&args.words.len()) => {
                    let step = match args.word(2) {
                        None => None,
                        Some(w) => Some(w.text.parse().map_err(|_| bad("step must be a number"))?),
                    };
                    Ok(SlashCommand::SessionReplay { id: id(1)?, step })
                }
                Some("resume" | "replay") => Err(bad("expected a session id")),
                _ => Err(bad("expected resume <id> or replay <id> [step]")),
            }
        }
        "settings" => no_args(SlashCommand::Settings),
        "help" => single().map(SlashCommand::Help),
        "clear" => no_args(SlashCommand::Clear),
        "providers" => no_args(SlashCommand::Providers),
        "onboarding" => no_args(SlashCommand::Onboarding),
        "nv-profile" => no_args(SlashCommand::NvProfile),
        "nv-verify" => no_args(SlashCommand::NvVerify),
        "nebius-status" => no_args(SlashCommand::NebiusStatus),
        "contree-branches" => no_args(SlashCommand::ContreeBranches),
        "datalab-sync" => no_args(SlashCommand::DatalabSync),
        "datalab-finetune" => no_args(SlashCommand::DatalabFinetune),
        "concurrency-stats" => no_args(SlashCommand::ConcurrencyStats),
        "cache" => match args.words.as_slice() {
            [w] if !w.quoted && w.text == "stats" => Ok(SlashCommand::CacheStats),
            [w] if !w.quoted && w.text == "purge" => Ok(SlashCommand::CachePurge),
            _ => Err(bad("expected stats or purge")),
        },
        "quota" => no_args(SlashCommand::Quota),
        "research" => match args.words.as_slice() {
            [] => Err(bad("expected a goal or status|report|resume")),
            [w] if !w.quoted && w.text == "status" => {
                Ok(SlashCommand::Research(ResearchCmd::Status))
            }
            [w] if !w.quoted && w.text == "report" => {
                Ok(SlashCommand::Research(ResearchCmd::Report))
            }
            [w] if !w.quoted && w.text == "resume" => {
                Ok(SlashCommand::Research(ResearchCmd::Resume))
            }
            _ => args
                .free_text()
                .map(|g| SlashCommand::Research(ResearchCmd::Start(g)))
                .ok_or_else(|| bad("expected a goal")),
        },
        "context-map" => no_args(SlashCommand::ContextMap),
        "context-expand" => match single()? {
            Some(r) => r
                .parse::<ShortRef>()
                .map(SlashCommand::ContextExpand)
                .map_err(|_| bad("expected §ref:<12hex>")),
            None => Err(bad("expected §ref:<12hex>")),
        },
        "parallel-dag" => no_args(SlashCommand::ParallelDag),
        "screenshot" => no_args(SlashCommand::Screenshot),
        "browser" => match args.words.as_slice() {
            [w, url] if !w.quoted && w.text == "goto" => {
                Ok(SlashCommand::Browser(BrowserCmd::Goto(url.text.clone())))
            }
            [w] if !w.quoted && w.text == "tree" => Ok(SlashCommand::Browser(BrowserCmd::Tree)),
            _ => Err(bad("expected goto <url> or tree")),
        },
        "ui-diff" => no_args(SlashCommand::UiDiff),
        "hack" => no_args(SlashCommand::Hack),
        "redteam" => no_args(SlashCommand::Redteam),
        "search-local" => args
            .free_text()
            .map(SlashCommand::SearchLocal)
            .ok_or_else(|| bad("expected a query")),
        "ci" => no_args(SlashCommand::Ci),
        "ci-run" => no_args(SlashCommand::CiRun),
        "nix-flake" => no_args(SlashCommand::NixFlake),
        "proof-of-done" => no_args(SlashCommand::ProofOfDone),
        other => Err(DirectiveError::UnknownCommand(other.to_owned())),
    }
}
