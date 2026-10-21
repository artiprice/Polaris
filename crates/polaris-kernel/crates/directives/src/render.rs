//! Canonical text form of a directive (used by `/status`, `/settings` and D11 persistence).
//! `parse(render(d)) == d` holds for every directive the parser can produce (property-tested).

use core::fmt::Write as _;

use crate::args::{Arg, is_atom_char, is_ident_char, is_ident_start};
use crate::directive::{
    Budget, ListArg, ModelEffort, NvidiaMode, Pool, SandboxMode, SessionDirective, Toggle,
};

/// Quotes `text` with `"…"`, escaping `\` and `"`.
fn push_quoted(out: &mut String, text: &str) {
    out.push('"');
    for c in text.chars() {
        if matches!(c, '\\' | '"') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// `true` when `text` must be quoted to re-parse as the same single word.
fn needs_quotes(text: &str) -> bool {
    let looks_like_kv = text.starts_with(is_ident_start) && {
        let after_ident = text.trim_start_matches(is_ident_char);
        after_ident.trim_start().starts_with('=')
    };
    text.is_empty() || looks_like_kv || !text.chars().all(is_atom_char)
}

fn push_word(out: &mut String, text: &str) {
    if needs_quotes(text) {
        push_quoted(out, text);
    } else {
        out.push_str(text);
    }
}

fn push_arg(out: &mut String, arg: &Arg) {
    match arg {
        Arg::Atom(text) => out.push_str(text),
        Arg::Quoted(text) => push_quoted(out, text),
        Arg::Kv { key, value } => {
            out.push_str(key);
            out.push('=');
            push_arg(out, value);
        }
        Arg::Call { name, quoted, args } => {
            if *quoted {
                push_quoted(out, name);
            } else {
                out.push_str(name);
            }
            push_args(out, args);
        }
    }
}

fn push_args(out: &mut String, args: &[Arg]) {
    out.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        push_arg(out, arg);
    }
    out.push(')');
}

fn toggle(name: &str, t: Toggle) -> String {
    match t {
        Toggle::On => format!("${name}"),
        Toggle::Off => format!("${name}(off)"),
    }
}

fn list(name: &str, l: &ListArg) -> String {
    match l {
        ListArg::On => format!("${name}"),
        ListArg::Off => format!("${name}(off)"),
        ListArg::Items(items) => {
            let mut out = format!("${name}(");
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                if item.eq_ignore_ascii_case("on") || item.eq_ignore_ascii_case("off") {
                    push_quoted(&mut out, item);
                } else {
                    push_word(&mut out, item);
                }
            }
            out.push(')');
            out
        }
    }
}

fn model_entry(out: &mut String, m: &ModelEffort) {
    push_word(out, &m.model);
    out.push('(');
    out.push_str(m.lo.name());
    if m.hi != m.lo {
        out.push('-');
        out.push_str(m.hi.name());
    }
    out.push(')');
}

const fn pool_name(p: Pool) -> &'static str {
    match p {
        Pool::Inference => "inference",
        Pool::Web => "web",
        Pool::Docs => "docs",
        Pool::Disk => "disk",
        Pool::S1 => "s1",
        Pool::Sandbox => "sandbox",
    }
}

/// Canonical text form of `d`, e.g. `$websearch`, `$sandbox(remote)`,
/// `$subagent-models(nano(low-medium))`.
#[must_use]
pub fn render(d: &SessionDirective) -> String {
    match d {
        SessionDirective::WebSearch(t) => toggle("websearch", *t),
        SessionDirective::Subagents(t) => toggle("subagents", *t),
        SessionDirective::SubagentModels(models) => {
            let mut out = String::from("$subagent-models(");
            for (i, m) in models.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                model_entry(&mut out, m);
            }
            out.push(')');
            out
        }
        SessionDirective::Links(l) => list("links", l),
        SessionDirective::Skills(l) => list("skills", l),
        SessionDirective::Mcp(l) => list("mcp", l),
        SessionDirective::Rag(l) => list("rag", l),
        SessionDirective::Nvidia(mode) => match mode {
            NvidiaMode::Auto => "$nvidia".to_owned(),
            NvidiaMode::List => "$nvidia(list)".to_owned(),
            NvidiaMode::Off => "$nvidia(off)".to_owned(),
        },
        SessionDirective::NvidiaDocs(_) => "$nvidia-docs(sync)".to_owned(),
        SessionDirective::Sandbox(mode) => {
            let word = match mode {
                SandboxMode::Auto => "auto",
                SandboxMode::Openshell => "openshell",
                SandboxMode::Microvm => "microvm",
                SandboxMode::Contree => "contree",
                SandboxMode::Remote => "remote",
            };
            format!("$sandbox({word})")
        }
        SessionDirective::NebiusBranching { on, max_branches } => match (on, max_branches) {
            (true, None) => "$nebius-branching".to_owned(),
            (true, Some(n)) => format!("$nebius-branching(on, max_branches={n})"),
            (false, None) => "$nebius-branching(off)".to_owned(),
            (false, Some(n)) => format!("$nebius-branching(off, max_branches={n})"),
        },
        SessionDirective::NebiusDatalab(t) => toggle("nebius-datalab", *t),
        SessionDirective::NebiusSpeculative(t) => toggle("nebius-speculative", *t),
        SessionDirective::Concurrency(pools) => {
            let mut out = String::from("$concurrency(");
            for (i, (pool, n)) in pools.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                let _infallible = write!(out, "{}={n}", pool_name(*pool));
            }
            out.push(')');
            out
        }
        SessionDirective::RateLimit { rpm, tpm } => {
            let parts: Vec<String> = [
                rpm.map(|v| format!("rpm={v}")),
                tpm.map(|v| format!("tpm={v}")),
            ]
            .into_iter()
            .flatten()
            .collect();
            if parts.is_empty() {
                "$ratelimit".to_owned()
            } else {
                format!("$ratelimit({})", parts.join(", "))
            }
        }
        SessionDirective::Budget(Budget::Usd(v)) => format!("$budget(usd={v})"),
        SessionDirective::Budget(Budget::Tokens(v)) => format!("$budget(tokens={v})"),
        SessionDirective::StorageLevel(level) => match level {
            crate::directive::StorageLevel::Global => "$storage-level(global)".to_owned(),
            crate::directive::StorageLevel::Isolated => "$storage-level(isolated)".to_owned(),
        },
        SessionDirective::Browser(t) => toggle("browser", *t),
        SessionDirective::Needle(t) => toggle("needle", *t),
        SessionDirective::Env(kind) => match kind {
            crate::directive::EnvKind::Nix => "$env(nix)".to_owned(),
            crate::directive::EnvKind::Docker => "$env(docker)".to_owned(),
            crate::directive::EnvKind::Host => "$env(host)".to_owned(),
        },
        SessionDirective::Ci(_) => "$ci(dagger)".to_owned(),
        SessionDirective::Custom { name, args } => {
            let mut out = format!("${name}");
            if !args.is_empty() {
                push_args(&mut out, args);
            }
            out
        }
    }
}
