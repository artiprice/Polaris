use super::{
    BackendMessage, BackendRequest, ColorMode, Rect, RenderOptions, UiState, handle_input,
    present_frame, submit_draft,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use polaris_sdk::ids::SessionId;
use polaris_ui_cli::{
    input::NormalizedInput,
    presenter::{BufferedPresenter, CursorState},
};
use ratatui::{Terminal, TerminalOptions, Viewport, backend::CrosstermBackend};
use tokio::sync::mpsc;
fn state() -> Result<UiState, crate::CliError> {
    UiState::new(
        SessionId::new_v7(),
        "workspace".to_owned(),
        None,
        Vec::new(),
    )
}
#[test]
fn control_backpressure_preserves_draft_without_closing_ui()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("$websearch(on)")?;
    let (tx, _rx) = mpsc::channel(1);
    tx.try_send(BackendRequest::Sessions)?;
    let result = submit_draft(&mut state, &tx);
    assert!(
        matches!(result, Ok(false)),
        "full control queue must be recoverable: {result:?}"
    );
    assert_eq!(
        state.composer.text(),
        "$websearch(on)",
        "full queue must retain draft"
    );
    assert!(
        state.pending.is_none(),
        "unaccepted submission cannot be pending"
    );
    Ok(())
}
#[test]
fn accepted_directive_clears_only_after_acknowledgement() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = state()?;
    state.composer.insert("$websearch(on)")?;
    let (tx, mut rx) = mpsc::channel(1);
    assert!(!submit_draft(&mut state, &tx)?, "directive isn't quit");
    assert_eq!(
        state.composer.text(),
        "$websearch(on)",
        "pre-admission draft remains visible"
    );
    assert!(
        matches!(rx.try_recv()?, BackendRequest::Directive { .. }),
        "actual directive control expected"
    );
    state.receive(BackendMessage::Committed)?;
    assert!(
        state.composer.is_empty(),
        "commit clears the prepared revision"
    );
    Ok(())
}
#[test]
fn rejection_and_unsupported_model_input_keep_exact_draft() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = state()?;
    let (tx, _rx) = mpsc::channel(1);
    state.composer.insert("$websearch(on)")?;
    let _quit = submit_draft(&mut state, &tx)?;
    state.receive(BackendMessage::Rejected("permission".to_owned()))?;
    assert_eq!(
        state.composer.text(),
        "$websearch(on)",
        "rejected directive retained"
    );
    state
        .composer
        .apply_completion(0..state.composer.text().len(), "please change a file")?;
    let _quit = submit_draft(&mut state, &tx)?;
    assert_eq!(
        state.composer.text(),
        "please change a file",
        "unavailable model operation retains draft"
    );
    assert!(state.pending.is_none(), "no fabricated execution");
    Ok(())
}
#[test]
fn repeated_frame_and_caret_move_do_not_clear_terminal() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state.composer.insert("draft")?;
    let area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions {
        colors: ColorMode::TrueColor,
        ascii: false,
    };
    let mut presenter =
        BufferedPresenter::new_with_cursor_state(Vec::<u8>::new(), CursorState::Hidden);
    let mut terminal = Terminal::with_options(
        CrosstermBackend::new(presenter.writer()),
        TerminalOptions {
            viewport: Viewport::Fixed(area),
        },
    )?;
    let first = present_frame(&mut terminal, &mut presenter, &mut state, &options, area)?;
    assert!(first > 0, "first frame must paint");
    let unchanged = present_frame(&mut terminal, &mut presenter, &mut state, &options, area)?;
    assert_eq!(unchanged, 0, "identical application frame emits zero bytes");
    state.composer.move_left();
    let offset = presenter.output().len();
    let moved = present_frame(&mut terminal, &mut presenter, &mut state, &options, area)?;
    assert!(
        moved > 0 && moved < 32,
        "caret-only movement is bounded cursor output, got {moved}"
    );
    let bytes = presenter
        .output()
        .get(offset..)
        .ok_or("missing cursor output")?;
    assert!(
        !bytes.windows(b"draft".len()).any(|s| s == b"draft"),
        "caret cannot repaint text"
    );
    assert!(
        !bytes.windows(4).any(|s| s == b"\x1b[2J"),
        "caret cannot clear canvas"
    );
    Ok(())
}
#[test]
fn release_is_idle_and_paste_remains_literal() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let (tx, mut rx) = mpsc::channel(1);
    let mut area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions {
        colors: ColorMode::NoColor,
        ascii: true,
    };
    let release = KeyEvent::new_with_kind(
        KeyCode::Enter,
        KeyModifiers::NONE,
        crossterm::event::KeyEventKind::Release,
    );
    assert_eq!(
        handle_input(
            &mut state,
            NormalizedInput::Key(release),
            &tx,
            &mut area,
            &options
        )?,
        (false, false),
        "release cannot repaint or submit"
    );
    let _outcome = handle_input(
        &mut state,
        NormalizedInput::Paste("$websearch(on)\n/quit".to_owned()),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(
        state.composer.text(),
        "$websearch(on)\n/quit",
        "paste is exact literal draft"
    );
    assert!(
        rx.try_recv().is_err(),
        "paste cannot become command dispatch"
    );
    Ok(())
}

#[test]
fn pending_resume_prevents_directive_from_crossing_sessions()
-> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let target = SessionId::new_v7();
    let (tx, mut rx) = mpsc::channel(4);
    state
        .composer
        .insert(&format!("/session resume {target}"))?;
    let _quit = submit_draft(&mut state, &tx)?;
    assert!(
        matches!(rx.try_recv()?,BackendRequest::Resume(id) if id==target),
        "targeted resume must reach backend"
    );
    state.composer.insert("$websearch(on)")?;
    let _quit = submit_draft(&mut state, &tx)?;
    assert!(
        rx.try_recv().is_err(),
        "no directive may cross a pending session switch"
    );
    assert_eq!(
        state.composer.text(),
        "$websearch(on)",
        "pending switch preserves later draft"
    );
    Ok(())
}

#[test]
fn help_only_advertises_available_local_controls() {
    let help = super::help_lines().join("\n");
    for unavailable in [
        "[Provider form]",
        "[Human questions]",
        "[Question notes]",
        "Paste (image first",
        "Select and copy the semantic",
    ] {
        assert!(
            !help.contains(unavailable),
            "unavailable action advertised as live: {unavailable}"
        );
    }
    assert!(
        help.contains("Ctrl+J"),
        "safe newline fallback must be documented"
    );
}

#[test]
fn draft_text_chip_submits_its_expanded_directive() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let (tx, mut rx) = mpsc::channel(1);
    let input = format!("$websearch({}on)", " ".repeat(600));
    state.composer.insert(&input)?;
    assert!(
        !state.composer.chips().is_empty(),
        "long literal paste becomes a real text chip"
    );
    let _quit = submit_draft(&mut state, &tx)?;
    assert!(
        matches!(rx.try_recv(),Ok(BackendRequest::Directive{text,..}) if text==input),
        "text chip must submit validated expanded text after explicit Enter"
    );
    Ok(())
}
#[test]
fn draft_limit_rejection_keeps_editor_alive() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    state
        .composer
        .insert(&"x".repeat(polaris_ui_model::composer::MAX_TEXT_BYTES))?;
    let (tx, _rx) = mpsc::channel(1);
    let mut area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions {
        colors: ColorMode::NoColor,
        ascii: true,
    };
    let input = NormalizedInput::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    let result = handle_input(&mut state, input, &tx, &mut area, &options);
    assert!(
        matches!(result, Ok((false, true))),
        "draft overflow must be recoverable, got {result:?}"
    );
    assert_eq!(
        state.composer.logical_len(),
        polaris_ui_model::composer::MAX_TEXT_BYTES,
        "failed edit cannot mutate draft"
    );
    Ok(())
}

#[test]
fn query_failure_cannot_cancel_an_inflight_directive() -> Result<(), Box<dyn std::error::Error>> {
    let mut state = state()?;
    let (tx, _rx) = mpsc::channel(1);
    state.composer.insert("$websearch(on)")?;
    let _quit = submit_draft(&mut state, &tx)?;
    state.receive(BackendMessage::QueryFailed(
        "registry read failed".to_owned(),
    ))?;
    assert!(
        state.pending.is_some(),
        "an unrelated query failure cannot cancel directive admission"
    );
    assert_eq!(
        state.composer.text(),
        "$websearch(on)",
        "query failure preserves the in-flight draft"
    );
    state.receive(BackendMessage::Committed)?;
    assert!(
        state.composer.is_empty(),
        "real directive acknowledgement must still commit the prepared draft"
    );
    Ok(())
}

#[test]
fn completion_wheel_clamps_without_scrolling_transcript() -> Result<(), Box<dyn std::error::Error>>
{
    let mut state = state()?;
    state.composer.insert("/")?;
    state.refresh_completion();
    assert!(
        !state.completion.is_empty(),
        "canonical slash candidates required"
    );
    let (tx, _rx) = mpsc::channel(1);
    let mut area = Rect::new(0, 0, 80, 24);
    let options = RenderOptions {
        colors: ColorMode::NoColor,
        ascii: true,
    };
    let wheel = |kind| {
        NormalizedInput::Mouse(crossterm::event::MouseEvent {
            kind,
            column: 5,
            row: 8,
            modifiers: KeyModifiers::NONE,
        })
    };
    let _outcome = handle_input(
        &mut state,
        wheel(crossterm::event::MouseEventKind::ScrollUp),
        &tx,
        &mut area,
        &options,
    )?;
    assert_eq!(
        state.completion_selected,
        Some(0),
        "wheel up selects/clamps first candidate"
    );
    assert_eq!(
        state.history_scroll, 0,
        "completion wheel cannot scroll transcript"
    );
    for _ in 0..state.completion.len().saturating_add(4) {
        let _outcome = handle_input(
            &mut state,
            wheel(crossterm::event::MouseEventKind::ScrollDown),
            &tx,
            &mut area,
            &options,
        )?;
    }
    assert_eq!(
        state.completion_selected,
        Some(state.completion.len().saturating_sub(1)),
        "wheel down clamps last candidate"
    );
    assert_eq!(state.history_scroll, 0, "transcript remains anchored");
    Ok(())
}
