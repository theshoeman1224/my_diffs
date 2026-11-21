//! Hex view for binary comparisons: equal rows once, changed rows as a `-` / `+`
//! pair with differing bytes highlighted.

use mydiff_core::RowKind;
use mydiff_core::binary::{HexDiff, ROW_BYTES, ascii};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, CHROME_ROWS, JUMP_CONTEXT, LineSide, Sidebar};
use crate::keys::ScreenKind;
use crate::view;

/// Inputs for an interactive binary comparison.
#[derive(Debug, Clone)]
pub struct HexSession {
    pub left_label: String,
    pub right_label: String,
    pub diff: HexDiff,
}

#[derive(Debug)]
pub struct HexApp {
    pub session: HexSession,
    /// (row index, side) per screen line.
    pub lines: Vec<(usize, LineSide)>,
    pub change_lines: Vec<usize>,
    pub current_change: Option<usize>,
    pub top: usize,
    pub viewport: usize,
    pub width: u16,
    pub show_help: bool,
    pub sidebar: Sidebar,
    pub should_quit: bool,
}

/// Columns of one hex row: marker, offset, 16 bytes, ASCII column.
pub const ROW_COLS: u16 = 79;

impl HexApp {
    pub fn new(session: HexSession) -> Self {
        let mut lines = Vec::new();
        let mut change_lines = Vec::new();
        let changed: std::collections::HashSet<usize> =
            session.diff.changed_rows.iter().copied().collect();
        for r in 0..session.diff.row_count() {
            if changed.contains(&r) {
                change_lines.push(lines.len());
                lines.push((r, LineSide::Left));
                lines.push((r, LineSide::Right));
            } else {
                lines.push((r, LineSide::Both));
            }
        }
        Self {
            session,
            lines,
            change_lines,
            current_change: None,
            top: 0,
            viewport: 20,
            width: 80,
            show_help: false,
            sidebar: Sidebar::default(),
            should_quit: false,
        }
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport)
    }

    fn scroll_to(&mut self, top: usize) {
        self.top = top.min(self.max_top());
        self.current_change = None;
    }

    pub fn update(&mut self, action: Action) {
        let page = self.viewport.saturating_sub(1).max(1);
        match action {
            Action::Quit | Action::ForceQuit => self.should_quit = true,
            Action::Back => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::ToggleSidebar | Action::DockSidebar(_) => self.sidebar.update(action),
            Action::Up(n) => self.scroll_to(self.top.saturating_sub(n)),
            Action::Down(n) => self.scroll_to(self.top + n),
            Action::PageUp => self.scroll_to(self.top.saturating_sub(page)),
            Action::PageDown => self.scroll_to(self.top + page),
            Action::Top => self.scroll_to(0),
            Action::Bottom => self.scroll_to(usize::MAX),
            Action::NextChange | Action::PrevChange => {
                let n = self.change_lines.len();
                let forward = action == Action::NextChange;
                let target = match (self.current_change, forward) {
                    (Some(i), true) => (i + 1 < n).then_some(i + 1),
                    (Some(i), false) => i.checked_sub(1),
                    (None, true) => self.change_lines.iter().position(|&l| l >= self.top),
                    (None, false) => self.change_lines.iter().rposition(|&l| l < self.top),
                };
                if let Some(i) = target {
                    self.top = self.change_lines[i]
                        .saturating_sub(JUMP_CONTEXT)
                        .min(self.max_top());
                    self.current_change = Some(i);
                }
            }
            Action::Resize { width, height } => {
                self.width = width;
                self.viewport = usize::from(height.saturating_sub(CHROME_ROWS)).max(1);
                self.top = self.top.min(self.max_top());
            }
            // Text-only toggles do not apply to bytes.
            _ => {}
        }
    }
}

pub fn render(app: &HexApp, frame: &mut Frame) {
    let (main, sidebar) = view::split_sidebar(frame.area(), app.sidebar, ROW_COLS);
    let [header, body, status] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(main);
    let w = usize::from(header.width);
    let s = &app.session;
    let title = view::pair_title("[binary] ", &s.left_label, &s.right_label, w);
    frame.render_widget(
        Paragraph::new(view::pad(&title, w)).style(view::bar()),
        header,
    );

    let diff = &s.diff;
    let lines: Vec<Line> = app
        .lines
        .iter()
        .skip(app.top)
        .take(usize::from(body.height))
        .map(|&(r, side)| {
            let row = diff.row(r);
            let (bytes, kind, marker) = match side {
                LineSide::Both => (row.left, RowKind::Equal, ' '),
                LineSide::Left => (row.left, RowKind::Removed, '-'),
                LineSide::Right => (row.right, RowKind::Added, '+'),
            };
            let base = view::row_style(kind);
            let mut spans = vec![
                Span::styled(format!("{marker} "), base),
                Span::styled(format!("{:08x}  ", row.offset), view::dim()),
            ];
            let mut text = Vec::new();
            for k in 0..ROW_BYTES {
                if k == ROW_BYTES / 2 {
                    spans.push(Span::raw(" "));
                }
                let style = if row.diff[k] { view::hot_style() } else { base };
                match bytes.get(k) {
                    Some(b) => {
                        spans.push(Span::styled(format!("{b:02x}"), style));
                        spans.push(Span::raw(" "));
                        text.push(Span::styled(ascii(*b).to_string(), style));
                    }
                    None => spans.push(Span::raw("   ")),
                }
            }
            spans.push(Span::raw("|"));
            spans.extend(text);
            spans.push(Span::raw("|"));
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), body);

    let st = diff.stats();
    let left = format!(" sizes {} vs {}", st.left_len, st.right_len);
    let position = match (app.current_change, app.change_lines.len()) {
        (_, 0) => "identical".to_owned(),
        (Some(i), n) => format!("row {}/{n}", i + 1),
        (None, n) => format!("{n} changed row{}", if n == 1 { "" } else { "s" }),
    };
    let right = format!(
        "{} bytes differ  {position}  tab:keys ?:help q:quit ",
        st.differing_bytes
    );
    view::render_status_line(frame, status, &left, &right);
    if let Some(area) = sidebar {
        view::render_sidebar(frame, area, app.sidebar.dock, ScreenKind::Hex, &|_| None);
    }
    if app.show_help {
        view::render_help(frame, ScreenKind::Hex);
    }
}

#[cfg(test)]
mod tests {
    use mydiff_core::binary::diff_bytes;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    fn app() -> HexApp {
        let a: Vec<u8> = (0u8..64).collect();
        let mut b = a.clone();
        b[5] = 0xff;
        b[60] = b'A';
        let mut app = HexApp::new(HexSession {
            left_label: "a.bin".into(),
            right_label: "b.bin".into(),
            diff: diff_bytes(a, b),
        });
        app.update(Action::Resize {
            width: 80,
            height: 9,
        });
        app
    }

    #[test]
    fn changed_rows_take_two_lines() {
        let a = app();
        assert_eq!(a.lines.len(), 6);
        assert_eq!(a.change_lines, [0, 4]);
    }

    #[test]
    fn jumps_between_changed_rows() {
        let mut a = app();
        a.update(Action::Resize {
            width: 80,
            height: 5,
        }); // 3 body rows, 6 lines
        a.update(Action::NextChange);
        a.update(Action::NextChange);
        assert_eq!((a.current_change, a.top), (Some(1), 1));
        a.update(Action::PrevChange);
        assert_eq!((a.current_change, a.top), (Some(0), 0));
    }

    fn screen_at(a: &HexApp, w: u16, h: u16) -> String {
        let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
        t.draw(|f| render(a, f)).unwrap();
        t.backend().to_string()
    }

    #[test]
    fn screen() {
        insta::assert_snapshot!(screen_at(&app(), 80, 9));
    }

    #[test]
    fn sidebar_needs_room_for_a_full_hex_row() {
        let a = app();
        let w = ROW_COLS + view::SIDEBAR_W;
        assert!(!screen_at(&a, w - 1, 9).contains("commands"));
        assert!(screen_at(&a, w, 9).contains("commands"));
    }

    #[test]
    fn screen_with_sidebar() {
        let mut a = app();
        a.update(Action::Resize {
            width: 113,
            height: 22,
        });
        insta::assert_snapshot!(screen_at(&a, 113, 22));
    }

    #[test]
    fn sidebar_actions() {
        let mut a = app();
        a.update(Action::ToggleSidebar);
        assert!(!a.sidebar.open);
        a.update(Action::DockSidebar(crate::app::Dock::Left));
        assert!(a.sidebar.open);
        a.update(Action::Back);
        assert!(a.should_quit);
    }
}
