//! Owned task/temp resources and the generic platform process seam.
use crate::{Disposition, Teardown, TeardownContext, TeardownError};
use polaris_sdk::registry::{PlatformOps, ProcessTree, StopOutcome};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

pub(crate) struct TaskResource(pub JoinHandle<()>);
impl Teardown for TaskResource {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        match (&mut self.0).await {
            Ok(()) => Ok(Disposition::Clean),
            Err(error) if error.is_cancelled() => Ok(Disposition::Forced),
            Err(error) => Err(TeardownError::Other(error.to_string())),
        }
    }
    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        self.0.abort();
        match (&mut self.0).await {
            Ok(()) => Ok(()),
            Err(error) if error.is_cancelled() => Ok(()),
            Err(error) => Err(TeardownError::Other(error.to_string())),
        }
    }
}
impl Drop for TaskResource {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub(crate) struct TempResource(pub tempfile::TempDir);
impl Teardown for TempResource {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        let path = self.0.path().to_path_buf();
        // Run filesystem work off the async worker. TempDir remains owned on failure, and its
        // Drop is only a best-effort backstop; the reaper retains the failed locator.
        tokio::task::spawn_blocking(move || remove_owned(&path))
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))??;
        Ok(Disposition::Clean)
    }
}
pub(crate) fn remove_owned(path: &Path) -> std::io::Result<()> {
    // Only absolute, concrete owned directories are accepted. Do not recurse through a replaced
    // symlink/junction or a filesystem root. Callers own the TempDir or validate the orphan prefix.
    if !path.is_absolute() || path.parent().is_none() || path.file_name().is_none() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "refused unowned directory",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "missing parent")
    })?;
    let mut attempts = 0u32;
    loop {
        let metadata = match std::fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "refused replaced directory",
            ));
        }
        let canonical = std::fs::canonicalize(path)?;
        let canonical_parent = std::fs::canonicalize(parent)?;
        if canonical.parent() != Some(canonical_parent.as_path()) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "directory escaped its parent",
            ));
        }
        match std::fs::remove_dir_all(&canonical) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error)
                if attempts < 4
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                attempts = attempts.saturating_add(1);
                std::thread::sleep(Duration::from_millis(
                    20u64.saturating_mul(u64::from(attempts)),
                ));
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) struct ProcessResource<P: PlatformOps> {
    pub platform: Arc<P>,
    pub tree: ProcessTree,
}
impl<P: PlatformOps> Drop for ProcessResource<P> {
    fn drop(&mut self) {
        // This is a synchronous emergency backstop. Explicit shutdown remains responsible for
        // waiting, reporting outcomes and updating the recovery journal.
        match self.platform.kill_tree(&self.tree) {
            Ok(()) | Err(polaris_sdk::registry::PlatformError::NotFound(_)) => {}
            Err(error) => tracing::error!(pid = self.tree.root_pid, error = %error,
                "failed to stop registered process while dropping resource guard"),
        }
    }
}
impl<P: PlatformOps> Teardown for ProcessResource<P> {
    async fn graceful(&mut self, ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        let grace = match ctx.budget.checked_div(2) {
            Some(grace) => grace,
            None => Duration::ZERO,
        };
        match self
            .platform
            .graceful_stop(&self.tree, grace)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))?
        {
            StopOutcome::Exited { .. } => Ok(Disposition::Clean),
            StopOutcome::Killed => Ok(Disposition::Forced),
        }
    }
    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        let _outcome = self
            .platform
            .graceful_stop(&self.tree, Duration::ZERO)
            .await
            .map_err(|error| TeardownError::Other(error.to_string()))?;
        Ok(())
    }
}

pub(crate) fn orphan_temp_path(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute()
        || !path
            .file_name()?
            .to_str()?
            .starts_with(crate::TEMP_DIR_PREFIX)
    {
        return None;
    }
    if path.components().any(|part| {
        matches!(
            part,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )
    }) {
        return None;
    }
    Some(path.to_owned())
}
