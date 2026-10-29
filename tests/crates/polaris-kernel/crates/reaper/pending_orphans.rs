//! Orphan-only Listing (descriptor: specs/descriptors/reaper_pending_orphans.json).

use std::error::Error;

use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, Teardown, TeardownContext, TeardownError,
};

type TestResult = Result<(), Box<dyn Error>>;

struct Noop;

impl Teardown for Noop {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        Ok(Disposition::Clean)
    }
}

#[test]
fn pending_orphans_excludes_live_registrations_and_returns_restart_records() -> TestResult {
    let directory = tempfile::tempdir()?;
    let id = {
        let reaper = Reaper::new(directory.path())?;
        let id = reaper.register_journaled(
            ResourceKind::Sandbox,
            Locator::Label("docker-pool-v1/pool/owner".to_owned()),
            Noop,
        )?;
        assert!(reaper.pending_orphans().is_empty());
        assert_eq!(reaper.leak_report().entries.len(), 1);
        id
    };

    let recovered = Reaper::new(directory.path())?;
    let orphans = recovered.pending_orphans();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans.first().map(|orphan| orphan.id), Some(id));
    recovered.acknowledge_orphan(id)?;
    assert!(recovered.pending_orphans().is_empty());
    Ok(())
}
