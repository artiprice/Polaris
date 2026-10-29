//! Leak Report (descriptor: specs/descriptors/reaper_leak_report.json).

use std::error::Error;

use polaris_kernel_reaper::{
    Disposition, Locator, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
    TeardownError,
};

type TestResult = Result<(), Box<dyn Error>>;

struct Noop;

impl Teardown for Noop {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        Ok(Disposition::Clean)
    }
}

#[test]
fn live_resources_are_listed_with_kind_and_locator() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let plain = reaper.register(ResourceKind::Task, Noop)?;
    let sandbox = reaper.register_journaled(
        ResourceKind::Sandbox,
        Locator::Label("sbx-1".to_owned()),
        Noop,
    )?;
    let process = reaper.register_journaled(ResourceKind::Process, Locator::Pid(1234), Noop)?;

    let report = reaper.leak_report();

    let listed: Vec<_> = report
        .entries
        .iter()
        .map(|e| (e.id, e.kind, e.locator.clone()))
        .collect();
    assert_eq!(
        listed,
        [
            (plain, ResourceKind::Task, Locator::None),
            (
                sandbox,
                ResourceKind::Sandbox,
                Locator::Label("sbx-1".to_owned())
            ),
            (process, ResourceKind::Process, Locator::Pid(1234)),
        ]
    );
    Ok(())
}

#[test]
fn released_resources_leave_the_report() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    let kept = reaper.register(ResourceKind::Task, Noop)?;
    let gone = reaper.register(ResourceKind::Task, Noop)?;

    reaper.release(gone)?;

    let ids: Vec<_> = reaper.leak_report().entries.iter().map(|e| e.id).collect();
    assert_eq!(ids, [kept]);
    Ok(())
}

#[tokio::test]
async fn empty_reaper_reports_no_leaks() -> TestResult {
    let dir = tempfile::tempdir()?;
    let reaper = Reaper::new(dir.path())?;
    assert!(reaper.leak_report().entries.is_empty());

    reaper.register(ResourceKind::Sandbox, Noop)?;
    reaper.shutdown_all(&ShutdownConfig::default()).await?;

    assert!(reaper.leak_report().entries.is_empty());
    Ok(())
}
