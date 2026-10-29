// Unit tests of crates/polaris-platform/crates/process/src/watch.rs (`tests`); compiled into that module via #[path].
use super::{RescanReason, collect_event, enqueue_event};
use notify::Event;
use notify::event::{EventAttributes, EventKind, Flag};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn backend_overflow_is_promoted_to_rescan() {
    let mut event = Event::new(EventKind::Other);
    let mut attributes = EventAttributes::new();
    attributes.set_flag(Flag::Rescan);
    event.attrs = attributes;
    let mut paths = Vec::new();
    let mut reason = None;
    collect_event(Ok(event), Path::new("."), &[], &mut paths, &mut reason);
    assert_eq!(reason, Some(RescanReason::BackendOverflow));
    assert!(paths.is_empty());
}

#[test]
fn full_bounded_callback_queue_sets_overflow_flag() {
    let (sender, receiver) = mpsc::sync_channel(1);
    let overflowed = AtomicBool::new(false);
    enqueue_event(&sender, &overflowed, Ok(Event::new(EventKind::Any)));
    enqueue_event(&sender, &overflowed, Ok(Event::new(EventKind::Any)));
    assert!(overflowed.load(Ordering::Acquire));
    assert!(receiver.recv_timeout(Duration::from_millis(1)).is_ok());
}

use std::path::Path;
