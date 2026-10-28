//! Polaris entry point; mandatory provider and allocator precede the kernel.
use clap::Parser;
use std::{io::Write, process::ExitCode};
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;
fn main() -> ExitCode {
    let args = match polaris_cli::Args::try_parse() {
        Ok(args) => args,
        Err(error) => {
            let success = matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            );
            if error.print().is_err() {
                return ExitCode::FAILURE;
            }
            return if success {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };
    if let Err(error) = polaris_cli::install_crypto_provider() {
        return fail(error);
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return fail(polaris_cli::CliError::Io(error)),
    };
    match runtime.block_on(polaris_cli::run(args)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(polaris_cli::CliError::TaskUnverified) => {
            fail(polaris_cli::CliError::TaskUnverified);
            ExitCode::from(3)
        }
        Err(error) => fail(error),
    }
}
fn fail(error: polaris_cli::CliError) -> ExitCode {
    let _reported = writeln!(std::io::stderr().lock(), "polaris: {error}");
    ExitCode::FAILURE
}
