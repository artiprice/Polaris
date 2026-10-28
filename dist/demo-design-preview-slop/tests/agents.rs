#[path = "support/capture.rs"]
mod capture;
use crossterm::event::{
    KeyCode as K, KeyEvent, KeyModifiers as M, MouseButton, MouseEvent, MouseEventKind,
};
use polaris::{
    agents::{AgentAction, AgentActivity, AgentRecord, AgentStatus as S},
    agents_ui,
    app::{App, Entry, EntryKind, Screen},
    interaction::{HitAction, TextRegionId, UiMap, ViewSelection},
    runtime::{MouseController, selection_text},
    ui,
};
use ratatui::{Terminal, backend::TestBackend, buffer::Buffer, style::Modifier};
fn key(app: &mut App, code: K) {
    app.key(KeyEvent::new(code, M::NONE));
}
fn draw(app: &App, w: u16, h: u16) -> (UiMap, Buffer) {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    let mut m = UiMap::default();
    t.draw(|f| m = ui::render_interactive(f, app)).unwrap();
    (m, t.backend().buffer().clone())
}
fn text(b: &Buffer) -> String {
    b.content.iter().map(|c| c.symbol()).collect()
}
fn scenario(name: &str) -> App {
    let mut a = App::default();
    a.load_scenario(name).unwrap();
    a
}
fn mouse(c: &mut MouseController, a: &mut App, m: &UiMap, kind: MouseEventKind, x: u16, y: u16) {
    c.event(
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: M::NONE,
        },
        m,
        a,
    );
}
fn click(a: &mut App, m: &UiMap, action: HitAction) {
    let r = m.targets.iter().find(|t| t.action == action).unwrap().rect;
    let mut c = MouseController::default();
    mouse(
        &mut c,
        a,
        m,
        MouseEventKind::Down(MouseButton::Left),
        r.x,
        r.y,
    );
    mouse(
        &mut c,
        a,
        m,
        MouseEventKind::Up(MouseButton::Left),
        r.x,
        r.y,
    );
}
#[test]
fn enabling_subagents_is_not_an_execution_or_active_roster() {
    let mut a = App::default();
    a.input.set("$subagents");
    a.submit();
    assert!(!a.agents.visible());
    assert!(!a.agents.active());
    assert_eq!(agents_ui::dock_height(&a, 40), 0);
    a.command("/agents");
    assert!(a.agents.focused);
    let (_, b) = draw(&a, 100, 30);
    assert!(text(&b).contains("No subagents started"));
    assert!(text(&b).contains("Main"));
}
#[test]
fn batches_expand_without_overriding_manual_collapse_and_terminal_records_remain() {
    let mut a = AgentActivity::default();
    a.insert(AgentRecord::new(42, "One", "Task", S::Working));
    assert!(a.expanded);
    a.expanded = false;
    a.manually_collapsed = true;
    a.insert(AgentRecord::new(99, "Two", "Task", S::Waiting));
    assert!(!a.expanded);
    a.set_status(42, S::Completed);
    a.set_status(99, S::Failed);
    assert!(!a.expanded);
    assert_eq!(a.records.len(), 2);
    a.set_status(99, S::Working);
    assert!(a.expanded);
    assert!(!a.manually_collapsed);
    assert_eq!(a.records.len(), 2);
    assert_eq!(a.get(99).unwrap().tokens, None);
    a.set_status(99, S::Cancelled);
    assert!(!a.active());
}
#[test]
fn unknown_ids_do_not_change_the_view_or_selection() {
    let mut a = scenario("agents-mixed");
    a.view_agent(Some(1));
    a.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: 2,
    });
    a.view_agent(Some(999));
    assert_eq!(a.agents.viewed, Some(1));
    assert!(a.view_selection.is_some());
}
#[test]
fn threads_preserve_main_draft_cursor_attachments_undo_scroll_and_tools() {
    let mut a = scenario("agents-mixed");
    a.input.set("main draft");
    a.input.cursor = 4;
    a.paste(&"x".repeat(501));
    let draft = a.input.clone();
    a.scroll = 7;
    a.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: 2,
    });
    a.view_agent(Some(1));
    assert_eq!(a.scroll, 0);
    assert!(a.view_selection.is_none());
    assert_eq!(a.input.text, draft.text);
    assert_eq!(a.input.cursor, draft.cursor);
    assert_eq!(a.input.spans.len(), 1);
    a.scroll = 3;
    a.key(KeyEvent::new(K::Char('o'), M::CONTROL));
    assert!(a.agents.get(1).unwrap().tools_expanded);
    assert!(!a.tool_details_expanded);
    a.view_agent(Some(2));
    assert_eq!(a.scroll, 0);
    assert!(!a.tool_expanded(1));
    a.view_agent(Some(1));
    assert_eq!(a.scroll, 3);
    assert!(a.tool_expanded(1));
    a.view_agent(None);
    assert_eq!(a.scroll, 7);
    assert_eq!(a.input.text, draft.text);
    a.input.undo();
    assert_eq!(a.input.text, "main draft");
}
#[test]
fn accepted_prompt_targets_main_but_rejected_submission_keeps_view_and_draft() {
    let mut a = scenario("agents-mixed");
    a.view_agent(Some(2));
    let n = a.agents.get(2).unwrap().transcript.len();
    a.input.set("Send to Main");
    a.working = true;
    a.submit();
    assert_eq!(a.agents.viewed, Some(2));
    assert_eq!(a.input.text, "Send to Main");
    assert!(a.submissions.is_empty());
    a.working = false;
    a.submit();
    assert_eq!(a.agents.viewed, None);
    assert_eq!(a.submissions.len(), 1);
    assert_eq!(a.entries.last().unwrap().kind, EntryKind::User);
    assert_eq!(a.agents.get(2).unwrap().transcript.len(), n);
}
#[test]
fn usage_and_escape_preserve_view_then_return_main_before_interrupting() {
    let mut a = scenario("agents-working");
    a.view_agent(Some(1));
    a.scroll = 2;
    a.input.set("draft");
    a.command("/usage");
    assert_eq!(a.screen, Screen::Usage);
    key(&mut a, K::Esc);
    assert_eq!(a.screen, Screen::Home);
    assert_eq!(a.agents.viewed, Some(1));
    assert_eq!(a.scroll, 2);
    a.open_agents();
    key(&mut a, K::Esc);
    assert!(!a.agents.focused);
    assert_eq!(a.agents.viewed, Some(1));
    key(&mut a, K::Esc);
    assert_eq!(a.agents.viewed, None);
    assert!(a.working);
    key(&mut a, K::Esc);
    assert!(!a.working);
    assert_eq!(a.input.text, "draft");
}
#[test]
fn explicit_roster_focus_scopes_arrows_and_enter_without_consuming_draft() {
    let mut a = scenario("agents-many");
    a.input.set("draft");
    let mode = a.mode;
    let effort = a.effort_label().to_owned();
    key(&mut a, K::F(3));
    key(&mut a, K::PageDown);
    assert_eq!(a.agents.selected, Some(3));
    assert_eq!(a.agents.viewed, None);
    a.key(KeyEvent::new(K::Down, M::SHIFT));
    assert_eq!(a.agents.selected, Some(4));
    assert_eq!(a.effort_label(), effort);
    key(&mut a, K::Enter);
    assert_eq!(a.agents.viewed, Some(4));
    assert!(!a.agents.focused);
    assert_eq!(a.input.text, "draft");
    key(&mut a, K::F(3));
    key(&mut a, K::BackTab);
    assert!(!a.agents.focused);
    assert_eq!(a.mode, mode);
    key(&mut a, K::F(3));
    key(&mut a, K::Char('!'));
    assert!(!a.agents.focused);
    assert_eq!(a.input.text, "draft!");
    key(&mut a, K::F(3));
    key(&mut a, K::Left);
    assert!(!a.agents.expanded);
    assert!(a.agents.manually_collapsed);
}
#[test]
fn f3_dismisses_completion_and_leaves_text_intact() {
    let mut a = scenario("agents-many");
    a.input.set("/");
    key(&mut a, K::F(3));
    assert!(a.menu_dismissed);
    assert_eq!(a.input.text, "/");
    assert!(a.agents.focused);
}
#[test]
fn driver_runs_after_main_finishes_and_clear_removes_all_activity() {
    let mut a = scenario("agents-working");
    for _ in 0..18 {
        a.tick();
    }
    assert!(!a.working);
    assert!(a.agents.active());
    for _ in 18..80 {
        a.tick();
    }
    assert!(!a.agents.active());
    assert!(!a.agents.expanded);
    assert_eq!(a.agents.records.len(), 2);
    assert!(a.agents.records.iter().all(|r| r.status == S::Completed));
    assert!(a.agents.get(1).unwrap().transcript.last().unwrap().lines[0].contains("Task complete"));
    a.command("/clear");
    assert!(!a.agents.visible());
    assert!(a.agents.records.is_empty());
    assert_eq!(a.agents.viewed, None);
}
#[test]
fn copying_uses_the_viewed_thread_and_switching_clears_reused_source_ids() {
    let mut a = scenario("agents-mixed");
    a.view_agent(Some(2));
    let (m, _) = draw(&a, 120, 40);
    let source = m
        .region(&TextRegionId::Conversation)
        .unwrap()
        .source
        .clone();
    assert!(source.contains("RTK tools activity"));
    assert!(!source.contains("Splitting the registry"));
    a.view_selection = Some(ViewSelection {
        region: TextRegionId::Conversation,
        anchor: 0,
        end: source.len(),
    });
    assert_eq!(selection_text(&a, &m), source.as_ref());
    a.view_agent(None);
    assert!(a.view_selection.is_none());
    let (m, _) = draw(&a, 120, 40);
    assert!(
        m.region(&TextRegionId::Conversation)
            .unwrap()
            .source
            .contains("Splitting the registry")
    );
}
#[test]
fn inline_and_dock_mouse_clicks_switch_threads_hover_and_wheel_do_not() {
    let mut a = scenario("agents-many");
    let (m, _) = draw(&a, 120, 40);
    let target = HitAction::Agent(AgentAction::Open(Some(1)));
    let targets = m
        .targets
        .iter()
        .filter(|t| t.action == target)
        .collect::<Vec<_>>();
    assert!(
        targets.len() >= 2,
        "inline launch task and dock row must be interactive"
    );
    let r = targets[0].rect;
    let mut c = MouseController::default();
    mouse(&mut c, &mut a, &m, MouseEventKind::Moved, r.x, r.y);
    assert_eq!(a.agents.viewed, None);
    click(&mut a, &m, target);
    assert_eq!(a.agents.viewed, Some(1));
    a.scroll = 2;
    let (m, _) = draw(&a, 120, 40);
    let r = m.agent_rect.unwrap();
    mouse(&mut c, &mut a, &m, MouseEventKind::ScrollDown, r.x, r.y);
    assert_eq!(a.agents.offset, 1);
    assert_eq!(a.scroll, 2);
    assert_eq!(a.agents.viewed, Some(1));
    click(&mut a, &m, HitAction::Agent(AgentAction::Open(None)));
    assert_eq!(a.agents.viewed, None);
}
#[test]
fn overflow_keeps_main_pinned_and_selected_child_visible_across_resize() {
    let mut a = scenario("agents-many");
    a.open_agents();
    for _ in 0..9 {
        key(&mut a, K::Down);
    }
    for h in [20, 24, 40, 60] {
        let offset = agents_ui::scroll_offset(&a, h);
        let rows = agents_ui::child_rows(&a, h);
        assert!(offset <= 8 && offset + rows >= 9);
        let (m, _) = draw(&a, 80, h);
        assert!(
            m.targets
                .iter()
                .any(|t| t.action == HitAction::Agent(AgentAction::Open(None)))
        );
        assert!(
            m.targets
                .iter()
                .any(|t| t.action == HitAction::Agent(AgentAction::Open(Some(9))))
        );
    }
}
#[test]
fn sheets_occlude_underlying_agent_targets() {
    let mut a = scenario("agents-many");
    a.command("/model");
    let (m, _) = draw(&a, 100, 30);
    let popup = ui::popup_area(&a, ratatui::layout::Rect::new(0, 0, 100, 30));
    assert!(
        m.targets
            .iter()
            .filter(|t| matches!(t.action, HitAction::Agent(_)))
            .all(|t| t.rect.intersection(popup).is_empty())
    );
}
#[test]
fn layout_matrix_keeps_editor_and_roster_in_bounds_with_plain_error_and_ascii_fallback() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for name in [
            "agents-working",
            "agents-mixed",
            "agents-completed",
            "agents-many",
        ] {
            for ascii in [false, true] {
                let mut a = scenario(name);
                a.ascii = ascii;
                a.input.set(&"line of text\n".repeat(40));
                a.view_agent(Some(1));
                let (m, b) = draw(&a, w, h);
                assert!(m.region(&TextRegionId::Composer).is_some());
                for t in &m.targets {
                    assert!(
                        t.rect.right() <= w && t.rect.bottom() <= h,
                        "{name} {w}x{h}: {:?}",
                        t
                    );
                }
                let plain = text(&b);
                assert!(plain.contains("prompt to Main"));
                if ascii {
                    assert!(!plain.contains('─'));
                    assert!(!plain.contains('●'));
                }
                if !ascii {
                    capture::export(&format!("{name}-thread"), &b);
                }
            }
        }
    }
    let mut a = scenario("agents-mixed");
    a.agents.offset = 3;
    let (_, b) = draw(&a, 120, 40);
    let failed = b
        .content
        .iter()
        .filter(|c| c.fg == a.theme.error().fg.unwrap())
        .collect::<Vec<_>>();
    assert!(!failed.is_empty());
    assert!(failed.iter().all(|c| !c.modifier.contains(Modifier::BOLD)));
    capture::export("agents-mixed-roster", &b);
    a.agents.offset = 0;
    let (_, b) = draw(&a, 120, 40);
    capture::export("agents-main", &b);
}
#[test]
fn ordinary_records_do_not_claim_demo_or_known_metrics() {
    let mut a = App::default();
    let mut r = AgentRecord::new(1, "Worker", "Task", S::Waiting);
    r.transcript
        .push(Entry::new(EntryKind::Assistant, "", &["Agent response"]));
    a.agents.insert(r);
    a.view_agent(Some(1));
    let (_, b) = draw(&a, 120, 40);
    let t = text(&b);
    assert!(!t.contains("[demo]"));
    assert!(t.contains("tokens unavailable"));
    assert!(t.contains("time unavailable"));
}

#[test]
fn paste_returns_to_composer_focus_and_preserves_the_inspected_thread() {
    let mut a = scenario("agents-many");
    a.view_agent(Some(2));
    a.open_agents();
    a.paste("pasted text");
    assert!(!a.agents.focused);
    assert_eq!(a.agents.viewed, Some(2));
    assert_eq!(a.input.text, "pasted text");
    a.open_agents();
    a.key(KeyEvent::new(K::Char('v'), M::CONTROL));
    assert!(!a.agents.focused);
    assert_eq!(a.agents.viewed, Some(2));
}
#[test]
fn latest_and_tool_title_mouse_actions_only_change_the_visible_thread() {
    let mut a = scenario("agents-many");
    a.scroll = 4;
    a.view_agent(Some(1));
    a.toggle_tool_details(1);
    assert!(a.tool_expanded(1));
    assert!(!a.tool_details_expanded);
    let (m, _) = draw(&a, 100, 40);
    click(&mut a, &m, HitAction::ToggleTool(1));
    assert!(!a.tool_expanded(1));
    a.agents.get_mut(1).unwrap().transcript.push(Entry::new(
        EntryKind::Assistant,
        "",
        &vec!["Long agent output"; 30],
    ));
    a.scroll = 2;
    let (m, _) = draw(&a, 100, 24);
    click(&mut a, &m, HitAction::ToLatest);
    assert_eq!(a.scroll, 0);
    a.view_agent(None);
    assert_eq!(a.scroll, 4);
}
#[test]
fn short_approval_and_goal_keep_editor_and_actions_reachable() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        let mut a = scenario("agents-working");
        a.input.set(&"draft line\n".repeat(60));
        a.goal = Some(polaris::app::GoalProgress {
            running_since: Some(std::time::Instant::now()),
            accumulated: std::time::Duration::ZERO,
        });
        a.approval = true;
        let (m, b) = draw(&a, w, h);
        assert!(m.region(&TextRegionId::Composer).is_some());
        assert!(
            m.targets
                .iter()
                .any(|t| matches!(t.action, HitAction::Approval(_)))
        );
        assert!(text(&b).contains("goal active"));
        assert!(
            m.targets
                .iter()
                .all(|t| t.rect.right() <= w && t.rect.bottom() <= h)
        );
        if h < 24 {
            assert_eq!(agents_ui::dock_height(&a, h), 2);
        }
        capture::export("agents-approval", &b);
    }
}
#[test]
fn terminal_transition_freezes_elapsed_and_loading_a_session_drops_stale_agents() {
    let mut a = scenario("agents-many");
    let r = a.agents.get_mut(1).unwrap();
    r.elapsed_seconds = None;
    r.started = Some(std::time::Instant::now() - std::time::Duration::from_secs(15));
    a.agents.set_status(1, S::Completed);
    assert_eq!(a.agents.get(1).unwrap().elapsed(), Some(15));
    assert!(a.agents.get(1).unwrap().started.is_none());
    a.view_agent(Some(1));
    a.command("/sessions");
    key(&mut a, K::Enter);
    assert_eq!(a.screen, Screen::Home);
    assert!(a.agents.records.is_empty());
    assert_eq!(a.agents.viewed, None);
}

#[test]
fn roster_symbols_distinguish_main_and_working_children_without_shifting_columns() {
    for (w, h) in [(60, 20), (80, 24), (120, 40), (160, 60)] {
        for ascii in [false, true] {
            for (status, unicode, fallback) in [
                (S::Working, "◯", "*"),
                (S::Queued, "○", "o"),
                (S::Waiting, "◌", "-"),
                (S::Paused, "Ⅱ", "-"),
                (S::Completed, "✓", "+"),
                (S::Failed, "!", "!"),
                (S::Cancelled, "−", "-"),
            ] {
                for working in [false, true] {
                    let mut app = App::default();
                    app.ascii = ascii;
                    app.working = working;
                    app.agents
                        .insert(AgentRecord::new(1, "Worker", "Task", status));
                    app.agents.expanded = true;
                    let (map, buffer) = draw(&app, w, h);
                    let dock = map.agent_rect.unwrap();
                    for (id, expected, initial) in [
                        (None, if ascii { "*" } else { "●" }, "M"),
                        (Some(1), if ascii { fallback } else { unicode }, "W"),
                    ] {
                        assert_eq!(unicode_width::UnicodeWidthStr::width(expected), 1);
                        let row = map
                            .targets
                            .iter()
                            .find(|t| {
                                t.action == HitAction::Agent(AgentAction::Open(id))
                                    && dock.contains((t.rect.x, t.rect.y).into())
                            })
                            .unwrap()
                            .rect;
                        assert_eq!(
                            buffer[(row.x + 2, row.y)].symbol(),
                            expected,
                            "{status:?} {w}x{h} ascii={ascii} main-working={working} id={id:?}"
                        );
                        assert_eq!(buffer[(row.x + 3, row.y)].symbol(), " ");
                        assert_eq!(buffer[(row.x + 4, row.y)].symbol(), initial);
                    }
                }
            }
        }
    }
}
