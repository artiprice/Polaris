//! Loom-friendly model of the bus ordering lock and shutdown boundary.

use loom::sync::{Arc, Condvar, Mutex, MutexGuard};
use loom::thread;
use std::sync::PoisonError;

#[derive(Default)]
struct State {
    persisted: Vec<u64>,
    published: Vec<u64>,
    closed: bool,
    active_publishers: u8,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(error) => PoisonError::into_inner(error),
    }
}

fn wait<'a, T>(condition: &Condvar, guard: MutexGuard<'a, T>) -> MutexGuard<'a, T> {
    match condition.wait(guard) {
        Ok(guard) => guard,
        Err(error) => PoisonError::into_inner(error),
    }
}

#[test]
fn publish_subscribe_and_close_keep_durable_prefix_order() {
    let mut model = loom::model::Builder::new();
    model.max_branches = 10_000;
    model.preemption_bound = Some(2);
    model.check(|| {
        let state = Arc::new((
            Mutex::new(State {
                active_publishers: 2,
                ..State::default()
            }),
            Condvar::new(),
        ));
        let reader_state = Arc::clone(&state);
        let reader = thread::spawn(move || {
            let (mutex, condition) = &*reader_state;
            let mut state = lock(mutex);
            while !state.closed || state.active_publishers != 0 {
                state = wait(condition, state);
            }
            assert!(state.published.len() <= state.persisted.len());
            state.published.clone()
        });

        let mut publishers = Vec::new();
        for _ in 0..2 {
            let publisher_state = Arc::clone(&state);
            publishers.push(thread::spawn(move || {
                for _ in 0..1 {
                    let (mutex, _) = &*publisher_state;
                    let mut state = lock(mutex);
                    if state.closed {
                        break;
                    }
                    let next = match u64::try_from(state.persisted.len()) {
                        Ok(previous) => previous.saturating_add(1),
                        Err(_) => break,
                    };
                    state.persisted.push(next);
                    state.published.push(next);
                }
                let (mutex, condition) = &*publisher_state;
                let mut state = lock(mutex);
                state.active_publishers = state.active_publishers.saturating_sub(1);
                if state.active_publishers == 0 {
                    condition.notify_all();
                }
            }));
        }
        let closer_state = Arc::clone(&state);
        let closer = thread::spawn(move || {
            let (mutex, condition) = &*closer_state;
            let mut state = lock(mutex);
            state.closed = true;
            while state.active_publishers != 0 {
                state = wait(condition, state);
            }
            condition.notify_all();
        });

        for publisher in publishers {
            assert!(publisher.join().is_ok());
        }
        assert!(closer.join().is_ok());
        let reader_result = reader.join();
        assert!(reader_result.is_ok());
        let observed = match reader_result {
            Ok(observed) => observed,
            Err(_) => Vec::new(),
        };
        let (mutex, _) = &*state;
        let state = lock(mutex);
        assert_eq!(state.persisted, state.published);
        assert_eq!(observed, state.published);
    });
}
