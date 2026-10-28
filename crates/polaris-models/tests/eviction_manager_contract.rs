use polaris_models::{
    EvictOutcome, EvictableSlot, EvictionManager, LoadError, ModelLoader, ModelsError, SlotConfig,
    SlotState,
};
use polaris_sdk::registry::MemoryPressure;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

#[derive(Debug, Clone)]
struct TestLoader {
    loads: Arc<AtomicUsize>,
    unloads: Arc<AtomicUsize>,
}

impl TestLoader {
    fn new() -> Self {
        Self {
            loads: Arc::new(AtomicUsize::new(0)),
            unloads: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl ModelLoader<usize> for TestLoader {
    async fn load(&self) -> Result<usize, LoadError> {
        let previous = self
            .loads
            .try_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value.checked_add(1)
            })
            .map_err(|_| LoadError::TaskFailed)?;
        previous.checked_add(1).ok_or(LoadError::TaskFailed)
    }

    async fn unload(&self, _model: &usize) -> Result<(), LoadError> {
        self.unloads.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn slot(loader: TestLoader) -> Result<EvictableSlot<usize, TestLoader>, ModelsError> {
    EvictableSlot::new(
        loader,
        SlotConfig {
            idle_ttl: Duration::ZERO,
            max_leases: 8,
            load_timeout: Duration::from_secs(1),
            unload_timeout: Duration::from_secs(1),
        },
    )
}

#[tokio::test]
async fn evict_all_orders_longest_idle_first() -> Result<(), Box<dyn std::error::Error>> {
    let oldest = slot(TestLoader::new())?;
    let middle = slot(TestLoader::new())?;
    let newest = slot(TestLoader::new())?;
    for model_slot in [&oldest, &middle, &newest] {
        let lease = model_slot.acquire().await?;
        drop(lease);
        tokio::time::sleep(Duration::from_millis(8)).await;
    }
    let manager = EvictionManager::new(vec![
        ("newest".to_owned(), newest),
        ("oldest".to_owned(), oldest),
        ("middle".to_owned(), middle),
    ])?;
    let reports = manager
        .evict_all(polaris_models::EvictReason::Idle, Duration::from_secs(1))
        .await;
    let names: Vec<_> = reports.iter().map(|report| report.name.as_str()).collect();
    assert_eq!(names, ["oldest", "middle", "newest"]);
    assert!(
        reports
            .iter()
            .all(|report| report.outcome == Ok(EvictOutcome::Evicted))
    );
    Ok(())
}

#[tokio::test]
async fn critical_pressure_drains_and_pauses_all_slots() -> Result<(), Box<dyn std::error::Error>> {
    let active = slot(TestLoader::new())?;
    let cold = slot(TestLoader::new())?;
    let lease = active.acquire().await?;
    let manager = EvictionManager::new(vec![
        ("active".to_owned(), active.clone()),
        ("cold".to_owned(), cold.clone()),
    ])?;
    let reports = manager
        .on_memory_pressure(MemoryPressure::Critical, Duration::from_millis(5))
        .await;
    assert_eq!(reports.len(), 2);
    assert!(matches!(active.acquire().await, Err(ModelsError::Draining)));
    assert!(matches!(active.state()?, SlotState::Draining { leases: 1 }));
    assert_eq!(cold.state()?, SlotState::Cold);
    drop(lease);
    tokio::time::timeout(Duration::from_secs(1), async {
        while active.state().ok() != Some(SlotState::Cold) {
            tokio::task::yield_now().await;
        }
    })
    .await?;
    let resumed = manager.resume_all();
    assert!(resumed.iter().all(|(_, result)| result.is_ok()));
    assert!(active.acquire().await.is_ok());
    Ok(())
}

#[tokio::test]
async fn elevated_pressure_only_evicts_idle_slots() -> Result<(), Box<dyn std::error::Error>> {
    let active = slot(TestLoader::new())?;
    let loaded = slot(TestLoader::new())?;
    let lease = active.acquire().await?;
    let loaded_lease = loaded.acquire().await?;
    drop(loaded_lease);
    let manager = EvictionManager::new(vec![
        ("active".to_owned(), active.clone()),
        ("loaded".to_owned(), loaded.clone()),
    ])?;
    let reports = manager
        .on_memory_pressure(MemoryPressure::Elevated, Duration::from_secs(1))
        .await;
    assert_eq!(reports.len(), 2);
    assert_eq!(active.state()?, SlotState::Active { leases: 1 });
    assert_eq!(loaded.state()?, SlotState::Cold);
    assert_eq!(
        reports
            .iter()
            .find(|report| report.name == "active")
            .map(|report| &report.outcome),
        Some(&Ok(EvictOutcome::NotIdle))
    );
    drop(lease);
    Ok(())
}

#[tokio::test]
async fn shutdown_requests_every_slot_even_after_budget() -> Result<(), Box<dyn std::error::Error>>
{
    let first_loader = TestLoader::new();
    let second_loader = TestLoader::new();
    let manager = EvictionManager::new(vec![
        ("first".to_owned(), slot(first_loader.clone())?),
        ("second".to_owned(), slot(second_loader.clone())?),
    ])?;
    let reports = manager
        .evict_all(polaris_models::EvictReason::Shutdown, Duration::ZERO)
        .await;
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|report| report.outcome == Ok(EvictOutcome::NotLoaded))
    );
    assert_eq!(first_loader.loads.load(Ordering::Acquire), 0);
    assert_eq!(second_loader.loads.load(Ordering::Acquire), 0);
    Ok(())
}

#[test]
fn invalid_or_duplicate_slot_names_are_rejected() -> Result<(), Box<dyn std::error::Error>> {
    assert!(EvictionManager::new(vec![(String::new(), slot(TestLoader::new())?)]).is_err());
    assert!(
        EvictionManager::new(vec![
            ("same".to_owned(), slot(TestLoader::new())?),
            ("same".to_owned(), slot(TestLoader::new())?),
        ])
        .is_err()
    );
    assert!(
        EvictionManager::new(vec![("bad\nname".to_owned(), slot(TestLoader::new())?)]).is_err()
    );
    let long_name = "a".repeat(129);
    assert!(EvictionManager::new(vec![(long_name, slot(TestLoader::new())?)]).is_err());
    Ok(())
}
