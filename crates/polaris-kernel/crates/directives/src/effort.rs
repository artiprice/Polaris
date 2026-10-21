//! `$subagent-models(model(lo-hi), …)`: model entries with effort ranges.
//! Efforts are sdk [`Effort`] names (`low`…`max`, any case) or numbers `1`–`5`; one effort means
//! `lo == hi`; `lo <= hi` is required.

use polaris_sdk::effort::Effort;

use crate::args::{Arg, ArgParser};
use crate::cursor::Cursor;
use crate::directive::ModelEffort;
use crate::error::DirectiveError;
use crate::parser::ParseLimits;

/// Parses `lo-hi` or a single effort.
fn range(text: &str, at: usize) -> Result<(Effort, Effort), DirectiveError> {
    let bad = || DirectiveError::BadEffortRange { at };
    let mut parts = text.split('-');
    let lo: Effort = parts
        .next()
        .ok_or_else(bad)?
        .trim()
        .parse()
        .map_err(|_| bad())?;
    let hi: Effort = match parts.next() {
        Some(part) => part.trim().parse().map_err(|_| bad())?,
        None => lo,
    };
    if parts.next().is_some() || lo > hi {
        return Err(bad());
    }
    Ok((lo, hi))
}

/// Converts parsed `$subagent-models` arguments into entries.
pub(crate) fn model_efforts_from_args(
    args: &[Arg],
    at: usize,
) -> Result<Vec<ModelEffort>, DirectiveError> {
    let bad_arg = |reason| DirectiveError::BadArg {
        directive: "$subagent-models".to_owned(),
        at,
        reason,
    };
    if args.is_empty() {
        return Err(bad_arg("expected (model(lo-hi), …)"));
    }
    args.iter()
        .map(|arg| match arg {
            Arg::Call { name, args, .. } => match args.as_slice() {
                [Arg::Atom(text)] => {
                    let (lo, hi) = range(text, at)?;
                    Ok(ModelEffort {
                        model: name.clone(),
                        lo,
                        hi,
                    })
                }
                _ => Err(DirectiveError::BadEffortRange { at }),
            },
            _ => Err(bad_arg("each entry must be model(lo-hi)")),
        })
        .collect()
}

/// Parses the inside of `$subagent-models( … )`, e.g. `nano(1-2), "nvidia/x"(high-max)`.
///
/// # Errors
/// [`DirectiveError::BadEffortRange`] for a malformed or reversed range, [`DirectiveError::BadArg`]
/// for an entry that is not `model(range)`, and the grammar errors of the argument parser.
pub fn parse_model_efforts(args: &str) -> Result<Vec<ModelEffort>, DirectiveError> {
    let limits = ParseLimits::default();
    if args.len() > limits.max_input_bytes {
        return Err(DirectiveError::TooLong);
    }
    let parsed = ArgParser::new(Cursor::new(args, 0), &limits).bare_list()?;
    model_efforts_from_args(&parsed, 0)
}
