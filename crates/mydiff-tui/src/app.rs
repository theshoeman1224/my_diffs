//! Application state and the pure `update` function.

use mydiff_core::{DiffOptions, Result, RowKind, TextDiff, diff_text};

/// Inputs for an interactive text comparison.
#[derive(Debug, Clone)]
pub struct TextSession {
    pub left_label: String,
    pub right_label: String,
    pub left: String,
    pub right: String,
    pub options: DiffOptions,
}

/// How rows are laid out on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Two panes.
    SideBySide,
    /// One column; modified rows take two lines (old, then new).
    Inline,
}

/// Which side of a row a display line shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineSide {
    Both,
    Left,
    Right,
}

/// One screen line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayLine {
    pub row: usize,
    pub side: LineSide,
}

/// Everything the user can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Quit,
    /// Quit from anywhere (Ctrl-C), even from a nested view.
    ForceQuit,
    /// Open the selected entry (folder view).
    Open,
    /// Show or hide identical entries (folder view).
    ToggleSame,
    /// Close the help overlay, or quit if it is not open.
    Back,
    Up(usize),
    Down(usize),
    PageUp,
    PageDown,
    Top,
    Bottom,
    NextChange,
    PrevChange,
    ScrollLeft,
    ScrollRight,
    CycleWhitespace,
    ToggleCase,
    ToggleBlankLines,
    ToggleLineEndings,
    ToggleLayout,
    ToggleHelp,
    /// Terminal size changed (full terminal size, in cells).
    Resize {
        width: u16,
        height: u16,
    },
}

/// Lines kept above a change when jumping to it.
pub const JUMP_CONTEXT: usize = 3;
/// Columns moved per horizontal scroll step.
pub const HSCROLL_STEP: usize = 8;
/// Rows used by the header and status bars.
pub const CHROME_ROWS: u16 = 2;

#[derive(Debug)]
pub struct App {
    pub session: TextSession,
    pub diff: TextDiff,
    pub layout: Layout,
    pub lines: Vec<DisplayLine>,
    /// Display-line index of the first change of each block.
    pub change_lines: Vec<usize>,
    /// Index into `change_lines` after a next/prev jump; cleared by manual scrolling.
    pub current_change: Option<usize>,
    pub top: usize,
    pub hscroll: usize,
    /// Body height in rows.
    pub viewport: usize,
    pub width: u16,
    pub show_help: bool,
    pub should_quit: bool,
}

impl App {
    pub fn new(session: TextSession) -> Result<Self> {
        let diff = diff_text(&session.left, &session.right, &session.options)?;
        let mut app = Self {
            session,
            diff,
            layout: Layout::SideBySide,
            lines: Vec::new(),
            change_lines: Vec::new(),
            current_change: None,
            top: 0,
            hscroll: 0,
            viewport: 20,
            width: 80,
            show_help: false,
            should_quit: false,
        };
        app.rebuild_lines();
        Ok(app)
    }

    pub fn update(&mut self, action: Action) {
        match action {
            Action::Quit | Action::ForceQuit => self.should_quit = true,
            Action::Open | Action::ToggleSame => {}
            Action::Back => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::Up(n) => self.scroll_to(self.top.saturating_sub(n)),
            Action::Down(n) => self.scroll_to(self.top + n),
            Action::PageUp => self.scroll_to(self.top.saturating_sub(self.page())),
            Action::PageDown => self.scroll_to(self.top + self.page()),
            Action::Top => self.scroll_to(0),
            Action::Bottom => self.scroll_to(usize::MAX),
            Action::NextChange => self.jump(true),
            Action::PrevChange => self.jump(false),
            Action::ScrollLeft => self.hscroll = self.hscroll.saturating_sub(HSCROLL_STEP),
            Action::ScrollRight => self.hscroll += HSCROLL_STEP,
            Action::CycleWhitespace => {
                self.session.options.whitespace = self.session.options.whitespace.cycle();
                self.recompute();
            }
            Action::ToggleCase => {
                self.session.options.ignore_case ^= true;
                self.recompute();
            }
            Action::ToggleBlankLines => {
                self.session.options.ignore_blank_lines ^= true;
                self.recompute();
            }
            Action::ToggleLineEndings => {
                self.session.options.ignore_line_endings ^= true;
                self.recompute();
            }
            Action::ToggleLayout => {
                let anchor = self.anchor();
                self.layout = match self.layout {
                    Layout::SideBySide => Layout::Inline,
                    Layout::Inline => Layout::SideBySide,
                };
                self.rebuild_lines();
                self.restore_anchor(anchor);
            }
            Action::Resize { width, height } => {
                self.width = width;
                self.viewport = usize::from(height.saturating_sub(CHROME_ROWS)).max(1);
                self.top = self.top.min(self.max_top());
            }
        }
    }

    fn page(&self) -> usize {
        self.viewport.saturating_sub(1).max(1)
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport)
    }

    fn scroll_to(&mut self, top: usize) {
        self.top = top.min(self.max_top());
        self.current_change = None;
    }

    fn jump(&mut self, forward: bool) {
        let n = self.change_lines.len();
        if n == 0 {
            return;
        }
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

    /// Re-run the comparison after an option toggle, keeping the view near the
    /// same source line.
    fn recompute(&mut self) {
        let anchor = self.anchor();
        let s = &self.session;
        // Options only change through toggles of already-validated settings.
        self.diff = diff_text(&s.left, &s.right, &s.options).expect("toggled options stay valid");
        self.rebuild_lines();
        self.restore_anchor(anchor);
    }

    /// Left (or right) line number shown at the top of the screen.
    fn anchor(&self) -> Option<(bool, usize)> {
        let row = &self.diff.rows[self.lines.get(self.top)?.row];
        row.left
            .as_ref()
            .map(|l| (true, l.number))
            .or_else(|| row.right.as_ref().map(|r| (false, r.number)))
    }

    fn restore_anchor(&mut self, anchor: Option<(bool, usize)>) {
        self.current_change = None;
        let Some((left, number)) = anchor else {
            self.top = 0;
            return;
        };
        let pos = self.lines.iter().position(|dl| {
            let row = &self.diff.rows[dl.row];
            let side = if left { &row.left } else { &row.right };
            side.as_ref().is_some_and(|l| l.number >= number)
        });
        self.top = pos.unwrap_or(0).min(self.max_top());
    }

    fn rebuild_lines(&mut self) {
        self.lines.clear();
        for (i, row) in self.diff.rows.iter().enumerate() {
            let one = |side| DisplayLine { row: i, side };
            match self.layout {
                Layout::SideBySide => self.lines.push(one(LineSide::Both)),
                Layout::Inline => match (row.kind, &row.left, &row.right) {
                    (RowKind::Equal, ..) => self.lines.push(one(LineSide::Both)),
                    (_, Some(l), Some(r)) if l.text == r.text => {
                        self.lines.push(one(LineSide::Both));
                    }
                    (_, l, r) => {
                        if l.is_some() {
                            self.lines.push(one(LineSide::Left));
                        }
                        if r.is_some() {
                            self.lines.push(one(LineSide::Right));
                        }
                    }
                },
            }
        }
        let starts = self.diff.change_starts();
        self.change_lines = starts
            .iter()
            .filter_map(|&row| self.lines.iter().position(|dl| dl.row == row))
            .collect();
        self.top = self.top.min(self.max_top());
    }
}

#[cfg(test)]
mod tests {
    use mydiff_core::WhitespaceMode;

    use super::*;

    fn session(left: &str, right: &str) -> TextSession {
        TextSession {
            left_label: "l".into(),
            right_label: "r".into(),
            left: left.into(),
            right: right.into(),
            options: DiffOptions::default(),
        }
    }

    /// 100 lines with changes at lines 10, 50 and 90.
    fn long_app(viewport: u16) -> App {
        let left: String = (1..=100).map(|i| format!("line {i}\n")).collect();
        let right = left
            .replace("line 10\n", "line ten\n")
            .replace("line 50\n", "line fifty\n")
            .replace("line 90\n", "line ninety\n");
        let mut app = App::new(session(&left, &right)).unwrap();
        app.update(Action::Resize {
            width: 80,
            height: viewport + CHROME_ROWS,
        });
        app
    }

    #[test]
    fn scrolling_is_clamped() {
        let mut app = long_app(10);
        app.update(Action::Up(5));
        assert_eq!(app.top, 0);
        app.update(Action::Bottom);
        assert_eq!(app.top, 90);
        app.update(Action::Down(1));
        assert_eq!(app.top, 90);
        app.update(Action::PageUp);
        assert_eq!(app.top, 81);
    }

    #[test]
    fn next_and_prev_change_walk_blocks() {
        let mut app = long_app(10);
        assert_eq!(app.change_lines, [9, 49, 89]);
        app.update(Action::NextChange);
        assert_eq!((app.top, app.current_change), (6, Some(0)));
        app.update(Action::NextChange);
        assert_eq!((app.top, app.current_change), (46, Some(1)));
        app.update(Action::NextChange);
        assert_eq!((app.top, app.current_change), (86, Some(2)));
        app.update(Action::NextChange);
        assert_eq!(app.current_change, Some(2), "stays on last change");
        app.update(Action::PrevChange);
        assert_eq!((app.top, app.current_change), (46, Some(1)));
    }

    #[test]
    fn manual_scroll_resets_change_cursor_and_next_searches_from_view() {
        let mut app = long_app(10);
        app.update(Action::Down(60));
        assert_eq!(app.current_change, None);
        app.update(Action::NextChange);
        assert_eq!(app.current_change, Some(2));
        app.update(Action::Top);
        app.update(Action::Down(60));
        app.update(Action::PrevChange);
        assert_eq!(app.current_change, Some(1));
    }

    #[test]
    fn whitespace_toggle_recomputes_diff() {
        let mut app = App::new(session("a  b\n", "a b\n")).unwrap();
        assert!(!app.diff.is_identical());
        app.update(Action::CycleWhitespace); // trailing
        assert!(!app.diff.is_identical());
        app.update(Action::CycleWhitespace); // change
        assert_eq!(app.session.options.whitespace, WhitespaceMode::Change);
        assert!(app.diff.is_identical());
    }

    #[test]
    fn case_and_blank_toggles() {
        let mut app = App::new(session("A\n\n", "a\n")).unwrap();
        app.update(Action::ToggleCase);
        assert!(!app.diff.is_identical(), "blank line still differs");
        app.update(Action::ToggleBlankLines);
        assert!(app.diff.is_identical());
        app.update(Action::ToggleCase);
        assert!(!app.diff.is_identical());
    }

    #[test]
    fn line_endings_toggle() {
        let mut app = App::new(session("a\r\n", "a\n")).unwrap();
        assert!(app.diff.is_identical());
        app.update(Action::ToggleLineEndings);
        assert!(!app.diff.is_identical());
    }

    #[test]
    fn inline_layout_splits_modified_rows() {
        let mut app = App::new(session("same\nold line\n", "same\nnew line\n")).unwrap();
        assert_eq!(app.lines.len(), 2);
        app.update(Action::ToggleLayout);
        assert_eq!(
            app.lines.iter().map(|l| l.side).collect::<Vec<_>>(),
            [LineSide::Both, LineSide::Left, LineSide::Right]
        );
    }

    #[test]
    fn toggles_keep_position_near_same_source_line() {
        let mut app = long_app(10);
        app.update(Action::Down(40));
        app.update(Action::ToggleLayout);
        let top_row = &app.diff.rows[app.lines[app.top].row];
        assert_eq!(top_row.left.as_ref().unwrap().number, 41);
    }

    #[test]
    fn back_closes_help_before_quitting() {
        let mut app = long_app(10);
        app.update(Action::ToggleHelp);
        app.update(Action::Back);
        assert!(!app.show_help && !app.should_quit);
        app.update(Action::Back);
        assert!(app.should_quit);
    }

    #[test]
    fn no_changes_means_jumps_do_nothing() {
        let mut app = App::new(session("a\n", "a\n")).unwrap();
        app.update(Action::NextChange);
        assert_eq!((app.top, app.current_change), (0, None));
    }

    #[test]
    fn horizontal_scroll_never_negative() {
        let mut app = long_app(10);
        app.update(Action::ScrollLeft);
        assert_eq!(app.hscroll, 0);
        app.update(Action::ScrollRight);
        assert_eq!(app.hscroll, HSCROLL_STEP);
    }
}
