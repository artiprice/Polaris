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
mod tests {
    use super::*;
    use crate::reaper::{
        Disposition, Locator, ResourceKind, Teardown, TeardownContext, TeardownError,
    };
    use std::collections::BTreeSet;

    struct Pending;

    impl Teardown for Pending {
        fn graceful(
            &mut self,
            _ctx: TeardownContext,
        ) -> impl Future<Output = Result<Disposition, TeardownError>> + Send {
            async { Ok(Disposition::Clean) }
        }
    }

    struct Probe {
        domain: Cid,
    }

    impl SandboxRecovery for Probe {
        fn recovery_domain(&self) -> Option<Cid> {
            Some(self.domain)
        }

        fn confirm_absence<'a>(
            &'a self,
            handle: &'a SandboxHandle,
        ) -> Pin<Box<dyn Future<Output = Result<bool, SandboxError>> + Send + 'a>> {
            Box::pin(async move {
                match handle.as_str() {
                    "absent" => Ok(true),
                    "present" => Ok(false),
                    _ => Err(SandboxError::Unavailable("query uncertain".to_owned())),
                }
            })
        }
    }

    #[test]
    fn locator_encoding_is_versioned_scoped_and_strict() {
        let handle = SandboxHandle::new("p2-0123456789abcdef");
        assert!(handle.is_ok());
        let Some(handle) = handle.ok() else {
            return;
        };
        let domain = Cid::from_bytes([7; 32]);
        let value = format!("{LOCATOR_PREFIX}/{domain}/{}", handle.as_str());
        assert_eq!(decode_locator(&value), Some((domain, handle)));
        assert!(decode_locator("p2-0123456789abcdef").is_none());
        assert!(decode_locator(&format!("{LOCATOR_PREFIX}/{domain}/bad/name")).is_none());
        assert!(decode_locator(&format!("{LOCATOR_PREFIX}/not-a-cid/p2-name")).is_none());
    }

    #[tokio::test]
    async fn recovery_acknowledges_only_absent_matching_scoped_records()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let domain = Cid::from_bytes([7; 32]);
        let other_domain = Cid::from_bytes([8; 32]);
        let first = Reaper::new(directory.path())?;
        let mut ids = Vec::new();
        for (domain, name) in [
            (domain, "absent"),
            (domain, "present"),
            (domain, "unknown"),
            (other_domain, "absent"),
        ] {
            let handle = SandboxHandle::new(name)?;
            ids.push(first.register_journaled(
                ResourceKind::Sandbox,
                Locator::Label(format!("{LOCATOR_PREFIX}/{domain}/{name}")),
                Pending,
            )?);
            drop(handle);
        }
        let legacy = first.register_journaled(
            ResourceKind::Sandbox,
            Locator::Label("legacy-name-only".to_owned()),
            Pending,
        )?;
        drop(first);

        let reaper = Reaper::new(directory.path())?;
        recover_absent_sandboxes(&reaper, &Probe { domain }).await?;
        let pending: BTreeSet<_> = reaper
            .pending_orphans()
            .into_iter()
            .map(|entry| entry.id)
            .collect();
        assert_eq!(pending.len(), 4);
        assert!(!pending.contains(&ids[0]));
        assert!(pending.contains(&ids[1]));
        assert!(pending.contains(&ids[2]));
        assert!(pending.contains(&ids[3]));
        assert!(pending.contains(&legacy));
        Ok(())
    }
}
