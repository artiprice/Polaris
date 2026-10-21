//! `cargo xtask <command>` entry point.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use xtask::error::XtaskError;
use xtask::table::CrateTable;
use xtask::{layers, members, metadata, policy};

const TABLE: &str = "@prime/tools/workspace.toml";
const USAGE: &str = "cargo xtask <check-members|check-layers|check-witness> [--root <repo>]";

fn run() -> Result<bool, XtaskError> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?;
    let mut root = PathBuf::from(".");
    while let Some(flag) = args.next() {
        match (flag.as_str(), args.next()) {
            ("--root", Some(path)) => root = PathBuf::from(path),
            _ => return Err(XtaskError::Usage(USAGE.to_owned())),
        }
    }
    let table = CrateTable::load(&root.join(TABLE))?;
    let mut out = std::io::stdout().lock();
    match command.as_str() {
        "check-members" => {
            let violations = members::check_members(&root, &table)?;
            for v in &violations {
                writeln!(out, "FAIL {v:?}")?;
            }
            writeln!(
                out,
                "check-members: {} packages -> {}",
                table.crates.len(),
                verdict(violations.is_empty())
            )?;
            Ok(violations.is_empty())
        }
        "check-layers" => {
            let actual = metadata::internal_edges(&root)?;
            let violations = layers::check_layers(&table, &actual);
            for v in &violations {
                writeln!(out, "FAIL {v:?}")?;
            }
            let edges: usize = actual.values().map(std::collections::BTreeSet::len).sum();
            writeln!(
                out,
                "check-layers: {} packages, {edges} edges -> {}",
                table.crates.len(),
                verdict(violations.is_empty())
            )?;
            Ok(violations.is_empty())
        }
        "check-witness" => {
            let violations = policy::check_witness(&root)?;
            for v in &violations {
                writeln!(out, "FAIL ApprovalWitness::new outside sdk/verifier at {v}")?;
            }
            writeln!(
                out,
                "check-witness: {} violation(s) -> {}",
                violations.len(),
                verdict(violations.is_empty())
            )?;
            Ok(violations.is_empty())
        }
        _ => Err(XtaskError::Usage(USAGE.to_owned())),
    }
}

const fn verdict(ok: bool) -> &'static str {
    if ok { "OK" } else { "FAILED" }
}

fn main() -> ExitCode {
    match run() {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            let _ignored = writeln!(std::io::stderr().lock(), "xtask: {e}");
            ExitCode::from(2)
        }
    }
}
