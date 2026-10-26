//! Shared editor geometry render coverage.

use std::error::Error;

use polaris_sdk::effort::Effort;
use polaris_sdk::ids::ModelId;
use polaris_ui_cli::screen::{RenderOptions, ScreenState, dump_cells};
use polaris_ui_model::composer::Composer;
use polaris_ui_model::view::ViewState;

#[test]
fn selected_text_is_painted_with_the_editor_selection_role() -> Result<(), Box<dyn Error>> {
    let model = ModelId::new("local:editor-selection")?;
    let view = ViewState::new(model, Effort::Low);
    let mut composer = Composer::new();
    composer.insert("hello world")?;
    composer.set_selection(0..5);
    let dump = dump_cells(
        &ScreenState::new(&view, &composer),
        &RenderOptions::default(),
        80,
        24,
    )?;
    let area = dump.metadata.composer_rect;
    let selected_index = usize::from(area.y.saturating_add(1))
        .saturating_mul(usize::from(dump.width))
        .saturating_add(usize::from(area.x.saturating_add(4)));
    let unselected_index = selected_index.saturating_add(6);
    assert_eq!(
        dump.cells.get(selected_index).and_then(|cell| cell.bg_rgb),
        Some((70, 70, 70))
    );
    assert_ne!(
        dump.cells
            .get(unselected_index)
            .and_then(|cell| cell.bg_rgb),
        Some((70, 70, 70))
    );
    Ok(())
}
