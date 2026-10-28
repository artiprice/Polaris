//! `polaris-desktop` binary: parses arguments and runs [`polaris_desktop::run`].

#![forbid(unsafe_code)]
// Release builds start without a console window; debug builds keep it for diagnostics.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use miette::IntoDiagnostic as _;

fn main() -> miette::Result<()> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let args = polaris_desktop::parse_args(
        std::env::args_os().skip(1),
        std::env::var_os("POLARIS_REDUCED_MOTION"),
        cwd,
        std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from),
    )
    .into_diagnostic()?;
    let _report = polaris_desktop::run(&args).into_diagnostic()?;
    Ok(())
}
