use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

use polaris_nebius_cloud_token_factory::{SseEventRef, SseParser};

struct CountingAllocator;

static COUNTING: AtomicBool = AtomicBool::new(false);
static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

// This test binary is isolated from the library unit-test process. Counting is enabled only
// around the parser call, after parser buffers and fixture input are already allocated.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegate the unchanged allocation request to the system allocator.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: pointer and layout originate from the paired system allocation above.
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if COUNTING.load(Ordering::Relaxed) {
            ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        }
        // SAFETY: delegate the unchanged reallocation request to the system allocator.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[test]
fn borrowed_frame_routing_allocates_nothing_after_parser_initialization() -> Result<(), String> {
    let mut parser = SseParser::new(1024, 8).map_err(|error| error.to_string())?;
    let first = b"data: {\"type\":\"response.created\"}\r\n\r\n";
    let second_prefix = b"data: \xce";
    let second_suffix = b"\xbb\xf0\x9f\x98\x80\n\n";
    let mut events = 0_usize;
    ALLOCATIONS.store(0, Ordering::SeqCst);
    COUNTING.store(true, Ordering::SeqCst);
    let result = parser.feed_with(first, |event| {
        if matches!(event, SseEventRef::Data(_) | SseEventRef::Done) {
            events = events.saturating_add(1);
        }
        Ok(())
    });
    let second_result = if result.is_ok() {
        parser.feed_with(second_prefix, |event| {
            if matches!(event, SseEventRef::Data(_) | SseEventRef::Done) {
                events = events.saturating_add(1);
            }
            Ok(())
        })
    } else {
        result
    };
    let third_result = if second_result.is_ok() {
        parser.feed_with(second_suffix, |event| {
            if matches!(event, SseEventRef::Data(_) | SseEventRef::Done) {
                events = events.saturating_add(1);
            }
            Ok(())
        })
    } else {
        second_result
    };
    COUNTING.store(false, Ordering::SeqCst);
    third_result.map_err(|error| error.to_string())?;
    let allocations = ALLOCATIONS.load(Ordering::SeqCst);
    if events != 2 || allocations != 0 {
        return Err(format!("events={events}, allocations={allocations}"));
    }
    Ok(())
}
