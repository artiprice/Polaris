//! Bounded shell tokenization followed by deterministic hazards and residual semantic review.
use core::fmt;
use polaris_sdk::registry::DecisionEngine;
use polaris_security_automata::{Category, ScanBuf};
use polaris_system_one_schemas::SchemaCatalog;
use crate::{DegradedReason, Finding, GateVerdict, Scanner, Verifier, VerifierError};

/// Shell grammar requested by an execution owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellDialect {
    /// POSIX shell quoting and operator subset.
    Posix,
    /// PowerShell needs an AST-capable executor; this boundary never treats it as POSIX.
    PowerShell,
    /// Windows cmd quoting and variable expansion need a cmd-specific executor.
    Cmd,
}

/// Trusted execution facts. An allow verdict is a safety result, not a sandbox capability.
pub struct CommandFacts<'a> {
    /// Exact command text to be executed, never a rewritten approximation.
    pub command: &'a str,
    /// Shell actually selected by the execution owner.
    pub dialect: ShellDialect,
    /// Workspace-relative working directory established by the execution owner.
    pub working_directory: &'a str,
    /// Whether the execution owner established a constrained sandbox for this command.
    pub sandbox_established: bool,
    /// Executable names allowed by the independent execution policy.
    pub permitted_executables: &'a [&'a str],
    /// Stated goal, masked before inference.
    pub intent: &'a str,
}

impl fmt::Debug for CommandFacts<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CommandFacts")
            .field("command_bytes", &self.command.len())
            .field("dialect", &self.dialect)
            .field("sandbox_established", &self.sandbox_established)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token { Word(String), Operator(String) }

struct ShellWords { tokens: Vec<Token>, opaque: bool }

/// Parse a deliberately constrained grammar. Expansions, substitutions, here-documents,
/// process substitutions and function definitions are opaque rather than guessed at.
fn tokenize(input: &str) -> Result<ShellWords, VerifierError> {
    const MAX_TOKENS: usize = 8192;
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    let mut escaped = false;
    let mut opaque = false;
    for character in input.chars() {
        if escaped {
            if character != '\n' { word.push(character); started = true; }
            escaped = false;
            continue;
        }
        match quote {
            Some('\'') => {
                if character == '\'' { quote = None; } else { word.push(character); }
                continue;
            },
            Some('"') => {
                match character {
                    '"' => quote = None,
                    '\\' => { escaped = true; opaque = true; },
                    '$' | '`' => { opaque = true; word.push(character); },
                    _ => word.push(character),
                }
                continue;
            },
            Some(_) => return Err(VerifierError::InvalidInput { reason: "unsupported shell quote state" }),
            None => {},
        }
        match character {
            '\\' => { escaped = true; started = true; },
            '\'' | '"' => { quote = Some(character); started = true; },
            '$' | '`' | '(' | ')' | '{' | '}' | '*' | '?' | '[' | ']' | '~' => {
                opaque = true;
                word.push(character);
                started = true;
            },
            ';' | '|' | '&' | '<' | '>' | '\n' => {
                if started {
                    tokens.push(Token::Word(core::mem::take(&mut word)));
                    started = false;
                }
                tokens.push(Token::Operator(character.to_string()));
                if matches!(character, '<' | '>') { opaque = true; }
            },
            '#' if !started => { opaque = true; word.push(character); started = true; },
            character if character.is_whitespace() => {
                if started { tokens.push(Token::Word(core::mem::take(&mut word))); started = false; }
            },
            character if character.is_control() => return Err(VerifierError::InvalidInput { reason: "control character in command" }),
            _ => { word.push(character); started = true; },
        }
        if tokens.len() > MAX_TOKENS {
            return Err(VerifierError::InvalidInput { reason: "too many shell tokens" });
        }
    }
    if quote.is_some() || escaped {
        return Err(VerifierError::InvalidInput { reason: "unterminated shell quoting" });
    }
    if started { tokens.push(Token::Word(word)); }
    Ok(ShellWords { tokens, opaque })
}

fn executable_name(word: &str) -> &str {
    match word.rsplit(['/', '\\']).next() { Some(name) => name, None => word }
}

fn privileged(name: &str) -> bool {
    matches!(name,
        "sudo" | "su" | "doas" | "runas" | "rm" | "rmdir" | "rd" | "del" | "erase"
        | "mkfs" | "dd" | "diskpart" | "format" | "mount" | "umount" | "chmod" | "chown"
        | "setfacl" | "chattr" | "eval" | "exec" | "source" | "." | "env" | "command"
        | "xargs" | "find" | "sh" | "bash" | "zsh" | "dash" | "powershell" | "pwsh" | "cmd")
        || name.starts_with("mkfs.")
}

impl<D: DecisionEngine, C: SchemaCatalog, S: Scanner> Verifier<D, C, S> {
    /// Assess command safety without executing it. Opaque shell constructs need approval.
    ///
    /// # Errors
    /// Returns input-bound, malformed-quoting or scanner errors; none imply permission.
    pub async fn check_command(&self, facts: &CommandFacts<'_>) -> Result<GateVerdict, VerifierError> {
        self.bound(facts.command.as_bytes())?;
        self.bound(facts.intent.as_bytes())?;
        let mut buffer = ScanBuf::with_capacity(self.config.max_scan_matches);
        self.scanner.scan(Category::DangerousShell, facts.command.as_bytes(), &mut buffer)?;
        if !buffer.is_empty() {
            return Ok(GateVerdict::Block { findings: buffer.matches().iter()
                .map(|found| Finding::DestructiveCommand { rule: found.rule }).collect() });
        }
        if facts.dialect != ShellDialect::Posix {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::OpaqueShell });
        }
        let shell = tokenize(facts.command)?;
        let normalized = shell.tokens.iter().map(|token| match token {
            Token::Word(word) | Token::Operator(word) => word.as_str(),
        }).collect::<Vec<_>>().join(" ");
        self.scanner.scan(Category::DangerousShell, normalized.as_bytes(), &mut buffer)?;
        if !buffer.is_empty() {
            return Ok(GateVerdict::Block { findings: buffer.matches().iter()
                .map(|found| Finding::DestructiveCommand { rule: found.rule }).collect() });
        }
        let mut command_start = true;
        let mut forbidden = false;
        let mut unlisted = false;
        let mut escape = false;
        for token in &shell.tokens {
            match token {
                Token::Operator(_) => command_start = true,
                Token::Word(word) => {
                    if command_start {
                        let name = executable_name(word);
                        forbidden |= privileged(&name.to_ascii_lowercase());
                        unlisted |= !facts.permitted_executables.contains(&name);
                        command_start = false;
                    }
                    escape |= word.starts_with("/proc/") || word.starts_with("/sys/")
                        || word.starts_with("/dev/") || word.split('/').any(|part| part == "..");
                },
            }
        }
        if forbidden { return Ok(GateVerdict::Block { findings: vec![Finding::PrivilegedCommand] }); }
        if escape || !facts.sandbox_established {
            return Ok(GateVerdict::Block { findings: vec![Finding::SandboxEscapeAttempt] });
        }
        if shell.opaque || shell.tokens.is_empty() || command_start || unlisted {
            return Ok(GateVerdict::AskUser { reason: DegradedReason::OpaqueShell });
        }
        let context = format!("INTENT\n{}\nSANDBOXED WORKING DIRECTORY\n{}", facts.intent, facts.working_directory);
        self.semantic("safety.command", facts.command.as_bytes(), Some(context.as_bytes())).await
    }
}
