//! Terminal events → [`Action`]s. Pure, so every binding is unit-tested.

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::app::{Action, Pane};
use crate::cmdline::PromptKey;
use crate::search::Direction;

/// Lines moved per mouse-wheel notch.
pub const WHEEL_LINES: usize = 3;

/// Which interactive screen a command applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenKind {
    Text,
    Hex,
    Folder,
}

/// Sidebar / help heading a command is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Navigate,
    Search,
    Compare,
    View,
    Folder,
    General,
}

impl Group {
    /// Display order.
    pub const ALL: [Self; 6] = [
        Self::Navigate,
        Self::Search,
        Self::Compare,
        Self::View,
        Self::Folder,
        Self::General,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Navigate => "Navigate",
            Self::Search => "Search",
            Self::Compare => "Compare",
            Self::View => "View",
            Self::Folder => "Folder",
            Self::General => "General",
        }
    }
}

/// One entry of the command list shown in the sidebar and the help overlay.
#[derive(Debug)]
pub struct Command {
    /// Every key that triggers it, for the help overlay.
    pub keys: &'static str,
    /// Compact keys for the sidebar (at most [`SHORT_KEYS_W`] columns).
    pub short: &'static str,
    pub label: &'static str,
    /// Extra detail shown after the label in the help overlay only.
    pub note: &'static str,
    pub group: Group,
    /// What the keys send. The first action names the command's live value.
    pub actions: &'static [Action],
    pub screens: &'static [ScreenKind],
}

/// Widest [`Command::short`].
pub const SHORT_KEYS_W: usize = 7;

const ALL: &[ScreenKind] = &[ScreenKind::Text, ScreenKind::Hex, ScreenKind::Folder];
const TEXT: &[ScreenKind] = &[ScreenKind::Text];
const TEXT_FOLDER: &[ScreenKind] = &[ScreenKind::Text, ScreenKind::Folder];
const FOLDER: &[ScreenKind] = &[ScreenKind::Folder];
const HEX_FOLDER: &[ScreenKind] = &[ScreenKind::Hex, ScreenKind::Folder];

impl Command {
    const fn note(self, note: &'static str) -> Self {
        Self { note, ..self }
    }
}

const fn cmd(
    keys: &'static str,
    short: &'static str,
    label: &'static str,
    group: Group,
    actions: &'static [Action],
    screens: &'static [ScreenKind],
) -> Command {
    Command {
        keys,
        short,
        label,
        note: "",
        group,
        actions,
        screens,
    }
}

/// Every runtime command. The single source for the sidebar and the help
/// overlay; tests check it against the key bindings in both directions.
pub const COMMANDS: &[Command] = &[
    cmd(
        "j k / ↓ ↑ / wheel",
        "j/k",
        "down / up",
        Group::Navigate,
        &[Action::Down(1), Action::Up(1)],
        ALL,
    ),
    cmd(
        "PgDn Space Ctrl-d / PgUp Ctrl-u",
        "PgDn/Up",
        "page down / up",
        Group::Navigate,
        &[Action::PageDown, Action::PageUp],
        ALL,
    ),
    cmd(
        "g Home / G End",
        "g/G",
        "top / bottom",
        Group::Navigate,
        &[Action::Top, Action::Bottom],
        ALL,
    ),
    cmd(
        "]c / [c p",
        "]c/[c",
        "next / prev change",
        Group::Navigate,
        &[Action::NextChange, Action::PrevChange],
        TEXT,
    ),
    // No search on these screens, so n / N keep stepping through changes.
    cmd(
        "]c n / [c N p",
        "]c/[c",
        "next / prev change",
        Group::Navigate,
        &[Action::NextChange, Action::PrevChange],
        HEX_FOLDER,
    ),
    cmd(
        "h ← / l →",
        "h/l",
        "scroll left / right",
        Group::Navigate,
        &[Action::ScrollLeft, Action::ScrollRight],
        TEXT,
    ),
    cmd(
        "/ ?",
        "/ ?",
        "search fwd / back",
        Group::Search,
        &[
            Action::StartSearch(Direction::Forward),
            Action::StartSearch(Direction::Backward),
        ],
        TEXT,
    )
    .note(r"\c ignores case"),
    cmd(
        "n / N",
        "n/N",
        "next / prev match",
        Group::Search,
        &[Action::SearchNext, Action::SearchPrev],
        TEXT,
    )
    .note("N goes the opposite way"),
    cmd(
        "Tab Ctrl-w w / Ctrl-w h ← / l →",
        "Tab",
        "active pane",
        Group::Search,
        &[
            Action::SwitchPane,
            Action::FocusPane(Pane::Left),
            Action::FocusPane(Pane::Right),
        ],
        TEXT,
    )
    .note("searches look only here"),
    cmd(
        "w",
        "w",
        "whitespace",
        Group::Compare,
        &[Action::CycleWhitespace],
        TEXT_FOLDER,
    )
    .note("exact → trailing → change → all"),
    cmd(
        "i",
        "i",
        "ignore case",
        Group::Compare,
        &[Action::ToggleCase],
        TEXT_FOLDER,
    ),
    cmd(
        "b",
        "b",
        "ignore blank lines",
        Group::Compare,
        &[Action::ToggleBlankLines],
        TEXT_FOLDER,
    ),
    cmd(
        "e",
        "e",
        "ignore line endings",
        Group::Compare,
        &[Action::ToggleLineEndings],
        TEXT_FOLDER,
    )
    .note("CRLF vs LF"),
    cmd(
        "u",
        "u",
        "layout",
        Group::View,
        &[Action::ToggleLayout],
        TEXT,
    ),
    cmd(
        "Enter",
        "Enter",
        "open file diff",
        Group::Folder,
        &[Action::Open],
        FOLDER,
    )
    .note("Esc returns"),
    cmd(
        "s",
        "s",
        "identical files",
        Group::Folder,
        &[Action::ToggleSame],
        FOLDER,
    ),
    cmd(
        "F1",
        "F1",
        "toggle this sidebar",
        Group::General,
        &[Action::ToggleSidebar],
        ALL,
    )
    .note("remembered between runs"),
    cmd(
        ":",
        ":",
        "command line",
        Group::General,
        &[Action::StartCommand],
        ALL,
    )
    .note("commands below"),
    cmd(
        "q / Esc",
        "q/Esc",
        "quit or go back",
        Group::General,
        &[Action::Quit, Action::Back],
        ALL,
    )
    .note("Esc closes :help first"),
    cmd(
        "Ctrl-c",
        "Ctrl-c",
        "quit from anywhere",
        Group::General,
        &[Action::ForceQuit],
        ALL,
    ),
];

/// Commands for `screen` under `group`, in table order.
pub fn commands(screen: ScreenKind, group: Group) -> impl Iterator<Item = &'static Command> {
    COMMANDS
        .iter()
        .filter(move |c| c.group == group && c.screens.contains(&screen))
}

/// Whether keys drive the screen or edit the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InputMode {
    #[default]
    Normal,
    /// The `/`, `?` or `:` line is open: keys become [`Action::Prompt`].
    Prompt,
}

/// The first key of a two-key sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pending {
    /// `Ctrl-w`: then a pane key, as in vim.
    Window,
    /// `]` (true) or `[` (false): then `c` jumps to a change, as in vimdiff.
    Bracket(bool),
}

/// Stateful key translation: adds the key sequences (`]c` / `[c`, `Ctrl-w`
/// then a pane key) and the command-line keys on top of the single-key
/// bindings of [`action_for`].
#[derive(Debug, Default)]
pub struct KeyMap {
    pending: Option<Pending>,
}

impl KeyMap {
    pub fn action_for(&mut self, event: &Event, mode: InputMode) -> Option<Action> {
        let Event::Key(key) = event else {
            return action_for(event);
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        if mode == InputMode::Prompt {
            self.pending = None;
            return prompt_action(*key);
        }
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match self.pending.take() {
            Some(Pending::Window) => return window_action(*key),
            Some(Pending::Bracket(next)) => {
                return (key.code == KeyCode::Char('c') && !ctrl).then_some(if next {
                    Action::NextChange
                } else {
                    Action::PrevChange
                });
            }
            None => {}
        }
        match key.code {
            KeyCode::Char('w') if ctrl => self.pending = Some(Pending::Window),
            KeyCode::Char(']') if !ctrl => self.pending = Some(Pending::Bracket(true)),
            KeyCode::Char('[') if !ctrl => self.pending = Some(Pending::Bracket(false)),
            _ => {}
        }
        key_action(*key)
    }
}

/// The key after `Ctrl-w`. Vim's window keys; with two panes `w`, `W` and `p`
/// all mean "the other one". Each also works with Ctrl held (`Ctrl-w Ctrl-w`).
/// Anything else cancels the sequence.
fn window_action(key: KeyEvent) -> Option<Action> {
    Some(match key.code {
        KeyCode::Char('w' | 'W' | 'p') => Action::SwitchPane,
        // Many terminals send Ctrl-h as Backspace.
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => Action::FocusPane(Pane::Left),
        KeyCode::Char('l') | KeyCode::Right => Action::FocusPane(Pane::Right),
        _ => return None,
    })
}

/// Keys while the command line is open.
fn prompt_action(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    Some(Action::Prompt(match key.code {
        KeyCode::Char('c') if ctrl => return Some(Action::ForceQuit),
        KeyCode::Char('h') if ctrl => PromptKey::Backspace,
        KeyCode::Char('w') if ctrl => PromptKey::DeleteWord,
        KeyCode::Char('u') if ctrl => PromptKey::Clear,
        KeyCode::Char('p') if ctrl => PromptKey::HistoryPrev,
        KeyCode::Char('n') if ctrl => PromptKey::HistoryNext,
        KeyCode::Char('[') if ctrl => PromptKey::Cancel,
        _ if ctrl || alt => return None,
        KeyCode::Char(c) => PromptKey::Char(c),
        KeyCode::Backspace => PromptKey::Backspace,
        KeyCode::Enter => PromptKey::Submit,
        KeyCode::Esc => PromptKey::Cancel,
        KeyCode::Up => PromptKey::HistoryPrev,
        KeyCode::Down => PromptKey::HistoryNext,
        _ => return None,
    }))
}

/// Single-event bindings (no key sequences; see [`KeyMap`]).
pub fn action_for(event: &Event) -> Option<Action> {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Release => key_action(*key),
        Event::Mouse(MouseEvent { kind, .. }) => match kind {
            MouseEventKind::ScrollDown => Some(Action::Down(WHEEL_LINES)),
            MouseEventKind::ScrollUp => Some(Action::Up(WHEEL_LINES)),
            _ => None,
        },
        Event::Resize(width, height) => Some(Action::Resize {
            width: *width,
            height: *height,
        }),
        _ => None,
    }
}

fn key_action(key: KeyEvent) -> Option<Action> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    Some(match key.code {
        KeyCode::Char('c' | 'q') if ctrl => Action::ForceQuit,
        KeyCode::Char('d' | 'f') if ctrl => Action::PageDown,
        KeyCode::Char('u' | 'b') if ctrl => Action::PageUp,
        _ if ctrl => return None,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::Back,
        KeyCode::Char('j') | KeyCode::Down => Action::Down(1),
        KeyCode::Enter => Action::Open,
        KeyCode::Char('s') => Action::ToggleSame,
        KeyCode::Char('k') | KeyCode::Up => Action::Up(1),
        KeyCode::PageDown | KeyCode::Char(' ') => Action::PageDown,
        KeyCode::PageUp => Action::PageUp,
        KeyCode::Char('g') | KeyCode::Home => Action::Top,
        KeyCode::Char('G') | KeyCode::End => Action::Bottom,
        KeyCode::Char('n') => Action::SearchNext,
        KeyCode::Char('N') => Action::SearchPrev,
        KeyCode::Char('p') => Action::PrevChange,
        KeyCode::Char('/') => Action::StartSearch(Direction::Forward),
        KeyCode::Char('?') => Action::StartSearch(Direction::Backward),
        KeyCode::Char(':') => Action::StartCommand,
        KeyCode::Char('h') | KeyCode::Left => Action::ScrollLeft,
        KeyCode::Char('l') | KeyCode::Right => Action::ScrollRight,
        KeyCode::Char('w') => Action::CycleWhitespace,
        KeyCode::Char('i') => Action::ToggleCase,
        KeyCode::Char('b') => Action::ToggleBlankLines,
        KeyCode::Char('e') => Action::ToggleLineEndings,
        KeyCode::Char('u') => Action::ToggleLayout,
        KeyCode::F(1) => Action::ToggleSidebar,
        KeyCode::Tab => Action::SwitchPane,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use ratatui::crossterm::event::KeyEventState;
    use rstest::rstest;

    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> Event {
        Event::Key(KeyEvent {
            code,
            modifiers,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        })
    }

    const NONE: KeyModifiers = KeyModifiers::NONE;
    const CTRL: KeyModifiers = KeyModifiers::CONTROL;
    const SHIFT: KeyModifiers = KeyModifiers::SHIFT;

    #[rstest]
    #[case::q(KeyCode::Char('q'), NONE, Some(Action::Quit))]
    #[case::ctrl_c(KeyCode::Char('c'), CTRL, Some(Action::ForceQuit))]
    #[case::enter(KeyCode::Enter, NONE, Some(Action::Open))]
    #[case::s(KeyCode::Char('s'), NONE, Some(Action::ToggleSame))]
    #[case::esc(KeyCode::Esc, NONE, Some(Action::Back))]
    #[case::j(KeyCode::Char('j'), NONE, Some(Action::Down(1)))]
    #[case::down(KeyCode::Down, NONE, Some(Action::Down(1)))]
    #[case::k(KeyCode::Char('k'), NONE, Some(Action::Up(1)))]
    #[case::space(KeyCode::Char(' '), NONE, Some(Action::PageDown))]
    #[case::ctrl_d(KeyCode::Char('d'), CTRL, Some(Action::PageDown))]
    #[case::ctrl_u(KeyCode::Char('u'), CTRL, Some(Action::PageUp))]
    #[case::g(KeyCode::Char('g'), NONE, Some(Action::Top))]
    #[case::shift_g(KeyCode::Char('G'), SHIFT, Some(Action::Bottom))]
    #[case::n_repeats_search(KeyCode::Char('n'), NONE, Some(Action::SearchNext))]
    #[case::shift_n_repeats_search_backward(KeyCode::Char('N'), SHIFT, Some(Action::SearchPrev))]
    #[case::p(KeyCode::Char('p'), NONE, Some(Action::PrevChange))]
    #[case::slash(
        KeyCode::Char('/'),
        NONE,
        Some(Action::StartSearch(Direction::Forward))
    )]
    #[case::question_searches_backward(
        KeyCode::Char('?'),
        SHIFT,
        Some(Action::StartSearch(Direction::Backward))
    )]
    #[case::colon(KeyCode::Char(':'), SHIFT, Some(Action::StartCommand))]
    #[case::w(KeyCode::Char('w'), NONE, Some(Action::CycleWhitespace))]
    #[case::i(KeyCode::Char('i'), NONE, Some(Action::ToggleCase))]
    #[case::b(KeyCode::Char('b'), NONE, Some(Action::ToggleBlankLines))]
    #[case::ctrl_b_is_page_up(KeyCode::Char('b'), CTRL, Some(Action::PageUp))]
    #[case::e(KeyCode::Char('e'), NONE, Some(Action::ToggleLineEndings))]
    #[case::u(KeyCode::Char('u'), NONE, Some(Action::ToggleLayout))]
    #[case::f1_toggles_sidebar(KeyCode::F(1), NONE, Some(Action::ToggleSidebar))]
    #[case::tab_swaps_panes(KeyCode::Tab, NONE, Some(Action::SwitchPane))]
    #[case::ctrl_tab(KeyCode::Tab, CTRL, None)]
    #[case::unbound(KeyCode::Char('z'), NONE, None)]
    #[case::unbound_ctrl(KeyCode::Char('w'), CTRL, None)]
    fn keys(#[case] code: KeyCode, #[case] mods: KeyModifiers, #[case] expected: Option<Action>) {
        assert_eq!(action_for(&key(code, mods)), expected);
    }

    /// Every action a single key press, or a two-key sequence, can produce.
    fn bound_actions() -> Vec<Action> {
        let mut codes: Vec<KeyCode> = (' '..='~').map(KeyCode::Char).collect();
        codes.extend([
            KeyCode::Enter,
            KeyCode::Esc,
            KeyCode::Tab,
            KeyCode::BackTab,
            KeyCode::Backspace,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Left,
            KeyCode::Right,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Insert,
            KeyCode::Delete,
        ]);
        codes.extend((1..=12).map(KeyCode::F));
        let mut out = Vec::new();
        let mut add = |a: Option<Action>| match a {
            Some(a) if !out.contains(&a) => out.push(a),
            _ => {}
        };
        let prefixes = [
            None,
            Some(key(KeyCode::Char('w'), CTRL)),
            Some(key(KeyCode::Char(']'), NONE)),
            Some(key(KeyCode::Char('['), NONE)),
        ];
        for code in codes {
            for mods in [NONE, CTRL, SHIFT, KeyModifiers::ALT] {
                add(action_for(&key(code, mods)));
                for prefix in prefixes.iter().flatten() {
                    let mut map = KeyMap::default();
                    map.action_for(prefix, InputMode::Normal);
                    add(map.action_for(&key(code, mods), InputMode::Normal));
                }
            }
        }
        out
    }

    fn listed(action: Action) -> bool {
        COMMANDS.iter().any(|c| c.actions.contains(&action))
    }

    #[test]
    fn every_bound_key_is_listed_in_commands() {
        let missing: Vec<_> = bound_actions()
            .into_iter()
            .filter(|&a| !listed(a))
            .collect();
        assert!(missing.is_empty(), "not in COMMANDS: {missing:?}");
    }

    #[test]
    fn every_listed_command_is_bound() {
        let bound = bound_actions();
        for c in COMMANDS {
            for a in c.actions {
                assert!(bound.contains(a), "{} ({a:?}) has no key", c.label);
            }
        }
    }

    #[test]
    fn short_keys_fit_the_sidebar_column() {
        for c in COMMANDS {
            assert!(
                mydiff_core::display::width(c.short) <= SHORT_KEYS_W,
                "{}",
                c.short
            );
        }
    }

    /// Feed `events` through one [`KeyMap`] and collect the actions.
    fn run_keymap(events: &[Event]) -> Vec<Option<Action>> {
        run_keymap_in(InputMode::Normal, events)
    }

    fn run_keymap_in(mode: InputMode, events: &[Event]) -> Vec<Option<Action>> {
        let mut map = KeyMap::default();
        events.iter().map(|e| map.action_for(e, mode)).collect()
    }

    fn ch(c: char) -> Event {
        key(KeyCode::Char(c), NONE)
    }

    #[test]
    fn bracket_c_jumps_between_changes() {
        assert_eq!(
            run_keymap(&[ch(']'), ch('c'), ch('['), ch('c')]),
            [
                None,
                Some(Action::NextChange),
                None,
                Some(Action::PrevChange)
            ]
        );
    }

    #[test]
    fn bracket_then_other_key_is_cancelled() {
        assert_eq!(
            run_keymap(&[ch(']'), ch('j'), ch('j')]),
            [None, None, Some(Action::Down(1))],
            "vim drops an unknown ]x, then keys work again"
        );
    }

    #[rstest]
    #[case::w(ch('w'), Some(Action::SwitchPane))]
    #[case::ctrl_w(key(KeyCode::Char('w'), CTRL), Some(Action::SwitchPane))]
    #[case::shift_w(key(KeyCode::Char('W'), SHIFT), Some(Action::SwitchPane))]
    #[case::p(ch('p'), Some(Action::SwitchPane))]
    #[case::h(ch('h'), Some(Action::FocusPane(Pane::Left)))]
    #[case::ctrl_h(key(KeyCode::Char('h'), CTRL), Some(Action::FocusPane(Pane::Left)))]
    #[case::ctrl_h_as_backspace(key(KeyCode::Backspace, NONE), Some(Action::FocusPane(Pane::Left)))]
    #[case::left(key(KeyCode::Left, NONE), Some(Action::FocusPane(Pane::Left)))]
    #[case::l(ch('l'), Some(Action::FocusPane(Pane::Right)))]
    #[case::ctrl_l(key(KeyCode::Char('l'), CTRL), Some(Action::FocusPane(Pane::Right)))]
    #[case::right(key(KeyCode::Right, NONE), Some(Action::FocusPane(Pane::Right)))]
    #[case::other_key_cancels(ch('x'), None)]
    #[case::esc_cancels(key(KeyCode::Esc, NONE), None)]
    fn ctrl_w_then_window_key(#[case] second: Event, #[case] expected: Option<Action>) {
        assert_eq!(
            run_keymap(&[key(KeyCode::Char('w'), CTRL), second]),
            [None, expected]
        );
    }

    #[test]
    fn ctrl_w_sequence_ends_after_one_key() {
        assert_eq!(
            run_keymap(&[key(KeyCode::Char('w'), CTRL), ch('l'), ch('l')]),
            [
                None,
                Some(Action::FocusPane(Pane::Right)),
                Some(Action::ScrollRight)
            ]
        );
    }

    #[rstest]
    #[case::letter(ch('n'), Some(Action::Prompt(PromptKey::Char('n'))))]
    #[case::question_mark(
        key(KeyCode::Char('?'), SHIFT),
        Some(Action::Prompt(PromptKey::Char('?')))
    )]
    #[case::backslash(ch('\\'), Some(Action::Prompt(PromptKey::Char('\\'))))]
    #[case::space(ch(' '), Some(Action::Prompt(PromptKey::Char(' '))))]
    #[case::enter(key(KeyCode::Enter, NONE), Some(Action::Prompt(PromptKey::Submit)))]
    #[case::esc(key(KeyCode::Esc, NONE), Some(Action::Prompt(PromptKey::Cancel)))]
    #[case::ctrl_bracket(key(KeyCode::Char('['), CTRL), Some(Action::Prompt(PromptKey::Cancel)))]
    #[case::backspace(
        key(KeyCode::Backspace, NONE),
        Some(Action::Prompt(PromptKey::Backspace))
    )]
    #[case::ctrl_h(
        key(KeyCode::Char('h'), CTRL),
        Some(Action::Prompt(PromptKey::Backspace))
    )]
    #[case::ctrl_w(
        key(KeyCode::Char('w'), CTRL),
        Some(Action::Prompt(PromptKey::DeleteWord))
    )]
    #[case::ctrl_u(key(KeyCode::Char('u'), CTRL), Some(Action::Prompt(PromptKey::Clear)))]
    #[case::up(key(KeyCode::Up, NONE), Some(Action::Prompt(PromptKey::HistoryPrev)))]
    #[case::down(key(KeyCode::Down, NONE), Some(Action::Prompt(PromptKey::HistoryNext)))]
    #[case::ctrl_p(
        key(KeyCode::Char('p'), CTRL),
        Some(Action::Prompt(PromptKey::HistoryPrev))
    )]
    #[case::ctrl_n(
        key(KeyCode::Char('n'), CTRL),
        Some(Action::Prompt(PromptKey::HistoryNext))
    )]
    #[case::ctrl_c_still_quits(key(KeyCode::Char('c'), CTRL), Some(Action::ForceQuit))]
    #[case::other_ctrl_ignored(key(KeyCode::Char('x'), CTRL), None)]
    #[case::alt_ignored(key(KeyCode::Char('x'), KeyModifiers::ALT), None)]
    #[case::tab_ignored(key(KeyCode::Tab, NONE), None)]
    fn prompt_keys(#[case] event: Event, #[case] expected: Option<Action>) {
        assert_eq!(run_keymap_in(InputMode::Prompt, &[event]), [expected]);
    }

    #[test]
    fn prompt_mode_drops_a_pending_sequence() {
        let mut map = KeyMap::default();
        map.action_for(&ch(']'), InputMode::Normal);
        map.action_for(&ch('x'), InputMode::Prompt);
        assert_eq!(
            map.action_for(&ch('c'), InputMode::Normal),
            None,
            "c alone is unbound"
        );
    }

    #[test]
    fn mouse_wheel_scrolls_while_typing() {
        let wheel = Event::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 0,
            row: 0,
            modifiers: NONE,
        });
        assert_eq!(
            run_keymap_in(InputMode::Prompt, &[wheel]),
            [Some(Action::Down(WHEEL_LINES))]
        );
    }

    #[test]
    fn tab_swaps_panes_every_press() {
        let tab = key(KeyCode::Tab, NONE);
        assert_eq!(
            run_keymap(&[tab.clone(), tab.clone(), tab]),
            [Some(Action::SwitchPane); 3],
            "one key, no sequence"
        );
    }

    #[test]
    fn arrows_always_scroll() {
        let tab = key(KeyCode::Tab, NONE);
        assert_eq!(
            run_keymap(&[tab, key(KeyCode::Left, NONE), key(KeyCode::Right, NONE)]),
            [
                Some(Action::SwitchPane),
                Some(Action::ScrollLeft),
                Some(Action::ScrollRight),
            ],
            "Tab no longer starts a docking sequence"
        );
    }

    #[test]
    fn key_release_and_mouse_moves_do_not_break_a_sequence() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Char('w'),
            modifiers: CTRL,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        let moved = Event::Mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 3,
            row: 3,
            modifiers: NONE,
        });
        assert_eq!(
            run_keymap(&[
                key(KeyCode::Char('w'), CTRL),
                release,
                moved,
                key(KeyCode::Char('l'), NONE)
            ]),
            [None, None, None, Some(Action::FocusPane(Pane::Right))]
        );
    }

    #[test]
    fn key_release_ignored() {
        let ev = Event::Key(KeyEvent {
            code: KeyCode::Char('q'),
            modifiers: NONE,
            kind: KeyEventKind::Release,
            state: KeyEventState::NONE,
        });
        assert_eq!(action_for(&ev), None);
    }

    #[test]
    fn mouse_wheel_scrolls() {
        let wheel = |kind| {
            Event::Mouse(MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: NONE,
            })
        };
        assert_eq!(
            action_for(&wheel(MouseEventKind::ScrollDown)),
            Some(Action::Down(WHEEL_LINES))
        );
        assert_eq!(
            action_for(&wheel(MouseEventKind::ScrollUp)),
            Some(Action::Up(WHEEL_LINES))
        );
    }

    #[test]
    fn resize_event() {
        assert_eq!(
            action_for(&Event::Resize(100, 40)),
            Some(Action::Resize {
                width: 100,
                height: 40
            })
        );
    }
}
