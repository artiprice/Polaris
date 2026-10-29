//! Editor geometry regression coverage.

use polaris_ui_cli::editor::EditorGeometry;

#[test]
fn tabs_wide_graphemes_and_exact_width_boundaries_map_to_cells() {
    let geometry = EditorGeometry::new("a\t界e\u{301}", &[], 6);
    assert_eq!(geometry.caret_position(2).column, 4);
    assert_eq!(geometry.caret_position(5).row, 1);
    assert_eq!(geometry.caret_position(5).column, 0);
    assert_eq!(geometry.rows().first().map(|row| row.cells), Some(6));
    assert_eq!(
        geometry.rows().first().map(|row| row.display_text.as_str()),
        Some("a   界")
    );

    let exact = EditorGeometry::new("xy", &[], 2);
    assert_eq!(exact.caret_position(2).row, 1);
    assert_eq!(exact.caret_position(2).column, 0);
}

#[test]
fn source_cell_lookup_is_exact_and_preserves_atomic_display_spans() {
    let geometry = EditorGeometry::new("a\t界e\u{301}", &[], 12);
    assert_eq!(geometry.source_at_cell(0, 0), Some(0..1));
    assert_eq!(geometry.source_at_cell(0, 2), Some(1..2));
    assert_eq!(geometry.source_at_cell(0, 4), Some(2..5));
    assert_eq!(geometry.source_at_cell(0, 5), Some(2..5));
    assert_eq!(geometry.source_at_cell(0, 11), None);
    assert_eq!(geometry.source_at_cell(1, 0), None);

    let chip_text = "x[Image 1]y";
    let chip_range = 1.."x[Image 1]".len();
    let chip_geometry = EditorGeometry::new(chip_text, &[chip_range], 20);
    assert_eq!(chip_geometry.source_at_cell(0, 2), Some(2..3));
    assert_eq!(chip_geometry.source_at_cell(0, 20), None);
}

#[test]
fn explicit_newlines_zwj_graphemes_and_mouse_stops_are_stable() {
    let family = "👨\u{200d}👩\u{200d}👧";
    let text = format!("a\n{family}z");
    let geometry = EditorGeometry::new(&text, &[], 4);
    assert_eq!(geometry.caret_position(2).row, 1);
    assert_eq!(geometry.caret_position(2 + family.len()).column, 2);

    let chip_label = "[Image 1]";
    let chip_text = format!("x{chip_label}y");
    let chip = 1..1 + chip_label.len();
    let chip_geometry = EditorGeometry::new(&chip_text, &[chip.clone()], 3);
    let clicked = chip_geometry.cursor_at_cell(1, 1);
    assert!(clicked == chip.start || clicked == chip.end);
    let clicked_later = chip_geometry.cursor_at_cell(2, 1);
    assert!(clicked_later == chip.start || clicked_later == chip.end);
}

#[test]
fn vertical_motion_keeps_sticky_column_and_viewport_tracks_cursor() {
    let text = "abcdef\nx\nuvwxyz";
    let geometry = EditorGeometry::new(text, &[], 20);
    let (middle, desired) = geometry.vertical_target(6, 1, None);
    assert_eq!(middle, 8);
    assert_eq!(desired, 6);
    let (last, retained) = geometry.vertical_target(middle, 1, Some(desired));
    assert_eq!(last, 15);
    assert_eq!(retained, 6);
    assert_eq!(geometry.visible_rows(2, 2), 1..3);
}
