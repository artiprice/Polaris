use polaris_models::{
    EvictOutcome, EvictableSlot, LoadError, ModelLoader, ModelsError, SlotConfig, SlotState,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::Semaphore;

#[derive(Debug, Clone)]
struct TestLoader {
    loads: Arc<AtomicUsize>,
    unloads: Arc<AtomicUsize>,
    load_started: Arc<Semaphore>,
    load_release: Arc<Semaphore>,
    unload_started: Arc<Semaphore>,
    unload_release: Arc<Semaphore>,
    fail_unloads: Arc<AtomicUsize>,
}

impl TestLoader {
    fn new() -> Self {
        Self {
            loads: Arc::new(AtomicUsize::new(0)),
            unloads: Arc::new(AtomicUsize::new(0)),
            load_started: Arc::new(Semaphore::new(0)),
            load_release: Arc::new(Semaphore::new(1024)),
            unload_started: Arc::new(Semaphore::new(0)),
            unload_release: Arc::new(Semaphore::new(1024)),
            fail_unloads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn gated() -> Self {
        let loader = Self::new();
        Self {
            load_release: Arc::new(Semaphore::new(0)),
            unload_release: Arc::new(Semaphore::new(0)),
            ..loader
        }
    }

    fn release_load(&self) {
        self.load_release.add_permits(1);
    }

    fn release_unload(&self) {
        self.unload_release.add_permits(1);
    }

    fn fail_next_unload(&self) {
        self.fail_unloads.fetch_add(1, Ordering::AcqRel);
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
        let load_number = previous.checked_add(1).ok_or(LoadError::TaskFailed)?;
        self.load_started.add_permits(1);
        let permit = self
            .load_release
            .acquire()
            .await
            .map_err(|_| LoadError::TaskFailed)?;
        permit.forget();
        Ok(load_number)
    }

    async fn unload(&self, _model: &usize) -> Result<(), LoadError> {
        self.unloads.fetch_add(1, Ordering::AcqRel);
        self.unload_started.add_permits(1);
        let permit = self
            .unload_release
            .acquire()
            .await
            .map_err(|_| LoadError::TaskFailed)?;
        permit.forget();
        if self
            .fail_unloads
            .try_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(1)
            })
            .is_ok()
        {
            return Err(LoadError::Unavailable);
        }
        Ok(())
    }
}

fn config(max_leases: u32, idle_ttl: Duration, unload_timeout: Duration) -> SlotConfig {
    SlotConfig {
        idle_ttl,
        max_leases,
        load_timeout: Duration::from_secs(2),
        unload_timeout,
    }
}

async fn started(gate: &Arc<Semaphore>) -> Result<(), Box<dyn std::error::Error>> {
    let permit = tokio::time::timeout(Duration::from_secs(2), gate.acquire()).await??;
    permit.forget();
    Ok(())
}

async fn yield_until_state<M, L>(
    slot: &EvictableSlot<M, L>,
    expected: SlotState,
) -> Result<(), Box<dyn std::error::Error>>
where
    M: Send + Sync + 'static,
    L: ModelLoader<M>,
{
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if slot.state().ok() == Some(expected) {
                return;
            }
            tokio::task::yield_now().await;
        }
    })
    .await?;
    Ok(())
}

#[tokio::test]
async fn concurrent_acquire_loads_once() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::gated();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(8, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let first_slot = slot.clone();
    let first = tokio::spawn(async move { first_slot.acquire().await });
    started(&loader.load_started).await?;
    let second_slot = slot.clone();
    let second = tokio::spawn(async move { second_slot.acquire().await });
    tokio::task::yield_now().await;
    assert_eq!(loader.loads.load(Ordering::Acquire), 1);
    loader.release_load();
    let first_lease = first.await??;
    let second_lease = second.await??;
    assert_eq!(first_lease.model()?, second_lease.model()?);
    assert_eq!(loader.loads.load(Ordering::Acquire), 1);
    drop((first_lease, second_lease));
    Ok(())
}

#[tokio::test]
async fn canceled_waiter_keeps_owner_load_alive() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::gated();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(8, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let canceled_slot = slot.clone();
    let canceled = tokio::spawn(async move { canceled_slot.acquire().await });
    started(&loader.load_started).await?;
    canceled.abort();
    let survivor_slot = slot.clone();
    let survivor = tokio::spawn(async move { survivor_slot.acquire().await });
    loader.release_load();
    let lease = survivor.await??;
    assert_eq!(*lease.model()?, 1);
    assert_eq!(loader.loads.load(Ordering::Acquire), 1);
    drop(lease);
    Ok(())
}

#[tokio::test]
async fn lease_bound_rejects_excess_acquisitions() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::new();
    let slot = EvictableSlot::new(
        loader,
        config(1, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let lease = slot.acquire().await?;
    assert!(matches!(slot.acquire().await, Err(ModelsError::LeaseLimit)));
    drop(lease);
    let next = slot.acquire().await?;
    drop(next);
    Ok(())
}

#[tokio::test]
async fn force_evict_deadline_pauses_then_resume_reopens() -> Result<(), Box<dyn std::error::Error>>
{
    let loader = TestLoader::new();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(2, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let lease = slot.acquire().await?;
    assert_eq!(
        slot.force_evict(Duration::from_millis(5)).await?,
        EvictOutcome::StillInUse { leases: 1 }
    );
    assert!(matches!(slot.acquire().await, Err(ModelsError::Draining)));
    drop(lease);
    yield_until_state(&slot, SlotState::Cold).await?;
    assert_eq!(loader.unloads.load(Ordering::Acquire), 1);
    slot.resume()?;
    let lease = slot.acquire().await?;
    drop(lease);
    Ok(())
}

#[tokio::test]
async fn eviction_during_load_drains_the_completed_load() -> Result<(), Box<dyn std::error::Error>>
{
    let loader = TestLoader::gated();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(2, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let acquire_slot = slot.clone();
    let acquire = tokio::spawn(async move { acquire_slot.acquire().await });
    started(&loader.load_started).await?;
    assert!(matches!(
        slot.force_evict(Duration::from_millis(5)).await?,
        EvictOutcome::Pending {
            state: SlotState::Loading
        }
    ));
    loader.release_load();
    started(&loader.unload_started).await?;
    loader.release_unload();
    assert!(matches!(acquire.await?, Err(ModelsError::Draining)));
    yield_until_state(&slot, SlotState::Cold).await?;
    assert_eq!(loader.loads.load(Ordering::Acquire), 1);
    assert_eq!(loader.unloads.load(Ordering::Acquire), 1);
    Ok(())
}

#[tokio::test]
async fn idle_ttl_evicts_and_reload_uses_real_loader() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::new();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(4, Duration::from_millis(15), Duration::from_secs(1)),
    )?;
    let lease = slot.acquire().await?;
    drop(lease);
    assert_eq!(slot.evict_if_idle().await?, EvictOutcome::NotIdle);
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(slot.evict_if_idle().await?, EvictOutcome::Evicted);
    let lease = slot.acquire().await?;
    assert_eq!(*lease.model()?, 2);
    assert_eq!(loader.loads.load(Ordering::Acquire), 2);
    drop(lease);
    Ok(())
}

#[tokio::test]
async fn close_and_unload_failure_are_reported() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::new();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(4, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let lease = slot.acquire().await?;
    drop(lease);
    loader.fail_next_unload();
    assert_eq!(
        slot.close(Duration::from_secs(1)).await?,
        EvictOutcome::Failed(LoadError::Unavailable)
    );
    assert!(matches!(slot.acquire().await, Err(ModelsError::Closed)));
    assert_eq!(
        slot.force_evict(Duration::from_secs(1)).await?,
        EvictOutcome::Evicted
    );
    assert_eq!(slot.state()?, SlotState::Closed);

    let closing = EvictableSlot::new(
        TestLoader::new(),
        config(2, Duration::from_secs(5), Duration::from_secs(1)),
    )?;
    let lease = closing.acquire().await?;
    assert_eq!(
        closing.close(Duration::from_millis(5)).await?,
        EvictOutcome::StillInUse { leases: 1 }
    );
    assert!(matches!(closing.acquire().await, Err(ModelsError::Closed)));
    drop(lease);
    yield_until_state(&closing, SlotState::Closed).await?;
    Ok(())
}

#[tokio::test]
async fn unload_timeout_keeps_shared_drain_owned() -> Result<(), Box<dyn std::error::Error>> {
    let loader = TestLoader::gated();
    loader.release_load();
    let slot = EvictableSlot::new(
        loader.clone(),
        config(2, Duration::from_secs(5), Duration::from_secs(2)),
    )?;
    let lease = slot.acquire().await?;
    drop(lease);
    let evict_slot = slot.clone();
    let eviction =
        tokio::spawn(async move { evict_slot.force_evict(Duration::from_millis(10)).await });
    started(&loader.unload_started).await?;
    assert!(matches!(
        eviction.await??,
        EvictOutcome::Pending {
            state: SlotState::Unloading
        }
    ));
    loader.release_unload();
    yield_until_state(&slot, SlotState::Cold).await?;
    assert_eq!(loader.unloads.load(Ordering::Acquire), 1);
    Ok(())
}
