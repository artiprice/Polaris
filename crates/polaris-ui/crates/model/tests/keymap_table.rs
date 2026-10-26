//! Keymap Table (descriptor: specs/descriptors/uimodel_keymap_table.json).

use std::collections::BTreeSet;
use std::error::Error;

use polaris_ui_model::keymap::{
    Action, Context, GuardState, Key, KeyChord, Modifiers, MouseGesture, Trigger, UNBOUND,
    footer_text, help_lines, keymap, lookup, lookup_mouse,
};
use proptest::prelude::*;

type TestResult = Result<(), Box<dyn Error>>;

const NONE: GuardState = GuardState {
    draft_empty: false,
    first_visual_row: false,
    last_visual_row: false,
};
const EMPTY_DRAFT: GuardState = GuardState {
    draft_empty: true,
    first_visual_row: false,
    last_visual_row: false,
};
const FIRST_ROW: GuardState = GuardState {
    draft_empty: false,
    first_visual_row: true,
    last_visual_row: false,
};
const LAST_ROW: GuardState = GuardState {
    draft_empty: false,
    first_visual_row: false,
    last_visual_row: true,
};

fn ch(c: char) -> KeyChord {
    KeyChord::plain(Key::Char(c))
}

fn resolves(context: Context, chord: KeyChord, state: GuardState, expected: Action) -> TestResult {
    let found =
        lookup(context, chord, state).ok_or_else(|| format!("{context:?} {chord} is unbound"))?;
    assert_eq!(found.action, expected, "{context:?} {chord}");
    Ok(())
}

#[test]
fn every_action_has_a_binding_or_is_marked_unbound() {
    for action in Action::ALL {
        let bound = keymap().iter().any(|b| b.action == *action);
        let unbound = UNBOUND.iter().any(|(a, _)| a == action);
        assert!(
            bound || unbound,
            "{action:?} is neither bound nor marked unbound"
        );
        assert!(
            !(bound && unbound),
            "{action:?} is both bound and marked unbound"
        );
    }
    for (action, reason) in UNBOUND {
        assert!(!reason.is_empty(), "{action:?} unbound without a reason");
    }
}

#[test]
fn unbound_actions_are_not_bound() {
    for (action, _) in UNBOUND {
        assert!(keymap().iter().all(|b| b.action != *action), "{action:?}");
    }
    assert!(UNBOUND.iter().any(|(a, _)| *a == Action::Quit));
}

#[test]
fn no_duplicate_chords_in_one_context() {
    let mut seen = BTreeSet::new();
    for b in keymap() {
        let key = format!("{:?}|{}|{:?}", b.context, b.trigger, b.guard);
        assert!(seen.insert(key.clone()), "duplicate binding {key}");
    }
}

#[test]
fn footer_text_generated_from_table() {
    assert_eq!(
        footer_text(keymap(), Context::Composer, EMPTY_DRAFT),
        "Enter send · Shift+Enter newline · Shift+Tab mode · ? shortcuts · Ctrl+O tools"
    );
    assert_eq!(
        footer_text(keymap(), Context::Composer, NONE),
        "Enter send · Shift+Enter newline · Shift+Tab mode · Ctrl+O tools"
    );
    assert!(
        footer_text(keymap(), Context::Working, NONE).starts_with("Esc interrupt · Enter send")
    );
    assert_eq!(
        footer_text(keymap(), Context::InfoPopup, NONE),
        "Enter close · Esc back"
    );
    assert_eq!(
        footer_text(keymap(), Context::ModelSheet, NONE),
        "P provider · C configure · Left less effort · Right more effort · Enter confirm · Esc back"
    );
}

#[test]
fn footer_changes_when_the_table_changes() -> TestResult {
    let mut table = keymap().to_vec();
    let submit = table
        .iter_mut()
        .find(|b| b.action == Action::Submit)
        .ok_or("submit binding")?;
    submit.trigger = Trigger::Key(KeyChord::ctrl(Key::Enter));
    submit.footer = Some("go");
    let footer = footer_text(&table, Context::Composer, NONE);
    assert!(footer.starts_with("Ctrl+Enter go · "), "{footer}");
    assert!(!footer.contains("Enter send"));
    Ok(())
}

#[test]
fn help_lines_list_every_chord() {
    let lines = help_lines(keymap());
    for b in keymap() {
        let chord = b.trigger.to_string();
        assert!(
            lines
                .iter()
                .any(|l| l.contains(&chord) && l.contains(b.description)),
            "no help line for {chord} / {}",
            b.description
        );
    }
    assert!(lines.iter().all(|l| !l.is_empty()));
    // Several newline chords are folded into one line for the action.
    let newline = lines.iter().filter(|l| l.contains("Shift+Enter")).count();
    assert_eq!(newline, 1);
}

#[test]
fn plan_keymap_rows_present() -> TestResult {
    use Action::*;
    use Context::*;
    resolves(Composer, KeyChord::plain(Key::Enter), NONE, Submit)?;
    resolves(Composer, KeyChord::shift(Key::Enter), NONE, Newline)?;
    resolves(Composer, KeyChord::alt(Key::Enter), NONE, Newline)?;
    assert!(
        lookup(Composer, KeyChord::ctrl(Key::Enter), NONE).is_none(),
        "Ctrl+Enter is intentionally unbound; Shift+Enter is the newline shortcut"
    );
    resolves(Composer, KeyChord::ctrl(Key::Char('j')), NONE, Newline)?;
    resolves(Composer, KeyChord::plain(Key::Up), FIRST_ROW, HistoryPrev)?;
    resolves(Composer, KeyChord::plain(Key::Down), LAST_ROW, HistoryNext)?;
    resolves(Composer, KeyChord::shift(Key::Tab), NONE, ToggleMode)?;
    resolves(Composer, KeyChord::shift(Key::Up), NONE, EffortUp)?;
    resolves(Composer, KeyChord::shift(Key::Down), NONE, EffortDown)?;
    resolves(Working, KeyChord::plain(Key::Esc), NONE, Interrupt)?;
    resolves(Popup, KeyChord::plain(Key::Esc), NONE, PopupBack)?;
    resolves(CompletionMenu, KeyChord::plain(Key::Esc), NONE, PopupBack)?;
    resolves(InfoPopup, KeyChord::plain(Key::Enter), NONE, ClosePopup)?;
    resolves(StatsSheet, KeyChord::plain(Key::Enter), NONE, ClosePopup)?;
    resolves(
        SelectionMenu,
        KeyChord::plain(Key::Enter),
        NONE,
        ConfirmSelection,
    )?;
    resolves(
        ModelSheet,
        KeyChord::plain(Key::Enter),
        NONE,
        ConfirmSelection,
    )?;
    resolves(
        CompletionMenu,
        KeyChord::plain(Key::Enter),
        NONE,
        ConfirmSelection,
    )?;
    resolves(Composer, ch('?'), EMPTY_DRAFT, OpenHelp)?;
    resolves(Composer, KeyChord::plain(Key::PageUp), NONE, ScrollPageUp)?;
    resolves(
        ModelSheet,
        KeyChord::plain(Key::PageDown),
        NONE,
        ScrollPageDown,
    )?;
    resolves(Composer, KeyChord::plain(Key::F(2)), NONE, OpenStatus)?;
    resolves(Composer, KeyChord::ctrl(Key::Char('o')), NONE, ToggleTools)?;
    resolves(
        Composer,
        KeyChord::ctrl(Key::Char('c')),
        NONE,
        InterruptOrExit,
    )?;
    resolves(
        InfoPopup,
        KeyChord::ctrl(Key::Char('c')),
        NONE,
        InterruptOrExit,
    )?;
    resolves(ProviderForm, KeyChord::plain(Key::Tab), NONE, NextField)?;
    resolves(ProviderForm, KeyChord::shift(Key::Tab), NONE, PrevField)?;
    resolves(
        ProviderForm,
        KeyChord::ctrl(Key::Char('s')),
        NONE,
        SaveSettings,
    )?;
    resolves(ModelSheet, ch('p'), NONE, ChooseProvider)?;
    resolves(ModelSheet, ch('c'), NONE, ConfigureProvider)?;
    resolves(ModelSheet, KeyChord::plain(Key::Left), NONE, EffortDown)?;
    resolves(ModelSheet, KeyChord::plain(Key::Right), NONE, EffortUp)?;
    resolves(StatsSheet, KeyChord::plain(Key::Tab), NONE, SwitchStatsTab)?;
    resolves(StatsSheet, KeyChord::plain(Key::Left), NONE, SwitchStatsTab)?;
    resolves(
        StatsSheet,
        KeyChord::plain(Key::Right),
        NONE,
        SwitchStatsTab,
    )?;
    resolves(StatsSheet, ch('r'), NONE, CycleStatsRange)?;
    resolves(Composer, KeyChord::ctrl(Key::Char('v')), NONE, Paste)?;
    resolves(Composer, KeyChord::shift(Key::Insert), NONE, Paste)?;
    resolves(AttachmentPreview, ch('e'), NONE, ExpandAttachment)?;
    resolves(AttachmentPreview, ch('c'), NONE, CopyAttachment)?;
    resolves(Composer, KeyChord::ctrl(Key::Char('z')), NONE, Undo)?;
    resolves(Composer, KeyChord::ctrl(Key::Char('y')), NONE, Redo)?;
    Ok(())
}

#[test]
fn lookup_respects_guards_and_context_chain() {
    // History keys are only bound on the first / last visual row.
    assert!(lookup(Context::Composer, KeyChord::plain(Key::Up), NONE).is_none());
    assert!(lookup(Context::Composer, KeyChord::plain(Key::Up), LAST_ROW).is_none());
    assert!(lookup(Context::Composer, KeyChord::plain(Key::Down), FIRST_ROW).is_none());
    // `?` opens help only on an empty draft.
    assert!(lookup(Context::Composer, ch('?'), NONE).is_none());
    // Plain letters are not bound in the composer (they are typed).
    assert!(lookup(Context::Composer, ch('p'), NONE).is_none());
    // A popup context does not inherit composer-only keys.
    assert!(lookup(Context::InfoPopup, KeyChord::shift(Key::Enter), NONE).is_none());
    assert!(lookup(Context::ProviderForm, KeyChord::ctrl(Key::Char('o')), NONE).is_none());
    // Shift+Tab means different things in composer and provider form.
    assert_eq!(
        lookup(Context::Composer, KeyChord::shift(Key::Tab), NONE).map(|b| b.action),
        Some(Action::ToggleMode)
    );
    assert_eq!(
        lookup(Context::ProviderForm, KeyChord::shift(Key::Tab), NONE).map(|b| b.action),
        Some(Action::PrevField)
    );
    // Esc while working interrupts, but in a popup it only goes back.
    assert_eq!(
        lookup(Context::Working, KeyChord::plain(Key::Esc), NONE).map(|b| b.action),
        Some(Action::Interrupt)
    );
    assert_eq!(
        lookup(Context::Composer, KeyChord::plain(Key::Esc), NONE).map(|b| b.action),
        None
    );
}

#[test]
fn chord_display_format() {
    assert_eq!(KeyChord::ctrl(Key::Char('o')).to_string(), "Ctrl+O");
    assert_eq!(KeyChord::shift(Key::Enter).to_string(), "Shift+Enter");
    assert_eq!(KeyChord::plain(Key::Esc).to_string(), "Esc");
    assert_eq!(KeyChord::plain(Key::F(2)).to_string(), "F2");
    assert_eq!(ch('?').to_string(), "?");
    assert_eq!(ch('p').to_string(), "P");
    assert_eq!(KeyChord::plain(Key::PageUp).to_string(), "PageUp");
    assert_eq!(ch(' ').to_string(), "Space");
    let all = KeyChord::new(
        Key::Up,
        Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
        },
    );
    assert_eq!(all.to_string(), "Ctrl+Alt+Shift+Up");
}

#[test]
fn chord_normalization() -> TestResult {
    let shifted = KeyChord::new(
        Key::Char('P'),
        Modifiers {
            ctrl: false,
            alt: false,
            shift: true,
        },
    );
    assert_eq!(shifted.normalized(), ch('p'));
    assert_eq!(KeyChord::shift(Key::Char('?')).normalized(), ch('?'));
    assert_ne!(
        KeyChord::shift(Key::Enter).normalized(),
        KeyChord::plain(Key::Enter)
    );
    resolves(Context::ModelSheet, shifted, NONE, Action::ChooseProvider)?;
    resolves(
        Context::Composer,
        KeyChord::ctrl(Key::Char('O')),
        NONE,
        Action::ToggleTools,
    )?;
    Ok(())
}

#[test]
fn mouse_gestures_bound() -> TestResult {
    let find = |ctx, gesture| lookup_mouse(ctx, gesture).map(|b| b.action);
    assert_eq!(
        find(Context::CompletionMenu, MouseGesture::WheelUp),
        Some(Action::MenuPrev)
    );
    assert_eq!(
        find(Context::CompletionMenu, MouseGesture::WheelDown),
        Some(Action::MenuNext)
    );
    assert_eq!(
        find(Context::Composer, MouseGesture::DragRelease),
        Some(Action::SelectCopy)
    );
    assert_eq!(
        find(Context::Composer, MouseGesture::DoubleClickDrag),
        Some(Action::SelectWord)
    );
    assert_eq!(
        find(Context::Composer, MouseGesture::ClickToolTitle),
        Some(Action::ToggleToolEntry)
    );
    assert_eq!(
        find(Context::Composer, MouseGesture::ClickToLatest),
        Some(Action::JumpToLatest)
    );
    // The wheel over the conversation is not bound: only over a completion menu.
    assert_eq!(find(Context::Composer, MouseGesture::WheelUp), None);
    assert_eq!(
        Trigger::Mouse(MouseGesture::DragRelease).to_string(),
        "Mouse drag + release"
    );
    Ok(())
}

fn context_strategy() -> impl Strategy<Value = Context> {
    proptest::sample::select(Context::ALL.to_vec())
}

proptest! {
    #[test]
    fn prop_lookup_never_panics(
        context in context_strategy(),
        c in any::<char>(),
        ctrl in any::<bool>(),
        alt in any::<bool>(),
        shift in any::<bool>(),
        empty in any::<bool>(),
        first in any::<bool>(),
        last in any::<bool>(),
        f in any::<u8>(),
    ) {
        let mods = Modifiers { ctrl, alt, shift };
        let state = GuardState { draft_empty: empty, first_visual_row: first, last_visual_row: last };
        let _ = lookup(context, KeyChord::new(Key::Char(c), mods), state);
        let _ = lookup(context, KeyChord::new(Key::F(f), mods), state);
        let _ = footer_text(keymap(), context, state);
    }
}

#[test]
fn questionnaire_contexts_bind_atomic_answer_and_requester_controls() -> TestResult {
    use Action as A;
    use Context as C;
    assert_eq!(C::Questionnaire.chain(), &[C::Questionnaire]);
    assert_eq!(C::QuestionnaireNotes.chain(), &[C::QuestionnaireNotes]);
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Left),
        NONE,
        A::QuestionnairePrevious,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Right),
        NONE,
        A::QuestionnaireNext,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Up),
        NONE,
        A::QuestionnaireChoiceUp,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Down),
        NONE,
        A::QuestionnaireChoiceDown,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Char(' ')),
        NONE,
        A::QuestionnaireToggleChoice,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Tab),
        NONE,
        A::QuestionnaireFocusNotes,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Enter),
        NONE,
        A::SubmitQuestionnaireAnswers,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::plain(Key::Esc),
        NONE,
        A::InterruptQuestionnaireRequester,
    )?;
    resolves(
        C::Questionnaire,
        KeyChord::ctrl(Key::Char('B')),
        NONE,
        A::DeferQuestionnaire,
    )?;
    resolves(
        C::QuestionnaireNotes,
        KeyChord::shift(Key::Tab),
        NONE,
        A::QuestionnaireFocusChoices,
    )?;
    resolves(
        C::QuestionnaireNotes,
        KeyChord::plain(Key::Enter),
        NONE,
        A::SubmitQuestionnaireAnswers,
    )?;
    resolves(
        C::QuestionnaireNotes,
        KeyChord::plain(Key::Esc),
        NONE,
        A::InterruptQuestionnaireRequester,
    )?;
    resolves(
        C::QuestionnaireNotes,
        KeyChord::ctrl(Key::Char('B')),
        NONE,
        A::DeferQuestionnaire,
    )?;
    assert_eq!(
        lookup(C::QuestionnaireNotes, KeyChord::plain(Key::Left), NONE).map(|b| b.action),
        None,
        "notes keep horizontal arrows for text cursor movement"
    );
    assert_eq!(
        lookup(C::Questionnaire, KeyChord::ctrl(Key::Char('C')), NONE).map(|b| b.action),
        None,
        "questionnaire Esc is explicit requester interruption; no global cancel fallback"
    );
    Ok(())
}
