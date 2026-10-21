//! The keymap as data: which key (or mouse gesture), in which [`Context`], triggers which
//! [`Action`]. Frontends translate native key events into a frontend-neutral [`KeyChord`] and call
//! [`lookup`]; the footer hints and the help screen are generated from the same table
//! ([`footer_text`], [`help_lines`]), so they cannot drift from the bindings.
//!
//! Source: the "Full keymap" table of `polaris-cli` in the PLAN. Two deliberate additions that
//! the table does not state: undo and redo are bound to `Ctrl+Z` / `Ctrl+Y` (the PLAN says the
//! chords must be assigned at implementation; they are UNVERIFIED against real terminals), and
//! `Shift+Enter` and `Ctrl+Enter` depend on the terminal (spike S0-10), which is why `Alt+Enter`
//! and `Ctrl+J` exist as well.

use core::fmt;

macro_rules! actions {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {
        /// Something the user can ask the UI to do.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Action {
            $($(#[$doc])* $name),+
        }

        impl Action {
            /// Every action, so tests can prove each is bound or explicitly unbound.
            pub const ALL: &'static [Self] = &[$(Self::$name),+];
        }
    };
}

actions! {
    /// Send the draft.
    Submit,
    /// Insert a line break into the draft.
    Newline,
    /// Recall the previous (older) prompt.
    HistoryPrev,
    /// Recall the next (newer) prompt, or the unsent draft after the newest.
    HistoryNext,
    /// Switch between auto and plan mode.
    ToggleMode,
    /// Raise the active model's effort.
    EffortUp,
    /// Lower the active model's effort.
    EffortDown,
    /// Interrupt the running work (after dismissing any popup, menu, preview or selection).
    Interrupt,
    /// Interrupt work, close a popup or clear the draft; pressed twice on an empty draft it exits.
    InterruptOrExit,
    /// Go back from a popup without applying a selection.
    PopupBack,
    /// Close an information popup.
    ClosePopup,
    /// Confirm the highlighted entry of a selection menu.
    ConfirmSelection,
    /// Open the shortcuts help.
    OpenHelp,
    /// Open the status popup.
    OpenStatus,
    /// Scroll the conversation or popup up one page.
    ScrollPageUp,
    /// Scroll the conversation or popup down one page.
    ScrollPageDown,
    /// Move the completion menu highlight up one entry.
    MenuPrev,
    /// Move the completion menu highlight down one entry.
    MenuNext,
    /// Expand or collapse all tool bodies.
    ToggleTools,
    /// Provider form: next field.
    NextField,
    /// Provider form: previous field.
    PrevField,
    /// Provider form: save the settings.
    SaveSettings,
    /// `/model`: choose a provider.
    ChooseProvider,
    /// `/model`: configure the provider.
    ConfigureProvider,
    /// `/stats`: switch between Overview and Models.
    SwitchStatsTab,
    /// `/stats`: cycle the range (all time, 7 days, 30 days).
    CycleStatsRange,
    /// Paste from the clipboard (image first, else text).
    Paste,
    /// Text-attachment preview: expand the attachment into the draft.
    ExpandAttachment,
    /// Text-attachment preview: copy the attachment.
    CopyAttachment,
    /// Select with the mouse and copy the semantic source text on release.
    SelectCopy,
    /// Select a word, then extend by whole words.
    SelectWord,
    /// Expand or collapse one tool entry.
    ToggleToolEntry,
    /// Jump to the bottom of the conversation.
    JumpToLatest,
    /// Undo the last composer edit.
    Undo,
    /// Redo the last undone composer edit.
    Redo,
    /// Leave the application.
    Quit,
}

/// Actions that have no key or mouse binding on purpose, with the reason.
pub const UNBOUND: &[(Action, &str)] = &[(
    Action::Quit,
    "reached through /quit, /exit and the double Ctrl+C rule of InterruptOrExit, not a key of its own",
)];

/// A non-modifier key, independent of any terminal or GUI toolkit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    /// Enter / Return.
    Enter,
    /// Escape.
    Esc,
    /// Tab (Shift+Tab is `Tab` with the shift modifier).
    Tab,
    /// Cursor up.
    Up,
    /// Cursor down.
    Down,
    /// Cursor left.
    Left,
    /// Cursor right.
    Right,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// A function key, `F(2)` is F2.
    F(u8),
    /// A character key.
    Char(char),
}

/// Held modifier keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Modifiers {
    /// Control.
    pub ctrl: bool,
    /// Alt / Option.
    pub alt: bool,
    /// Shift.
    pub shift: bool,
}

impl Modifiers {
    /// No modifier.
    pub const NONE: Self = Self {
        ctrl: false,
        alt: false,
        shift: false,
    };
}

/// A key plus modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyChord {
    /// The key.
    pub key: Key,
    /// The modifiers held with it.
    pub mods: Modifiers,
}

impl KeyChord {
    /// A chord from its parts.
    #[must_use]
    pub const fn new(key: Key, mods: Modifiers) -> Self {
        Self { key, mods }
    }

    /// The key alone.
    #[must_use]
    pub const fn plain(key: Key) -> Self {
        Self::new(key, Modifiers::NONE)
    }

    /// The key with Ctrl.
    #[must_use]
    pub const fn ctrl(key: Key) -> Self {
        Self::new(
            key,
            Modifiers {
                ctrl: true,
                alt: false,
                shift: false,
            },
        )
    }

    /// The key with Alt.
    #[must_use]
    pub const fn alt(key: Key) -> Self {
        Self::new(
            key,
            Modifiers {
                ctrl: false,
                alt: true,
                shift: false,
            },
        )
    }

    /// The key with Shift.
    #[must_use]
    pub const fn shift(key: Key) -> Self {
        Self::new(
            key,
            Modifiers {
                ctrl: false,
                alt: false,
                shift: true,
            },
        )
    }

    /// The canonical form used for matching: for character keys Shift is dropped (the character
    /// already carries it) and letters are lower-cased, so `Shift+P`, `P` and `p` all compare
    /// equal. Other keys keep their modifiers.
    #[must_use]
    pub fn normalized(self) -> Self {
        match self.key {
            Key::Char(c) => {
                let lowered = if c.is_alphabetic() {
                    match c.to_lowercase().next() {
                        Some(lowered) => lowered,
                        None => c,
                    }
                } else {
                    c
                };
                Self::new(
                    Key::Char(lowered),
                    Modifiers {
                        shift: false,
                        ..self.mods
                    },
                )
            }
            _ => self,
        }
    }
}

impl fmt::Display for KeyChord {
    /// `Ctrl+Alt+Shift+Key`; letters are shown upper-case (`Ctrl+O`, `P`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.mods.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.mods.alt {
            f.write_str("Alt+")?;
        }
        if self.mods.shift {
            f.write_str("Shift+")?;
        }
        match self.key {
            Key::Enter => f.write_str("Enter"),
            Key::Esc => f.write_str("Esc"),
            Key::Tab => f.write_str("Tab"),
            Key::Up => f.write_str("Up"),
            Key::Down => f.write_str("Down"),
            Key::Left => f.write_str("Left"),
            Key::Right => f.write_str("Right"),
            Key::PageUp => f.write_str("PageUp"),
            Key::PageDown => f.write_str("PageDown"),
            Key::Insert => f.write_str("Insert"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Char(' ') => f.write_str("Space"),
            Key::Char(c) => {
                for upper in c.to_uppercase() {
                    write!(f, "{upper}")?;
                }
                Ok(())
            }
        }
    }
}

/// A mouse gesture a frontend can recognise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MouseGesture {
    /// Wheel notch away from the user.
    WheelUp,
    /// Wheel notch towards the user.
    WheelDown,
    /// Press, drag and release over text.
    DragRelease,
    /// Double-click, then drag.
    DoubleClickDrag,
    /// Click on a tool entry's title.
    ClickToolTitle,
    /// Click on the `To latest` control.
    ClickToLatest,
}

impl fmt::Display for MouseGesture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::WheelUp => "Mouse wheel up",
            Self::WheelDown => "Mouse wheel down",
            Self::DragRelease => "Mouse drag + release",
            Self::DoubleClickDrag => "Double-click, then drag",
            Self::ClickToolTitle => "Click tool title",
            Self::ClickToLatest => "Click To latest",
        })
    }
}

/// What triggers a binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    /// A key chord (stored in normalised form).
    Key(KeyChord),
    /// A mouse gesture.
    Mouse(MouseGesture),
}

impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(chord) => chord.fmt(f),
            Self::Mouse(gesture) => gesture.fmt(f),
        }
    }
}

macro_rules! contexts {
    ($($(#[$doc:meta])* $name:ident),+ $(,)?) => {
        /// Where a key arrived; decides which bindings apply (see [`Context::chain`]).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Context {
            $($(#[$doc])* $name),+
        }

        impl Context {
            /// Every context.
            pub const ALL: &'static [Self] = &[$(Self::$name),+];
        }
    };
}

contexts! {
    /// Bindings that apply everywhere.
    Global,
    /// Editing the draft.
    Composer,
    /// The agent is working (the composer stays usable).
    Working,
    /// Any popup or bottom sheet.
    Popup,
    /// Information popups: status, usage, stats, diff, transcript, help.
    InfoPopup,
    /// Selection menus.
    SelectionMenu,
    /// The `/`, `@` and `$` completion menu.
    CompletionMenu,
    /// The `/provider` form.
    ProviderForm,
    /// The `/model` sheet.
    ModelSheet,
    /// The `/stats` sheet.
    StatsSheet,
    /// The text-attachment preview.
    AttachmentPreview,
}

impl Context {
    /// The contexts consulted, most specific first, when a key arrives in `self`. Plain
    /// characters that match nothing are typed into the focused editor by the frontend.
    #[must_use]
    pub const fn chain(self) -> &'static [Self] {
        match self {
            Self::Global => &[Self::Global],
            Self::Composer => &[Self::Composer, Self::Global],
            Self::Working => &[Self::Working, Self::Composer, Self::Global],
            Self::Popup => &[Self::Popup, Self::Global],
            Self::InfoPopup => &[Self::InfoPopup, Self::Popup, Self::Global],
            Self::SelectionMenu => &[Self::SelectionMenu, Self::Popup, Self::Global],
            Self::CompletionMenu => &[
                Self::CompletionMenu,
                Self::SelectionMenu,
                Self::Popup,
                Self::Composer,
                Self::Global,
            ],
            Self::ProviderForm => &[Self::ProviderForm, Self::Popup, Self::Global],
            Self::ModelSheet => &[
                Self::ModelSheet,
                Self::SelectionMenu,
                Self::Popup,
                Self::Global,
            ],
            Self::StatsSheet => &[Self::StatsSheet, Self::InfoPopup, Self::Popup, Self::Global],
            Self::AttachmentPreview => &[Self::AttachmentPreview, Self::Popup, Self::Global],
        }
    }

    /// Title used by the generated help.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Global => "Everywhere",
            Self::Composer => "Composer",
            Self::Working => "While working",
            Self::Popup => "Popups",
            Self::InfoPopup => "Info popups",
            Self::SelectionMenu => "Selection menus",
            Self::CompletionMenu => "Completion menu",
            Self::ProviderForm => "Provider form",
            Self::ModelSheet => "/model",
            Self::StatsSheet => "/stats",
            Self::AttachmentPreview => "Text attachment",
        }
    }
}

/// A condition on the editor state that a binding needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Guard {
    /// No condition.
    Always,
    /// Only while the draft is empty (`?` opens help).
    EmptyDraft,
    /// Only when the cursor is on the first visual row of the composer.
    FirstVisualRow,
    /// Only when the cursor is on the last visual row of the composer.
    LastVisualRow,
}

/// The editor facts a frontend reports with a key so guards can be evaluated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GuardState {
    /// The draft has no text and no attachment.
    pub draft_empty: bool,
    /// The cursor is on the first visual row.
    pub first_visual_row: bool,
    /// The cursor is on the last visual row.
    pub last_visual_row: bool,
}

impl Guard {
    /// Whether the guard holds in `state`.
    #[must_use]
    pub const fn holds(self, state: GuardState) -> bool {
        match self {
            Self::Always => true,
            Self::EmptyDraft => state.draft_empty,
            Self::FirstVisualRow => state.first_visual_row,
            Self::LastVisualRow => state.last_visual_row,
        }
    }
}

/// One row of the keymap table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyBinding {
    /// The context in which the binding applies.
    pub context: Context,
    /// The key or gesture.
    pub trigger: Trigger,
    /// Extra condition on the editor state.
    pub guard: Guard,
    /// What it does.
    pub action: Action,
    /// One-line description for the help screen.
    pub description: &'static str,
    /// Label for the footer hint (`<chord> <label>`); `None` keeps the binding out of the footer.
    pub footer: Option<&'static str>,
}

const fn key(
    context: Context,
    chord: KeyChord,
    guard: Guard,
    action: Action,
    description: &'static str,
    footer: Option<&'static str>,
) -> KeyBinding {
    KeyBinding {
        context,
        trigger: Trigger::Key(chord),
        guard,
        action,
        description,
        footer,
    }
}

const fn mouse(
    context: Context,
    gesture: MouseGesture,
    action: Action,
    description: &'static str,
) -> KeyBinding {
    KeyBinding {
        context,
        trigger: Trigger::Mouse(gesture),
        guard: Guard::Always,
        action,
        description,
        footer: None,
    }
}

const fn ch(c: char) -> KeyChord {
    KeyChord::plain(Key::Char(c))
}

const fn ctrl_ch(c: char) -> KeyChord {
    KeyChord::ctrl(Key::Char(c))
}

use Action as A;
use Context as C;
use Guard::{Always, EmptyDraft, FirstVisualRow, LastVisualRow};

/// The complete keymap. Character keys use their lower-case form (see [`KeyChord::normalized`]).
const BINDINGS: &[KeyBinding] = &[
    // Composer
    key(
        C::Composer,
        KeyChord::plain(Key::Enter),
        Always,
        A::Submit,
        "Send the draft",
        Some("send"),
    ),
    key(
        C::Composer,
        KeyChord::shift(Key::Enter),
        Always,
        A::Newline,
        "Insert a line break",
        Some("newline"),
    ),
    key(
        C::Composer,
        KeyChord::alt(Key::Enter),
        Always,
        A::Newline,
        "Insert a line break",
        None,
    ),
    key(
        C::Composer,
        KeyChord::ctrl(Key::Enter),
        Always,
        A::Newline,
        "Insert a line break",
        None,
    ),
    key(
        C::Composer,
        ctrl_ch('j'),
        Always,
        A::Newline,
        "Insert a line break",
        None,
    ),
    key(
        C::Composer,
        KeyChord::plain(Key::Up),
        FirstVisualRow,
        A::HistoryPrev,
        "Older prompt (cursor on the first row)",
        None,
    ),
    key(
        C::Composer,
        KeyChord::plain(Key::Down),
        LastVisualRow,
        A::HistoryNext,
        "Newer prompt, then the unsent draft (cursor on the last row)",
        None,
    ),
    key(
        C::Composer,
        KeyChord::shift(Key::Tab),
        Always,
        A::ToggleMode,
        "Toggle auto and plan mode",
        Some("mode"),
    ),
    key(
        C::Composer,
        KeyChord::shift(Key::Up),
        Always,
        A::EffortUp,
        "Raise the model effort",
        None,
    ),
    key(
        C::Composer,
        KeyChord::shift(Key::Down),
        Always,
        A::EffortDown,
        "Lower the model effort",
        None,
    ),
    key(
        C::Composer,
        ch('?'),
        EmptyDraft,
        A::OpenHelp,
        "Open the shortcuts (empty draft)",
        Some("shortcuts"),
    ),
    key(
        C::Composer,
        ctrl_ch('o'),
        Always,
        A::ToggleTools,
        "Expand or collapse all tool bodies",
        Some("tools"),
    ),
    key(
        C::Composer,
        ctrl_ch('v'),
        Always,
        A::Paste,
        "Paste (image first, else text)",
        None,
    ),
    key(
        C::Composer,
        KeyChord::shift(Key::Insert),
        Always,
        A::Paste,
        "Paste (image first, else text)",
        None,
    ),
    key(C::Composer, ctrl_ch('z'), Always, A::Undo, "Undo", None),
    key(C::Composer, ctrl_ch('y'), Always, A::Redo, "Redo", None),
    // While working
    key(
        C::Working,
        KeyChord::plain(Key::Esc),
        Always,
        A::Interrupt,
        "Interrupt the work (the draft is kept)",
        Some("interrupt"),
    ),
    // Popups
    key(
        C::Popup,
        KeyChord::plain(Key::Esc),
        Always,
        A::PopupBack,
        "Back to the previous view without applying a selection",
        Some("back"),
    ),
    key(
        C::InfoPopup,
        KeyChord::plain(Key::Enter),
        Always,
        A::ClosePopup,
        "Close the popup",
        Some("close"),
    ),
    key(
        C::SelectionMenu,
        KeyChord::plain(Key::Enter),
        Always,
        A::ConfirmSelection,
        "Confirm the highlighted entry",
        Some("confirm"),
    ),
    mouse(
        C::CompletionMenu,
        MouseGesture::WheelUp,
        A::MenuPrev,
        "Previous suggestion (one per notch, clamped)",
    ),
    mouse(
        C::CompletionMenu,
        MouseGesture::WheelDown,
        A::MenuNext,
        "Next suggestion (one per notch, clamped)",
    ),
    // Provider form
    key(
        C::ProviderForm,
        KeyChord::plain(Key::Tab),
        Always,
        A::NextField,
        "Next field",
        Some("next field"),
    ),
    key(
        C::ProviderForm,
        KeyChord::shift(Key::Tab),
        Always,
        A::PrevField,
        "Previous field",
        Some("previous field"),
    ),
    key(
        C::ProviderForm,
        ctrl_ch('s'),
        Always,
        A::SaveSettings,
        "Save the settings",
        Some("save"),
    ),
    // /model
    key(
        C::ModelSheet,
        ch('p'),
        Always,
        A::ChooseProvider,
        "Choose the provider",
        Some("provider"),
    ),
    key(
        C::ModelSheet,
        ch('c'),
        Always,
        A::ConfigureProvider,
        "Configure the provider",
        Some("configure"),
    ),
    key(
        C::ModelSheet,
        KeyChord::plain(Key::Left),
        Always,
        A::EffortDown,
        "Lower the effort",
        Some("less effort"),
    ),
    key(
        C::ModelSheet,
        KeyChord::plain(Key::Right),
        Always,
        A::EffortUp,
        "Raise the effort",
        Some("more effort"),
    ),
    // /stats
    key(
        C::StatsSheet,
        KeyChord::plain(Key::Tab),
        Always,
        A::SwitchStatsTab,
        "Switch between Overview and Models",
        Some("switch tab"),
    ),
    key(
        C::StatsSheet,
        KeyChord::plain(Key::Left),
        Always,
        A::SwitchStatsTab,
        "Switch between Overview and Models",
        None,
    ),
    key(
        C::StatsSheet,
        KeyChord::plain(Key::Right),
        Always,
        A::SwitchStatsTab,
        "Switch between Overview and Models",
        None,
    ),
    key(
        C::StatsSheet,
        ch('r'),
        Always,
        A::CycleStatsRange,
        "Cycle All time, 7 days, 30 days",
        Some("range"),
    ),
    // Text-attachment preview
    key(
        C::AttachmentPreview,
        ch('e'),
        Always,
        A::ExpandAttachment,
        "Expand the attachment into the draft",
        Some("expand"),
    ),
    key(
        C::AttachmentPreview,
        ch('c'),
        Always,
        A::CopyAttachment,
        "Copy the attachment",
        Some("copy"),
    ),
    // Everywhere
    key(
        C::Global,
        ctrl_ch('c'),
        Always,
        A::InterruptOrExit,
        "Interrupt work, close a popup, clear the draft; twice on an empty draft exits",
        None,
    ),
    key(
        C::Global,
        KeyChord::plain(Key::F(2)),
        Always,
        A::OpenStatus,
        "Open the status popup",
        None,
    ),
    key(
        C::Global,
        KeyChord::plain(Key::PageUp),
        Always,
        A::ScrollPageUp,
        "Scroll up one page",
        None,
    ),
    key(
        C::Global,
        KeyChord::plain(Key::PageDown),
        Always,
        A::ScrollPageDown,
        "Scroll down one page",
        None,
    ),
    mouse(
        C::Global,
        MouseGesture::DragRelease,
        A::SelectCopy,
        "Select and copy the semantic source text on release",
    ),
    mouse(
        C::Global,
        MouseGesture::DoubleClickDrag,
        A::SelectWord,
        "Select a word, then extend by whole words",
    ),
    mouse(
        C::Global,
        MouseGesture::ClickToolTitle,
        A::ToggleToolEntry,
        "Expand or collapse that tool entry",
    ),
    mouse(
        C::Global,
        MouseGesture::ClickToLatest,
        A::JumpToLatest,
        "Jump to the bottom of the conversation",
    ),
];

/// The keymap table.
#[must_use]
pub const fn keymap() -> &'static [KeyBinding] {
    BINDINGS
}

/// The binding for a key arriving in `context` when the editor is in `state`. The contexts of
/// [`Context::chain`] are tried in order; the first matching binding wins. `None` means the key
/// is not a shortcut here (plain characters are then typed).
#[must_use]
pub fn lookup(context: Context, chord: KeyChord, state: GuardState) -> Option<&'static KeyBinding> {
    let wanted = Trigger::Key(chord.normalized());
    context.chain().iter().find_map(|ctx| {
        BINDINGS
            .iter()
            .find(|b| b.context == *ctx && b.trigger == wanted && b.guard.holds(state))
    })
}

/// The binding for a mouse gesture recognised in `context`.
#[must_use]
pub fn lookup_mouse(context: Context, gesture: MouseGesture) -> Option<&'static KeyBinding> {
    let wanted = Trigger::Mouse(gesture);
    context.chain().iter().find_map(|ctx| {
        BINDINGS
            .iter()
            .find(|b| b.context == *ctx && b.trigger == wanted)
    })
}

/// The footer hint line for `context`, generated from `bindings`: for every action with a footer
/// label in the context chain (the `Global` context is not shown), the first chord and the label,
/// joined by ` · `. Hints guarded by the draft state appear only when the guard holds.
#[must_use]
pub fn footer_text(bindings: &[KeyBinding], context: Context, state: GuardState) -> String {
    let mut seen: Vec<Action> = Vec::new();
    let mut hints: Vec<String> = Vec::new();
    for ctx in context.chain().iter().filter(|c| **c != Context::Global) {
        for b in bindings.iter().filter(|b| b.context == *ctx) {
            let Some(label) = b.footer else { continue };
            if !b.guard.holds(state) || seen.contains(&b.action) {
                continue;
            }
            seen.push(b.action);
            hints.push(format!("{} {label}", b.trigger));
        }
    }
    hints.join(" · ")
}

/// Help lines generated from `bindings`: one line per (context, action) listing every chord,
/// in table order, as `[<context title>] <chords>  <description>`.
#[must_use]
pub fn help_lines(bindings: &[KeyBinding]) -> Vec<String> {
    let mut groups: Vec<(Context, Action, Vec<String>, &'static str)> = Vec::new();
    for b in bindings {
        match groups
            .iter_mut()
            .find(|(ctx, action, _, _)| *ctx == b.context && *action == b.action)
        {
            Some((_, _, triggers, _)) => triggers.push(b.trigger.to_string()),
            None => groups.push((
                b.context,
                b.action,
                vec![b.trigger.to_string()],
                b.description,
            )),
        }
    }
    groups
        .into_iter()
        .map(|(ctx, _, triggers, description)| {
            format!("[{}] {}  {description}", ctx.title(), triggers.join(" / "))
        })
        .collect()
}
