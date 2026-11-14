//! Terminal events → [`Action`]s. Pure, so every binding is unit-tested.

use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};

use crate::app::Action;

/// Lines moved per mouse-wheel notch.
pub const WHEEL_LINES: usize = 3;

/// Key bindings shown in the help overlay: (keys, description).
pub const HELP: &[(&str, &str)] = &[
    ("j / ↓ / wheel", "scroll down"),
    ("k / ↑ / wheel", "scroll up"),
    ("PgDn / Space / Ctrl-d", "page down"),
    ("PgUp / Ctrl-u", "page up"),
    ("g / Home", "top"),
    ("G / End", "bottom"),
    ("n", "next change"),
    ("p / N", "previous change"),
    ("h / ←   l / →", "scroll left / right"),
    ("w", "cycle whitespace (exact/trailing/change/all)"),
    ("i", "toggle ignore case"),
    ("b", "toggle ignore blank lines"),
    ("e", "toggle ignore line endings (CRLF/LF)"),
    ("u", "toggle side-by-side / inline layout"),
    ("Enter", "folder view: open file diff (Esc/q returns)"),
    ("s", "folder view: show/hide identical files"),
    ("?", "toggle this help"),
    ("q / Esc", "quit or go back (Esc closes help first)"),
    ("Ctrl-c", "quit from anywhere"),
];

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
    #[case::unbound(KeyCode::Char('z'), NONE, None)]
    #[case::unbound_ctrl(KeyCode::Char('w'), CTRL, None)]
    fn keys(#[case] code: KeyCode, #[case] mods: KeyModifiers, #[case] expected: Option<Action>) {
        assert_eq!(action_for(&key(code, mods)), expected);
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
