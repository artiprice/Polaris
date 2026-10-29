//! Scoped, absence-only recovery for durable sandbox creation intents.

use crate::{KernelError, reaper::Reaper};
use polaris_sdk::{
    ids::Cid,
    registry::{SandboxBackend, SandboxError, SandboxHandle},
};
use std::{future::Future, pin::Pin};

const LOCATOR_PREFIX: &str = "openshell-sandbox-v1";

/// Object-safe startup view over the generic backend contract.
pub(crate) trait SandboxRecovery: Send + Sync {
    fn recovery_domain(&self) -> Option<Cid>;
    fn confirm_absence<'a>(
        &'a self,
        handle: &'a SandboxHandle,
    ) -> Pin<Box<dyn Future<Output = Result<bool, SandboxError>> + Send + 'a>>;
}

impl<B: SandboxBackend> SandboxRecovery for B {
    fn recovery_domain(&self) -> Option<Cid> {
        SandboxBackend::recovery_domain(self)
    }

    fn confirm_absence<'a>(
        &'a self,
        handle: &'a SandboxHandle,
    ) -> Pin<Box<dyn Future<Output = Result<bool, SandboxError>> + Send + 'a>> {
        Box::pin(SandboxBackend::confirm_absence(self, handle))
    }
}

/// Encode a future creation intent only when the backend exposes a stable recovery authority.
pub(crate) fn locator_for(
    backend: &impl SandboxBackend,
    handle: &SandboxHandle,
) -> crate::reaper::Locator {
    let locator = backend.recovery_domain().map_or_else(
        || handle.as_str().to_owned(),
        |domain| format!("{LOCATOR_PREFIX}/{domain}/{}", handle.as_str()),
    );
    crate::reaper::Locator::Label(locator)
}

/// Resolve only versioned sandbox intents belonging to this exact backend, and only when an
/// authoritative query proves the recorded name absent. Present, unknown, legacy and mismatched
/// records remain in the journal for the generic sweep to report as `NeedsOperator`.
pub(crate) async fn recover_absent_sandboxes(
    reaper: &Reaper,
    backend: &dyn SandboxRecovery,
) -> Result<(), KernelError> {
    let Some(domain) = backend.recovery_domain() else {
        return Ok(());
    };
    for orphan in reaper.pending_orphans() {
        if orphan.kind != crate::reaper::ResourceKind::Sandbox {
            continue;
        }
        let crate::reaper::Locator::Label(value) = orphan.locator else {
            continue;
        };
        let Some((record_domain, handle)) = decode_locator(&value) else {
            continue;
        };
        if record_domain != domain {
            continue;
        }
        if matches!(backend.confirm_absence(&handle).await, Ok(true)) {
            reaper.acknowledge_orphan(orphan.id)?;
        }
    }
    Ok(())
}

fn decode_locator(value: &str) -> Option<(Cid, SandboxHandle)> {
    let mut fields = value.split('/');
    if fields.next()? != LOCATOR_PREFIX {
        return None;
    }
    let domain = Cid::from_hex(fields.next()?).ok()?;
    let name = fields.next()?;
    if fields.next().is_some() {
        return None;
    }
    Some((domain, SandboxHandle::new(name).ok()?))
}

#[cfg(test)]
#[path = "../../../tests/crates/polaris-kernel/src/sandbox_recovery.rs"]
mod tests;
