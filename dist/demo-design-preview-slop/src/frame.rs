//! Demand-driven frame deadlines and ordered semantic input batching.
use crossterm::event::{KeyEvent, MouseEvent, MouseEventKind};
use std::{
    cell::Cell,
    collections::VecDeque,
    time::{Duration, Instant},
};
pub const FRAME_INTERVAL: Duration = Duration::from_nanos(16_666_667);
pub struct Scheduler {
    dirty: bool,
    next: Instant,
}
impl Scheduler {
    pub fn new(now: Instant) -> Self {
        Self {
            dirty: true,
            next: now,
        }
    }
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn due(&self, now: Instant) -> bool {
        self.dirty && now >= self.next
    }
    pub fn wait(&self, now: Instant, other: Duration) -> Duration {
        if self.dirty {
            other.min(self.next.saturating_duration_since(now))
        } else {
            other
        }
    }
    pub fn presented(&mut self, now: Instant) {
        self.dirty = false;
        self.next = now + FRAME_INTERVAL;
    }
}
#[derive(Debug)]
pub enum UiEvent {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    Resize,
    Notice(String),
}
#[derive(Default)]
pub struct EventQueue {
    events: VecDeque<UiEvent>,
}
impl EventQueue {
    pub fn push(&mut self, event: UiEvent) {
        if matches!((&event,self.events.back()),(UiEvent::Mouse(a),Some(UiEvent::Mouse(b))) if a.kind==MouseEventKind::Moved && b.kind==MouseEventKind::Moved)
            || matches!(
                (&event, self.events.back()),
                (UiEvent::Resize, Some(UiEvent::Resize))
            )
        {
            self.events.pop_back();
        }
        self.events.push_back(event);
    }
    pub fn pop(&mut self) -> Option<UiEvent> {
        self.events.pop_front()
    }
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn len(&self) -> usize {
        self.events.len()
    }
}
thread_local! {static TIME:Cell<Option<Instant>>=const{Cell::new(None)};}
pub fn now() -> Instant {
    TIME.with(|t| t.get().unwrap_or_else(Instant::now))
}
pub struct FrameTime(Option<Instant>);
impl FrameTime {
    pub fn capture() -> Self {
        Self(TIME.with(|t| {
            let old = t.get();
            if old.is_none() {
                t.set(Some(Instant::now()));
            }
            old
        }))
    }
}
impl Drop for FrameTime {
    fn drop(&mut self) {
        TIME.with(|t| t.set(self.0));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyModifiers, MouseButton};
    #[test]
    fn deadlines_cap_frames_without_idle_work_or_starvation() {
        let start = Instant::now();
        let mut s = Scheduler::new(start);
        let mut frames = 0;
        for ms in 0..1000 {
            let t = start + Duration::from_millis(ms);
            s.invalidate();
            if s.due(t) {
                frames += 1;
                s.presented(t);
            }
        }
        assert!(frames <= 60);
        assert!(frames >= 58);
        let t = start + Duration::from_secs(2);
        assert!(s.due(t), "the final burst still has a pending update");
        s.presented(t);
        assert!(!s.due(t + Duration::from_secs(1)));
        s.invalidate();
        assert!(s.due(t + Duration::from_secs(1)));
        s.presented(t);
        assert_eq!(s.wait(t, Duration::from_secs(1)), Duration::from_secs(1));
    }
    #[test]
    fn coalescing_respects_key_paste_click_drag_and_release_boundaries() {
        let mut q = EventQueue::default();
        let mouse = |kind, x| {
            UiEvent::Mouse(MouseEvent {
                kind,
                column: x,
                row: 1,
                modifiers: KeyModifiers::NONE,
            })
        };
        q.push(mouse(MouseEventKind::Moved, 1));
        q.push(mouse(MouseEventKind::Moved, 2));
        q.push(UiEvent::Key(KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::NONE,
        )));
        q.push(mouse(MouseEventKind::Down(MouseButton::Left), 2));
        q.push(mouse(MouseEventKind::Drag(MouseButton::Left), 3));
        q.push(mouse(MouseEventKind::Drag(MouseButton::Left), 4));
        q.push(mouse(MouseEventKind::Up(MouseButton::Left), 4));
        q.push(UiEvent::Paste("p".into()));
        q.push(UiEvent::Resize);
        q.push(UiEvent::Resize);
        assert_eq!(q.len(), 8);
        assert!(matches!(
            q.pop(),
            Some(UiEvent::Mouse(MouseEvent { column: 2, .. }))
        ));
        assert!(matches!(q.pop(), Some(UiEvent::Key(_))));
        assert!(matches!(
            q.pop(),
            Some(UiEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(_),
                ..
            }))
        ));
        assert!(matches!(
            q.pop(),
            Some(UiEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(_),
                column: 3,
                ..
            }))
        ));
        assert!(matches!(
            q.pop(),
            Some(UiEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(_),
                column: 4,
                ..
            }))
        ));
        assert!(matches!(
            q.pop(),
            Some(UiEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Up(_),
                ..
            }))
        ));
        assert!(matches!(q.pop(), Some(UiEvent::Paste(_))));
        assert!(matches!(q.pop(), Some(UiEvent::Resize)));
    }
}
