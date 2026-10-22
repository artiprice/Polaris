//! Piped RTK CLI entry: bounded stdin, CAS preservation, ledger reconciliation and stable output.

use std::{
    io::{self, Read, Write},
    path::PathBuf,
};

use clap::Args;

/// Configuration and mode for `polaris rtk`.
///
/// The application root fills `state_dir` and `workspace` from its global options before calling
/// the RTK runner. They are skipped by Clap here to avoid defining duplicate command-line flags.
#[derive(Debug, Args)]
pub struct RtkArgs {
    /// Pipeline mode: auto, json, filter, or issues.
    #[arg(long, value_enum, default_value_t = RtkModeArg::Auto)]
    pub mode: RtkModeArg,
    /// Override the default `.polaris/issues.md` path.
    #[arg(long)]
    pub issues_file: Option<PathBuf>,
    /// State directory from the application root.
    #[arg(skip)]
    pub state_dir: PathBuf,
    /// Workspace from the application root.
    #[arg(skip)]
    pub workspace: PathBuf,
}

/// RTK presentation/reconciliation mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum RtkModeArg {
    /// Filter input and update the ledger.
    Auto,
    /// Emit a structured JSON report without changing the ledger.
    Json,
    /// Emit filtered diagnostics without changing the ledger.
    Filter,
    /// Update the ledger and emit only its summary.
    Issues,
}

/// Typed failures at the command boundary.
#[derive(Debug, thiserror::Error)]
pub enum RtkCliError {
    /// Reading piped compiler output failed.
    #[error("could not read RTK input: {0}")]
    Input(#[from] io::Error),
    /// The CAS could not be opened.
    #[error("could not open RTK content store")]
    Store,
    /// The RTK parser or ledger failed.
    #[error(transparent)]
    Pipeline(#[from] polaris_tools_rtk::RtkError),
    /// Writing the filtered report failed.
    #[error("could not write RTK output: {0}")]
    Output(io::Error),
    /// The latest stream left one or more open ledger issues.
    #[error("RTK reports open issues")]
    OpenIssues,
}

/// Read stdin under the engine byte cap, preserve the raw stream in CAS, and run the selected mode.
///
/// This function is synchronous because stream parsing, CAS access, locking and atomic ledger I/O
/// are blocking operations. The CLI dispatcher must call it on a blocking worker.
///
/// # Errors
/// Returns a typed input, CAS, pipeline, output, or open-issues failure.
pub fn run(args: RtkArgs) -> Result<(), RtkCliError> {
    let read_limit = match u64::try_from(polaris_tools_rtk::MAX_STREAM_BYTES) {
        Ok(limit) => limit.saturating_add(1),
        Err(_) => u64::MAX,
    };
    let mut input = Vec::new();
    io::stdin().take(read_limit).read_to_end(&mut input)?;

    let cas = polaris_storage_cas::Cas::open(polaris_storage_cas::CasConfig::new(
        args.state_dir.join("cas"),
    ))
    .map_err(|_| RtkCliError::Store)?;
    let ledger =
        polaris_tools_rtk::LedgerFile::resolve(args.issues_file.as_deref(), &args.workspace);
    let mode = match args.mode {
        RtkModeArg::Auto => polaris_tools_rtk::RtkMode::Auto,
        RtkModeArg::Json => polaris_tools_rtk::RtkMode::Json,
        RtkModeArg::Filter => polaris_tools_rtk::RtkMode::Filter,
        RtkModeArg::Issues => polaris_tools_rtk::RtkMode::Issues,
    };
    let output = polaris_tools_rtk::run(mode, &input, &ledger, jiff::Timestamp::now(), &cas)?;

    let mut stdout = io::stdout().lock();
    if !output.filtered.is_empty() {
        stdout
            .write_all(output.filtered.as_bytes())
            .map_err(RtkCliError::Output)?;
        if !output.filtered.ends_with('\n') {
            writeln!(stdout).map_err(RtkCliError::Output)?;
        }
    }
    if !output.view.is_empty() {
        if mode == polaris_tools_rtk::RtkMode::Issues {
            stdout
                .write_all(output.view.as_bytes())
                .map_err(RtkCliError::Output)?;
        } else {
            let mut stderr = io::stderr().lock();
            stderr
                .write_all(output.view.as_bytes())
                .map_err(RtkCliError::Output)?;
            stderr.flush().map_err(RtkCliError::Output)?;
        }
    }
    stdout.flush().map_err(RtkCliError::Output)?;
    if output.exit_code != 0 {
        return Err(RtkCliError::OpenIssues);
    }
    Ok(())
}
