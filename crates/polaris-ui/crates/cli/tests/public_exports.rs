use std::error::Error;

use crossterm::event::Event;
use polaris_ui_cli::{input, presenter, scheduler, terminal};

#[test]
fn terminal_modules_are_usable_through_the_public_crate_api() -> Result<(), Box<dyn Error>> {
    let event = input::normalize_event(Event::FocusLost)?;
    assert_eq!(event, input::NormalizedInput::Focus { focused: false });

    let mut scheduler = scheduler::InputScheduler::new();
    scheduler.mark_dirty();
    assert!(scheduler.next_frame_due().is_some());

    let mut presenter = presenter::BufferedPresenter::new(Vec::<u8>::new());
    presenter.begin_frame()?;
    presenter.abort_frame()?;

    let _ops = terminal::CrosstermOps::default();
    Ok(())
}
