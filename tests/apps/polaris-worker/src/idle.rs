// Unit tests of apps/polaris-worker/src/idle.rs (`tests`); compiled into that module via #[path].
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{IdleSession, S1IdlePolicy};

struct ObservedDrop(Arc<AtomicBool>);

impl Drop for ObservedDrop {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Release);
    }
}

#[test]
fn eviction_acknowledges_completed_destruction_and_is_idempotent()
-> Result<(), Box<dyn std::error::Error>> {
    let dropped = Arc::new(AtomicBool::new(false));
    let mut session = IdleSession::new();
    let _model = session.get_or_load(|| Ok(ObservedDrop(Arc::clone(&dropped))))?;
    assert!(session.evict(), "a loaded model must be reported");
    assert!(
        dropped.load(Ordering::Acquire),
        "drop must precede acknowledgement"
    );
    assert!(!session.evict(), "a second eviction must report no model");
    assert!(
        session.deadline().is_none(),
        "no deadline for a cold session"
    );
    Ok(())
}

#[test]
fn only_completed_work_moves_the_idle_deadline() -> Result<(), Box<dyn std::error::Error>> {
    let now = Instant::now();
    let policy = S1IdlePolicy::new(Duration::from_millis(10))?;
    let mut session = IdleSession::new();
    let _model = session.get_or_load(|| Ok(()))?;
    session.completed_work(now, policy)?;
    let deadline = session.deadline().ok_or("missing loaded deadline")?;
    let just_before = deadline
        .checked_sub(Duration::from_nanos(1))
        .ok_or("clock")?;
    assert!(
        !session.expire(just_before),
        "must not evict before deadline"
    );
    assert_eq!(
        session.deadline(),
        Some(deadline),
        "observation is not activity"
    );
    assert!(session.expire(deadline), "deadline is inclusive");
    assert!(!session.expire(deadline), "expired session is already cold");
    let _reloaded = session.get_or_load(|| Ok(()))?;
    session.completed_work(deadline, policy)?;
    assert!(session.deadline().is_some_and(|next| next > deadline));
    Ok(())
}

#[test]
fn failed_loading_keeps_the_session_cold() -> Result<(), Box<dyn std::error::Error>> {
    let mut session = IdleSession::<()>::new();
    assert!(
        session
            .get_or_load(|| Err("fixture load failure".to_owned()))
            .is_err()
    );
    session.completed_work(Instant::now(), S1IdlePolicy::default())?;
    assert!(session.deadline().is_none());
    assert!(!session.evict());
    Ok(())
}
