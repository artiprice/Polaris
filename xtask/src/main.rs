//! `cargo xtask <command>` entry point.

use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

use xtask::error::XtaskError;
use xtask::table::CrateTable;
use xtask::{layers, members, metadata, policy, release_sbom};

const TABLE: &str = "@prime/tools/workspace.toml";
const USAGE: &str = "cargo xtask calibrate-binary --fixtures <jsonl> --predictions <jsonl> --checkpoint <file> --schema <id> --version <n> --minimum-precision-ppm <0..1000000> --minimum-positive <n> --output <new-toml> | cargo xtask <check-members|check-layers|check-witness> [--root <repo>] | cargo xtask release-sbom --root <workspace> --output <path> --created YYYY-MM-DDTHH:MM:SSZ [--force]";

fn run() -> Result<bool, XtaskError> {
    let mut args = std::env::args().skip(1);
    let command = args
        .next()
        .ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?;
    if command == "calibrate-binary" {
        xtask::calibration::run_cli(args)?;
        writeln!(
            std::io::stdout().lock(),
            "calibrate-binary: measured candidate written; builtin policy unchanged"
        )?;
        return Ok(true);
    }
    let mut root = PathBuf::from(".");
    let mut output_path = None;
    let mut force = false;
    let mut created = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--root" => {
                root = PathBuf::from(
                    args.next()
                        .ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?,
                );
            }
            "--output" if command == "release-sbom" => {
                output_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?,
                ));
            }
            "--force" if command == "release-sbom" => force = true,
            "--created" if command == "release-sbom" => {
                created = Some(
                    args.next()
                        .ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?,
                );
            }
            _ => return Err(XtaskError::Usage(USAGE.to_owned())),
        }
    }
    if command == "release-sbom" {
        let output = output_path.ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?;
        let created = created.ok_or_else(|| XtaskError::Usage(USAGE.to_owned()))?;
        let absolute_root = root.canonicalize()?;
        let resolved_output = if output.is_absolute() {
            output
        } else {
            absolute_root.join(output)
        };
        let parent = resolved_output.parent().ok_or_else(|| {
            XtaskError::Usage("SBOM output must be inside the requested workspace".to_owned())
        })?;
        let absolute_parent = parent.canonicalize()?;
        if !absolute_parent.starts_with(&absolute_root) {
            return Err(XtaskError::Usage(
                "SBOM output must be inside the requested workspace".to_owned(),
            ));
        }
        let output = absolute_parent.join(
            resolved_output
                .file_name()
                .ok_or_else(|| XtaskError::Usage("SBOM output must name a file".to_owned()))?,
        );
        let byte_count =
            release_sbom::create_release_sbom(&absolute_root, &output, force, &created)?;
        writeln!(
            std::io::stdout().lock(),
            "release-sbom: wrote {} bytes to {}",
            byte_count,
            output.display()
        )?;
        return Ok(true);
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
