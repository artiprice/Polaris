use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use polaris_ui_cli::{
    input::{self, NormalizedInput},
    scheduler::{InputScheduler, SchedulerError},
};
use std::{error::Error, time::Duration};
type TestResult = Result<(), Box<dyn Error>>;

fn key(code: char) -> Result<NormalizedInput, input::InputError> {
    input::normalize_event(Event::Key(KeyEvent::new(
        KeyCode::Char(code),
        KeyModifiers::NONE,
    )))
}

#[test]
fn preserves_sequence_across_reserved_cancel_lane() -> TestResult {
    let mut scheduler = InputScheduler::new();
    scheduler.enqueue(key('a')?)?;
    scheduler.enqueue(input::normalize_event(Event::Key(KeyEvent::new(
        KeyCode::Esc,
        KeyModifiers::NONE,
    )))?)?;
    scheduler.enqueue(key('b')?)?;
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    assert!(
        matches!(batch.next(Duration::ZERO), Some(NormalizedInput::Key(k)) if k.code == KeyCode::Char('a'))
    );
    assert!(
        matches!(batch.next(Duration::ZERO), Some(NormalizedInput::Key(k)) if k.code == KeyCode::Esc)
    );
    assert!(
        matches!(batch.next(Duration::ZERO), Some(NormalizedInput::Key(k)) if k.code == KeyCode::Char('b'))
    );
    Ok(())
}

#[test]
fn enforces_record_payload_and_batch_limits() -> TestResult {
    let mut scheduler = InputScheduler::new();
    for _ in 0..1024 {
        scheduler.enqueue(key('x')?)?;
    }
    assert_eq!(
        scheduler.enqueue(key('x')?).err(),
        Some(SchedulerError::QueueFull)
    );
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    let mut count = 0;
    while batch.next(Duration::ZERO).is_some() {
        count += 1;
    }
    assert_eq!(count, 256);
    Ok(())
}

#[test]
fn schedules_dirty_frames_at_sixty_fps_without_idle_wakeups() -> TestResult {
    let mut scheduler = InputScheduler::new();
    assert_eq!(scheduler.next_frame_due(), None);
    scheduler.mark_dirty();
    assert_eq!(scheduler.next_frame_due(), Some(Duration::ZERO));
    assert!(scheduler.frame_due(Duration::ZERO));
    scheduler.mark_presented(Duration::ZERO);
    assert_eq!(scheduler.next_frame_due(), None);
    scheduler.mark_dirty();
    assert!(!scheduler.frame_due(Duration::from_millis(10)));
    Ok(())
}

#[test]
fn reserved_cancel_queue_preserves_cross_lane_order_and_works_when_regular_queue_full() -> TestResult
{
    let mut scheduler = InputScheduler::new();
    for _ in 0..1024 {
        scheduler.enqueue(key('a')?)?;
    }
    let cancel =
        input::normalize_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))?;
    scheduler.enqueue(cancel)?;
    assert_eq!(
        scheduler.enqueue(key('z')?).err(),
        Some(SchedulerError::QueueFull)
    );
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    for _ in 0..256 {
        assert!(batch.next(Duration::ZERO).is_some());
    }
    assert!(scheduler.pending() > 0);
    Ok(())
}

#[test]
fn payload_budget_and_batch_deadline_are_enforced() -> TestResult {
    let mut scheduler = InputScheduler::new();
    scheduler.enqueue(input::normalize_event(Event::Paste(
        "x".repeat(1024 * 1024),
    ))?)?;
    scheduler.enqueue(input::normalize_event(Event::Paste(
        "y".repeat(1024 * 1024),
    ))?)?;
    assert_eq!(
        scheduler
            .enqueue(input::normalize_event(Event::Paste("z".to_owned()))?)
            .err(),
        Some(SchedulerError::PayloadFull)
    );
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    assert!(batch.next(Duration::ZERO).is_some());
    assert_eq!(batch.next(Duration::from_millis(4)), None);
    Ok(())
}

#[test]
fn coalesces_adjacent_resize_events_to_latest_dimensions() -> TestResult {
    let mut scheduler = InputScheduler::new();
    scheduler.enqueue(input::normalize_event(Event::Resize(80, 24))?)?;
    scheduler.enqueue(input::normalize_event(Event::Resize(100, 40))?)?;
    assert_eq!(scheduler.pending(), 1);
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    assert!(matches!(
        batch.next(Duration::ZERO),
        Some(NormalizedInput::Resize {
            columns: 100,
            rows: 40
        })
    ));
    assert_eq!(batch.next(Duration::ZERO), None);
    Ok(())
}

#[test]
fn coalesces_only_adjacent_mouse_moves_without_crossing_semantic_barriers() -> TestResult {
    let moved = |column, row| {
        input::normalize_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }))
    };
    let mut scheduler = InputScheduler::new();
    scheduler.enqueue(moved(1, 1)?)?;
    scheduler.enqueue(moved(4, 5)?)?;
    scheduler.enqueue(input::normalize_event(Event::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 4,
        row: 5,
        modifiers: KeyModifiers::NONE,
    }))?)?;
    scheduler.enqueue(moved(6, 7)?)?;
    assert_eq!(scheduler.pending(), 3);
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    assert!(matches!(
        batch.next(Duration::ZERO),
        Some(NormalizedInput::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 4,
            row: 5,
            ..
        }))
    ));
    assert!(matches!(
        batch.next(Duration::ZERO),
        Some(NormalizedInput::Mouse(MouseEvent {
            kind: MouseEventKind::Down(_),
            ..
        }))
    ));
    assert!(matches!(
        batch.next(Duration::ZERO),
        Some(NormalizedInput::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 6,
            row: 7,
            ..
        }))
    ));
    Ok(())
}

#[test]
fn every_semantic_and_cancel_kind_breaks_mouse_move_coalescing() -> TestResult {
    let moved = |column| {
        input::normalize_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }))
    };
    let events = vec![
        moved(1)?,
        input::normalize_event(Event::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )))?,
        moved(2)?,
        input::normalize_event(Event::Paste("p".to_owned()))?,
        moved(3)?,
        input::normalize_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 3,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }))?,
        moved(4)?,
        input::normalize_event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: 4,
            row: 1,
            modifiers: KeyModifiers::NONE,
        }))?,
        moved(5)?,
        input::normalize_event(Event::Resize(90, 30))?,
        moved(6)?,
        input::normalize_event(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)))?,
        moved(7)?,
    ];
    let mut scheduler = InputScheduler::new();
    for event in events {
        scheduler.enqueue(event)?;
    }
    assert_eq!(scheduler.pending(), 13);
    let mut batch = scheduler.begin_batch(Duration::ZERO);
    for _ in 0..13 {
        assert!(batch.next(Duration::ZERO).is_some());
    }
    assert_eq!(batch.next(Duration::ZERO), None);
    Ok(())
}
