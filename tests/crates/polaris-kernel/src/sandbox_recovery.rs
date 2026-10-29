// Unit tests of crates/polaris-kernel/src/sandbox_recovery.rs (`tests`); compiled into that module via #[path].
use super::*;
use crate::reaper::{Disposition, Locator, ResourceKind, Teardown, TeardownContext, TeardownError};
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
