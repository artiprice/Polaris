//! Slash commands every frontend answers locally, on the session's workspace, without the model:
//! `/nv-profile`, `/nv-verify` (P5), `/hack` (P7), `/ci` and `/ci-run` (P8). Each returns report lines that the terminal
//! shows as a sheet and the web and desktop views as a report entry.

use std::path::Path;

use polaris_kernel::KernelHandle;
use polaris_kernel::directives::directive::NvidiaMode;

use crate::SessionHostError;
use crate::nvidia::{nvidia_mode, report_lines};

/// A command answered locally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalCommand {
    /// `/nv-profile`: the NVIDIA stack, build targets and local GPUs.
    NvProfile,
    /// `/nv-verify`: the CUDA advisor over `.cu`/`.cuh` sources.
    NvVerify,
    /// `/hack`: races, blocking calls in async code, unsafe code, secrets and CUDA bugs.
    Hack,
    /// `/ci`: the workspace's CI pipeline, its caches and whether Docker can run it.
    Ci,
    /// `/ci-run`: run the pipeline in Docker with cache volumes.
    CiRun,
}

impl LocalCommand {
    /// The command `text` is exactly, if any (`/hack` with arguments is not one).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim() {
            "/nv-profile" => Some(Self::NvProfile),
            "/nv-verify" => Some(Self::NvVerify),
            "/hack" => Some(Self::Hack),
            "/ci" => Some(Self::Ci),
            "/ci-run" => Some(Self::CiRun),
            _ => None,
        }
    }

    /// Report title.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::NvProfile => "NVIDIA profile",
            Self::NvVerify => "CUDA advisor",
            Self::Hack => "Hack audit",
            Self::Ci => "CI pipeline",
            Self::CiRun => "CI run",
        }
    }

    /// Run the command for a session (the NVIDIA ones under its `$nvidia` mode).
    /// # Errors
    /// The workspace cannot be listed, or an analyzer fails.
    pub async fn run(
        self,
        kernel: &KernelHandle,
        workspace: &Path,
    ) -> Result<Vec<String>, SessionHostError> {
        let env = kernel.directives().get("env").cloned();
        match self {
            Self::Ci => crate::ci::plan_report(workspace, env.as_ref()).await,
            Self::CiRun => crate::ci::run_report(workspace, env.as_ref()).await,
            _ => self.run_with(nvidia_mode(kernel), workspace).await,
        }
    }

    /// Run the command on `workspace` (no session needed, as `polaris hack`), the NVIDIA ones
    /// under `mode`.
    /// # Errors
    /// The workspace cannot be listed, or an analyzer fails.
    pub async fn run_with(
        self,
        mode: NvidiaMode,
        workspace: &Path,
    ) -> Result<Vec<String>, SessionHostError> {
        match self {
            Self::NvProfile => report_lines(false, mode, workspace).await,
            Self::NvVerify => report_lines(true, mode, workspace).await,
            Self::Hack => {
                let root = workspace.to_owned();
                let report = tokio::task::spawn_blocking(move || crate::hack::hack(&root))
                    .await
                    .map_err(|_| SessionHostError::Unavailable("audit task failed"))??;
                Ok(crate::hack::hack_lines(&report))
            }
            Self::Ci => crate::ci::plan_report(workspace, None).await,
            Self::CiRun => crate::ci::run_report(workspace, None).await,
        }
    }
}
