//! Cockpit State (descriptor: specs/descriptors/uimodel_cockpit_state.json).

use polaris_ui_model::cockpit::{
    CockpitFocus, CockpitState, Panel, PanelPresentation, ViewportClass,
};

#[test]
fn default_is_main_focused_with_panel_closed() {
    let state = CockpitState::default();
    assert!(!state.is_open());
    assert!(!state.is_pinned());
    assert_eq!(state.focus(), CockpitFocus::Main);
    assert_eq!(state.presentation(), PanelPresentation::Closed);
}
#[test]
fn toggle_selected_panel_and_focus_cycle() {
    let mut state = CockpitState::default();
    state.select(Panel::Agents);
    state.toggle_panel(ViewportClass::Wide);
    assert_eq!(state.selected(), Panel::Agents);
    assert_eq!(state.presentation(), PanelPresentation::Sidebar);
    state.cycle_focus();
    assert_eq!(state.focus(), CockpitFocus::Panel);
    state.cycle_focus();
    assert_eq!(state.focus(), CockpitFocus::Main);
    state.toggle_panel(ViewportClass::Wide);
    assert_eq!(state.focus(), CockpitFocus::Main);
}
#[test]
fn resize_collapses_and_restores_without_focus_theft() {
    let mut state = CockpitState::default();
    state.set_pinned(true);
    state.toggle_panel(ViewportClass::Wide);
    state.set_viewport(ViewportClass::Narrow);
    assert_eq!(state.presentation(), PanelPresentation::Indicator);
    assert_eq!(state.focus(), CockpitFocus::Main);
    state.set_viewport(ViewportClass::Wide);
    assert_eq!(state.presentation(), PanelPresentation::Sidebar);
    assert_eq!(state.focus(), CockpitFocus::Main);
}
#[test]
fn focused_panel_becomes_sheet_and_small_viewport_notice() {
    let mut state = CockpitState::default();
    state.toggle_panel(ViewportClass::Wide);
    state.cycle_focus();
    state.set_viewport(ViewportClass::Narrow);
    assert_eq!(state.presentation(), PanelPresentation::Sheet);
    state.set_viewport(ViewportClass::TooSmall);
    assert_eq!(state.presentation(), PanelPresentation::ResizeNotice);
}
#[test]
fn panel_focus_never_changes_main_target() {
    let mut state = CockpitState::default();
    state.toggle_panel(ViewportClass::Wide);
    state.cycle_focus();
    assert!(state.submission_target_is_main());
}

#[test]
fn unpinned_main_panel_closes_on_narrow_layout() {
    let mut state = CockpitState::default();
    state.toggle_panel(ViewportClass::Wide);
    state.set_viewport(ViewportClass::Narrow);
    assert!(!state.is_open());
    assert_eq!(state.presentation(), PanelPresentation::Closed);
    state.set_viewport(ViewportClass::Wide);
    assert_eq!(state.presentation(), PanelPresentation::Closed);
}
