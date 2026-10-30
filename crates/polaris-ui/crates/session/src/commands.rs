//! Slash commands every frontend answers locally, on the session's workspace, without the model:
//! `/nv-profile`, `/nv-verify`, `/search-local <query>`, `/nv-docs-sync`, `/nv-docs <query>`,
//! `/search-web <query>` (P5), `/hack` (P7), `/ci` and `/ci-run` (P8). Each returns report lines
//! that the terminal shows as a sheet and the web and desktop views as a report entry.

use std::path::Path;

use polaris_kernel::KernelHandle;
use polaris_kernel::directives::directive::NvidiaMode;

use crate::SessionHostError;
use crate::nvidia::{nvidia_mode, report_lines};

/// A command answered locally.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LocalCommand {
    /// `/nv-profile`: the NVIDIA stack, build targets and local GPUs.
    NvProfile,
    /// `/nv-verify`: the CUDA advisor over `.cu`/`.cuh` sources.
    NvVerify,
    /// `/search-local <query>`: ranked workspace passages with their provenance.
    SearchLocal(String),
    /// `/hack`: races, blocking calls in async code, unsafe code, secrets and CUDA bugs.
    Hack,
    /// `/ci`: the workspace's CI pipeline, its caches and whether Docker can run it.
    Ci,
    /// `/ci-run`: run the pipeline in Docker with cache volumes.
    CiRun,
    /// `/nv-docs-sync` or `$nvidia-docs(sync)`: download the NVIDIA documentation seed.
    NvDocsSync,
    /// `/nv-docs <query>`: search the local NVIDIA documentation seed.
    NvDocs(String),
    /// `/search-web <query>`: results from the configured SearXNG instance.
    SearchWeb(String),
    /// `/diff`: the changes this session applied.
    Diff,
    /// `/undo`: restore the newest applied change still in place.
    Undo,
    /// `/research <goal>|status|report|resume`: the `program.md` research loop.
    Research(crate::research::ResearchCommand),
}

impl LocalCommand {
    /// The command `text` is, if any (`/hack` with arguments is not one; the search commands
    /// need a query, quoted or not).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let query = |rest: &str| {
            let query = rest.trim().trim_matches('"').trim();
            (!query.is_empty()).then(|| query.to_owned())
        };
        if let Some(rest) = text.strip_prefix("/search-local ") {
            return query(rest).map(Self::SearchLocal);
        }
        if let Some(rest) = text.strip_prefix("/nv-docs ") {
            return query(rest).map(Self::NvDocs);
        }
        if let Some(rest) = text.strip_prefix("/search-web ") {
            return query(rest).map(Self::SearchWeb);
        }
        if let Some(rest) = text.strip_prefix("/research ") {
            return crate::research::ResearchCommand::parse(rest).map(Self::Research);
        }
        match text {
            "/nv-profile" => Some(Self::NvProfile),
            "/nv-verify" => Some(Self::NvVerify),
            "/hack" => Some(Self::Hack),
            "/ci" => Some(Self::Ci),
            "/ci-run" => Some(Self::CiRun),
            "/diff" => Some(Self::Diff),
            "/undo" => Some(Self::Undo),
            // The directive is an action, not session state: it runs once here.
            "/nv-docs-sync" | "$nvidia-docs(sync)" | "$nvidia-docs" => Some(Self::NvDocsSync),
            _ => None,
        }
    }

    /// Report title.
    #[must_use]
    pub const fn title(&self) -> &'static str {
        match self {
            Self::NvProfile => "NVIDIA profile",
            Self::NvVerify => "CUDA advisor",
            Self::SearchLocal(_) => "Local search",
            Self::Hack => "Hack audit",
            Self::Ci => "CI pipeline",
            Self::CiRun => "CI run",
            Self::NvDocsSync => "NVIDIA docs sync",
            Self::NvDocs(_) => "NVIDIA docs",
            Self::SearchWeb(_) => "Web search",
            Self::Diff => "Applied changes",
            Self::Undo => "Undo",
            Self::Research(_) => "Research",
        }
    }

    /// Run the command for a session (the NVIDIA ones under its `$nvidia` mode, the CI ones
    /// noting its `$env`).
    /// # Errors
    /// The workspace cannot be listed, or an analyzer fails.
    pub async fn run(
        &self,
        kernel: &KernelHandle,
        workspace: &Path,
    ) -> Result<Vec<String>, SessionHostError> {
        let env = kernel.directives().get("env").cloned();
        match self {
            Self::Ci => crate::ci::plan_report(workspace, env.as_ref()).await,
            Self::CiRun => crate::ci::run_report(workspace, env.as_ref()).await,
            Self::Diff => crate::changes::diff_lines(kernel).await,
            Self::Undo => crate::changes::undo_lines(kernel, workspace).await,
            _ => self.run_with(nvidia_mode(kernel), workspace).await,
        }
    }

    /// Run the command on `workspace` (no session needed, as `polaris hack`), the NVIDIA ones
    /// under `mode`.
    /// # Errors
    /// The workspace cannot be listed, or an analyzer fails.
    pub async fn run_with(
        &self,
        mode: NvidiaMode,
        workspace: &Path,
    ) -> Result<Vec<String>, SessionHostError> {
        match self {
            Self::NvProfile => report_lines(false, mode, workspace).await,
            Self::NvVerify => report_lines(true, mode, workspace).await,
            Self::SearchLocal(query) => {
                let root = workspace.to_owned();
                let owned = query.clone();
                let hits = tokio::task::spawn_blocking(move || {
                    crate::local_search::search(&root, &owned, crate::local_search::SEARCH_HITS)
                })
                .await
                .map_err(|_| SessionHostError::Unavailable("search task failed"))??;
                Ok(crate::local_search::search_lines(query, &hits))
            }
            Self::Hack => {
                let root = workspace.to_owned();
                let report = tokio::task::spawn_blocking(move || crate::hack::hack(&root))
                    .await
                    .map_err(|_| SessionHostError::Unavailable("audit task failed"))??;
                Ok(crate::hack::hack_lines(&report))
            }
            Self::Ci => crate::ci::plan_report(workspace, None).await,
            Self::CiRun => crate::ci::run_report(workspace, None).await,
            Self::NvDocsSync => crate::nvidia::docs_sync_lines(workspace).await,
            Self::NvDocs(query) => {
                let root = workspace.to_owned();
                let owned = query.clone();
                tokio::task::spawn_blocking(move || crate::nvidia::docs_search_lines(&root, &owned))
                    .await
                    .map_err(|_| SessionHostError::Unavailable("docs search task failed"))?
            }
            Self::SearchWeb(query) => crate::web_search::search_lines(workspace, query).await,
            Self::Research(command) => crate::research::research_lines(workspace, command).await,
            Self::Diff | Self::Undo => Err(SessionHostError::Unavailable(
                "/diff and /undo need an open session",
            )),
        }
    }
}
