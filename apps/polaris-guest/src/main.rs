//! Minimal guest entry point for the host protocol-version handshake.

use std::io::{self, Write};
use std::process::ExitCode;

mod sync_cas;
mod sync_snapshot;

const PROTOCOL_VERSION: &str = "1";

fn main() -> ExitCode {
    match std::env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [command, root] if command == "sync-init" => sync_cas::run_init(root),
        [command, root, generation, offset, limit] if command == "sync-snapshot" => {
            sync_snapshot::run(root, generation, offset, limit)
        }
        [argument] if argument == "sync-cas" => sync_cas::run(),
        [argument] if argument == "--protocol-version" => write_stdout(PROTOCOL_VERSION),
        [argument] if argument == "--version" => write_stdout(env!("CARGO_PKG_VERSION")),
        [argument] if argument == "--help" || argument == "-h" => write_stdout(
            "polaris-guest --protocol-version | --version | sync-init ROOT | sync-snapshot ROOT GENERATION OFFSET LIMIT | sync-cas | --help",
        ),
        _ => {
            let mut stderr = io::stderr().lock();
            match writeln!(
                stderr,
                "usage: polaris-guest --protocol-version | --version | sync-init ROOT | sync-snapshot ROOT GENERATION OFFSET LIMIT | sync-cas | --help"
            ) {
                Ok(()) => ExitCode::from(2),
                Err(_) => ExitCode::FAILURE,
            }
        }
    }
}

fn write_stdout(value: &str) -> ExitCode {
    let mut stdout = io::stdout().lock();
    match writeln!(stdout, "{value}") {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}
