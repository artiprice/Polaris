//! Outcome Accounting (descriptor: specs/descriptors/reaper_outcome_accounting.json).

use std::cmp::Reverse;
use std::collections::BTreeSet;
use std::time::Duration;

use polaris_kernel_reaper::{
    Disposition, Outcome, ReapId, Reaper, ResourceKind, ShutdownConfig, Teardown, TeardownContext,
    TeardownError, TeardownReason,
};
use proptest::prelude::*;
use proptest::sample::select;

#[derive(Debug, Clone, Copy)]
enum Behaviour {
    Ok,
    Fail,
    ForcedDisposition,
    HangThenForceOk,
    HangForever,
}

impl Behaviour {
    fn expected(self) -> Outcome {
        match self {
            Self::Ok => Outcome::Released,
            Self::Fail => Outcome::Failed("scripted failure".to_owned()),
            Self::ForcedDisposition | Self::HangThenForceOk => Outcome::Forced,
            Self::HangForever => Outcome::Abandoned,
        }
    }
}

struct Scripted(Behaviour);

impl Teardown for Scripted {
    async fn graceful(&mut self, _ctx: TeardownContext) -> Result<Disposition, TeardownError> {
        match self.0 {
            Behaviour::Ok => Ok(Disposition::Clean),
            Behaviour::Fail => Err(TeardownError::Other("scripted failure".to_owned())),
            Behaviour::ForcedDisposition => Ok(Disposition::Forced),
            Behaviour::HangThenForceOk | Behaviour::HangForever => {
                std::future::pending::<Result<Disposition, TeardownError>>().await
            }
        }
    }

    async fn force(&mut self, _ctx: TeardownContext) -> Result<(), TeardownError> {
        match self.0 {
            Behaviour::HangThenForceOk => Ok(()),
            _ => std::future::pending::<Result<(), TeardownError>>().await,
        }
    }
}

fn behaviour() -> impl Strategy<Value = Behaviour> {
    select(vec![
        Behaviour::Ok,
        Behaviour::Fail,
        Behaviour::ForcedDisposition,
        Behaviour::HangThenForceOk,
        Behaviour::HangForever,
    ])
}

fn kind() -> impl Strategy<Value = ResourceKind> {
    select(ResourceKind::ALL.to_vec())
}

fn fail(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}

proptest! {
    #[test]
    fn prop_every_resource_is_accounted_for_exactly_once(
        resources in proptest::collection::vec((kind(), behaviour()), 0..40),
    ) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .map_err(fail)?;
        let dir = tempfile::tempdir().map_err(fail)?;
        let reaper = Reaper::new(dir.path()).map_err(fail)?;
        let mut registered: Vec<(ReapId, ResourceKind, Behaviour)> = Vec::new();
        for (kind, behaviour) in &resources {
            let id = reaper.register(*kind, Scripted(*behaviour)).map_err(fail)?;
            registered.push((id, *kind, *behaviour));
        }
        let config = ShutdownConfig {
            reason: TeardownReason::Shutdown,
            graceful: Duration::from_secs(1),
            forced: Duration::from_secs(1),
            overall: Duration::from_secs(24 * 3600),
        };

        let report = runtime.block_on(reaper.shutdown_all(&config)).map_err(fail)?;

        // Exactly once each.
        prop_assert_eq!(report.entries.len(), registered.len());
        let reported: BTreeSet<ReapId> = report.entries.iter().map(|e| e.id).collect();
        let expected_ids: BTreeSet<ReapId> = registered.iter().map(|(id, _, _)| *id).collect();
        prop_assert_eq!(reported, expected_ids);

        // In teardown order: kind priority, then newest first.
        let mut order = registered.clone();
        order.sort_by_key(|(id, kind, _)| (*kind, Reverse(*id)));
        let reported_order: Vec<ReapId> = report.entries.iter().map(|e| e.id).collect();
        let expected_order: Vec<ReapId> = order.iter().map(|(id, _, _)| *id).collect();
        prop_assert_eq!(reported_order, expected_order);

        // Each outcome is the one its behaviour implies.
        for (id, kind, behaviour) in &registered {
            let entry = report.entries.iter().find(|e| e.id == *id);
            prop_assert_eq!(entry.map(|e| e.kind), Some(*kind));
            prop_assert_eq!(entry.map(|e| e.outcome.clone()), Some(behaviour.expected()));
        }

        // Unresolved resources (Failed, Abandoned) are exactly the leak report.
        let unresolved: Vec<ReapId> = registered
            .iter()
            .filter(|(_, _, b)| matches!(b, Behaviour::Fail | Behaviour::HangForever))
            .map(|(id, _, _)| *id)
            .collect();
        let leaked: Vec<ReapId> = reaper.leak_report().entries.iter().map(|e| e.id).collect();
        prop_assert_eq!(leaked, unresolved);
        prop_assert!(!report.deadline_exceeded);
    }
}
