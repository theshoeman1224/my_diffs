//! Terminal events → [`Action`]s. Pure, so every binding is unit-tested.

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::app::{Action, Dock};

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
    Compare,
    View,
    Folder,
    General,
}

impl Group {
    /// Display order.
    pub const ALL: [Self; 5] = [
        Self::Navigate,
        Self::Compare,
        Self::View,
        Self::Folder,
        Self::General,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Navigate => "Navigate",
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
        "n / p N",
        "n/p",
        "next / prev change",
        Group::Navigate,
        &[Action::NextChange, Action::PrevChange],
        ALL,
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
        "Tab",
        "Tab",
        "toggle this sidebar",
        Group::General,
        &[Action::ToggleSidebar],
        ALL,
    ),
    cmd(
        "Tab then ← / →",
        "Tab ←/→",
        "dock left / right",
        Group::General,
        &[
            Action::DockSidebar(Dock::Left),
            Action::DockSidebar(Dock::Right),
        ],
        ALL,
    ),
    cmd(
        "?",
        "?",
        "toggle help",
        Group::General,
        &[Action::ToggleHelp],
        ALL,
    ),
    cmd(
        "q / Esc",
        "q/Esc",
        "quit or go back",
        Group::General,
        &[Action::Quit, Action::Back],
        ALL,
    )
    .note("Esc closes help first"),
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

/// Stateful key translation: adds the `Tab` then `←`/`→` sequence (dock the
/// sidebar) on top of the single-key bindings of [`action_for`].
#[derive(Debug, Default)]
pub struct KeyMap {
    after_tab: bool,
}

impl KeyMap {
    pub fn action_for(&mut self, event: &Event) -> Option<Action> {
        let Event::Key(key) = event else {
            return action_for(event);
        };
        if key.kind == KeyEventKind::Release {
            return None;
        }
        let plain = key.modifiers.is_empty();
        let after_tab = std::mem::replace(&mut self.after_tab, plain && key.code == KeyCode::Tab);
        match key.code {
            KeyCode::Left if after_tab && plain => Some(Action::DockSidebar(Dock::Left)),
            KeyCode::Right if after_tab && plain => Some(Action::DockSidebar(Dock::Right)),
            _ => key_action(*key),
        }
    }
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
        KeyCode::Char('n') => Action::NextChange,
        KeyCode::Char('p' | 'N') => Action::PrevChange,
        KeyCode::Char('h') | KeyCode::Left => Action::ScrollLeft,
        KeyCode::Char('l') | KeyCode::Right => Action::ScrollRight,
        KeyCode::Char('w') => Action::CycleWhitespace,
        KeyCode::Char('i') => Action::ToggleCase,
        KeyCode::Char('b') => Action::ToggleBlankLines,
        KeyCode::Char('e') => Action::ToggleLineEndings,
        KeyCode::Char('u') => Action::ToggleLayout,
        KeyCode::Char('?') => Action::ToggleHelp,
        KeyCode::Tab => Action::ToggleSidebar,
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
    #[case::n(KeyCode::Char('n'), NONE, Some(Action::NextChange))]
    #[case::shift_n(KeyCode::Char('N'), SHIFT, Some(Action::PrevChange))]
    #[case::w(KeyCode::Char('w'), NONE, Some(Action::CycleWhitespace))]
    #[case::i(KeyCode::Char('i'), NONE, Some(Action::ToggleCase))]
    #[case::b(KeyCode::Char('b'), NONE, Some(Action::ToggleBlankLines))]
    #[case::ctrl_b_is_page_up(KeyCode::Char('b'), CTRL, Some(Action::PageUp))]
    #[case::e(KeyCode::Char('e'), NONE, Some(Action::ToggleLineEndings))]
    #[case::u(KeyCode::Char('u'), NONE, Some(Action::ToggleLayout))]
    #[case::help(KeyCode::Char('?'), SHIFT, Some(Action::ToggleHelp))]
    #[case::tab(KeyCode::Tab, NONE, Some(Action::ToggleSidebar))]
    #[case::ctrl_tab(KeyCode::Tab, CTRL, None)]
    #[case::unbound(KeyCode::Char('z'), NONE, None)]
    #[case::unbound_ctrl(KeyCode::Char('w'), CTRL, None)]
    fn keys(#[case] code: KeyCode, #[case] mods: KeyModifiers, #[case] expected: Option<Action>) {
        assert_eq!(action_for(&key(code, mods)), expected);
    }

    /// Every action a single key press can produce.
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
        for code in codes {
            for mods in [NONE, CTRL, SHIFT, KeyModifiers::ALT] {
                match action_for(&key(code, mods)) {
                    Some(a) if !out.contains(&a) => out.push(a),
                    _ => {}
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
                let reachable = bound.contains(a) || matches!(a, Action::DockSidebar(_));
                assert!(reachable, "{} ({a:?}) has no key", c.label);
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
        let mut map = KeyMap::default();
        events.iter().map(|e| map.action_for(e)).collect()
    }

    #[test]
    fn tab_then_arrow_docks_sidebar() {
        let tab = key(KeyCode::Tab, NONE);
        let left = key(KeyCode::Left, NONE);
        let right = key(KeyCode::Right, NONE);
        assert_eq!(
            run_keymap(&[tab.clone(), left.clone(), left.clone()]),
            [
                Some(Action::ToggleSidebar),
                Some(Action::DockSidebar(Dock::Left)),
                Some(Action::ScrollLeft),
            ],
            "only the key right after Tab docks"
        );
        assert_eq!(
            run_keymap(&[tab.clone(), right.clone()]),
            [
                Some(Action::ToggleSidebar),
                Some(Action::DockSidebar(Dock::Right)),
            ]
        );
    }

    #[test]
    fn arrows_scroll_unless_right_after_tab() {
        let tab = key(KeyCode::Tab, NONE);
        let right = key(KeyCode::Right, NONE);
        assert_eq!(
            run_keymap(std::slice::from_ref(&right)),
            [Some(Action::ScrollRight)]
        );
        assert_eq!(
            run_keymap(&[tab.clone(), key(KeyCode::Char('j'), NONE), right.clone()]),
            [
                Some(Action::ToggleSidebar),
                Some(Action::Down(1)),
                Some(Action::ScrollRight),
            ]
        );
        assert_eq!(
            run_keymap(&[tab, key(KeyCode::Char('l'), NONE)]),
            [Some(Action::ToggleSidebar), Some(Action::ScrollRight)],
            "only arrow keys dock, not h/l"
        );
    }

    #[test]
    fn tab_release_and_mouse_moves_do_not_break_the_sequence() {
        let release = Event::Key(KeyEvent {
            code: KeyCode::Tab,
            modifiers: NONE,
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
                key(KeyCode::Tab, NONE),
                release,
                moved,
                key(KeyCode::Left, NONE)
            ]),
            [
                Some(Action::ToggleSidebar),
                None,
                None,
                Some(Action::DockSidebar(Dock::Left)),
            ]
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
