//! Application state and the pure `update` function.

use mydiff_core::display::{sanitize, width};
use mydiff_core::{
    DiffOptions, LineRef, Result, Row, RowKind, TextDiff, WhitespaceMode, diff_text,
};
use regex::Regex;

use crate::cmdline::{
    CmdLine, Command, Message, PromptEvent, PromptKey, PromptKind, parse_command,
};
use crate::keys::InputMode;
use crate::search::{self, Cursor, Direction, Match};

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

/// One side of the comparison. The active pane is where searches look,
/// like the current window in vimdiff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Pane {
    #[default]
    Left,
    Right,
}

impl Pane {
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    /// This side's line of `row`, if it has one.
    pub fn line(self, row: &Row) -> Option<&LineRef> {
        match self {
            Self::Left => row.left.as_ref(),
            Self::Right => row.right.as_ref(),
        }
    }
}

/// Which edge of the screen the command sidebar sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dock {
    Left,
    Right,
}

/// The command sidebar: open on the right by default. Every screen owns one;
/// a folder view hands its sidebar to an opened file and takes it back after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sidebar {
    /// What the user asked for. A narrow terminal may still hide it.
    pub open: bool,
    pub dock: Dock,
}

impl Default for Sidebar {
    fn default() -> Self {
        Self {
            open: true,
            dock: Dock::Right,
        }
    }
}

impl Sidebar {
    /// `:sidebar` (toggle) or `:sidebar left|right` (open on that edge).
    pub fn command(&mut self, dock: Option<Dock>) {
        self.update(dock.map_or(Action::ToggleSidebar, Action::DockSidebar));
    }

    /// Apply a sidebar action; other actions are ignored.
    pub fn update(&mut self, action: Action) {
        match action {
            Action::ToggleSidebar => self.open ^= true,
            Action::DockSidebar(dock) => {
                self.open = true;
                self.dock = dock;
            }
            _ => {}
        }
    }
}

/// Live value of a comparison toggle for the sidebar: (text, is it on).
pub fn option_value(options: &DiffOptions, action: Action) -> Option<(&'static str, bool)> {
    let on_off = |b: bool| (if b { "on" } else { "off" }, b);
    Some(match action {
        Action::CycleWhitespace => (
            options.whitespace.label(),
            options.whitespace != WhitespaceMode::Exact,
        ),
        Action::ToggleCase => on_off(options.ignore_case),
        Action::ToggleBlankLines => on_off(options.ignore_blank_lines),
        Action::ToggleLineEndings => on_off(options.ignore_line_endings),
        _ => return None,
    })
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
    /// Show or hide the command sidebar (F1, `:sidebar`).
    ToggleSidebar,
    /// Open the command sidebar on one edge (`:sidebar left|right`).
    DockSidebar(Dock),
    /// Open the search line (`/` forward, `?` backward).
    StartSearch(Direction),
    /// Open the `:` command line.
    StartCommand,
    /// An editing key while the search or command line is open.
    Prompt(PromptKey),
    /// `n`: repeat the last search in its direction.
    SearchNext,
    /// `N`: repeat the last search the opposite way.
    SearchPrev,
    /// `Tab` or `Ctrl-w w`: make the other pane active.
    SwitchPane,
    /// `Ctrl-w h` / `Ctrl-w l`: make the left / right pane active.
    FocusPane(Pane),
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
    /// Display width of the widest line on either side, tabs expanded.
    /// Bounds `hscroll`: scrolling right stops once that line's end is visible.
    pub widest: usize,
    /// Body height in rows.
    pub viewport: usize,
    pub width: u16,
    pub show_help: bool,
    pub sidebar: Sidebar,
    /// Pane that searches look in, marked `▶` in the header.
    pub focus: Pane,
    /// The `/`, `?` or `:` line on the bottom row.
    pub cmdline: CmdLine,
    pub search: SearchState,
    /// Shown in the status bar until the next key.
    pub message: Option<Message>,
    pub should_quit: bool,
    /// `:qa`: quit the whole program, not just this view.
    pub quit_all: bool,
}

/// The last search and where its current match is.
#[derive(Debug, Default)]
pub struct SearchState {
    pub last: Option<LastSearch>,
    /// Highlight every match of `last` (off after `:noh`, on again after `n`, `N` or a new search).
    pub highlight: bool,
    /// The match the view last jumped to; `n` and `N` continue from here.
    pub current: Option<Match>,
    /// While the search line is open: what was typed and where the search started.
    pub typing: Option<Typing>,
}

#[derive(Debug)]
pub struct LastSearch {
    pub pattern: String,
    pub regex: Regex,
    pub direction: Direction,
}

/// Incremental search state: the view jumps to matches as the pattern is typed
/// and goes back to `origin_*` when the search is cancelled.
#[derive(Debug)]
pub struct Typing {
    /// The typed pattern, when it compiles.
    pub regex: Option<Regex>,
    origin_top: usize,
    origin_hscroll: usize,
    origin_current: Option<Match>,
}

impl App {
    pub fn new(session: TextSession) -> Result<Self> {
        let diff = diff_text(&session.left, &session.right, &session.options)?;
        let widest = diff
            .rows
            .iter()
            .flat_map(|r| [r.left.as_ref(), r.right.as_ref()])
            .flatten()
            .map(|l| width(&sanitize(&l.text, 0)))
            .max()
            .unwrap_or(0);
        let mut app = Self {
            session,
            diff,
            layout: Layout::SideBySide,
            lines: Vec::new(),
            change_lines: Vec::new(),
            current_change: None,
            top: 0,
            hscroll: 0,
            widest,
            viewport: 20,
            width: 80,
            show_help: false,
            sidebar: Sidebar::default(),
            focus: Pane::Left,
            cmdline: CmdLine::default(),
            search: SearchState::default(),
            message: None,
            should_quit: false,
            quit_all: false,
        };
        app.rebuild_lines();
        Ok(app)
    }

    /// Whether keys should edit the command line.
    pub fn input_mode(&self) -> InputMode {
        if self.cmdline.is_open() {
            InputMode::Prompt
        } else {
            InputMode::Normal
        }
    }

    pub fn update(&mut self, action: Action) {
        self.apply(action);
        // Layout, sidebar and terminal width all change how much text fits.
        self.hscroll = self.hscroll.min(self.max_hscroll());
    }

    /// Furthest right the view can scroll: the widest line's end at the right edge.
    pub fn max_hscroll(&self) -> usize {
        self.widest.saturating_sub(crate::view::text_width(self))
    }

    fn apply(&mut self, action: Action) {
        if !matches!(action, Action::Resize { .. }) {
            self.message = None;
        }
        match action {
            Action::Quit => self.should_quit = true,
            Action::ForceQuit => {
                self.should_quit = true;
                self.quit_all = true;
            }
            Action::Open | Action::ToggleSame => {}
            Action::Back => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            Action::ToggleSidebar | Action::DockSidebar(_) => self.sidebar.update(action),
            Action::Up(n) => self.scroll_to(self.top.saturating_sub(n)),
            Action::Down(n) => self.scroll_to(self.top + n),
            Action::PageUp => self.scroll_to(self.top.saturating_sub(self.page())),
            Action::PageDown => self.scroll_to(self.top + self.page()),
            Action::Top => self.scroll_to(0),
            Action::Bottom => self.scroll_to(usize::MAX),
            Action::NextChange => self.jump(true),
            Action::PrevChange => self.jump(false),
            Action::StartSearch(dir) => {
                self.show_help = false;
                self.cmdline.open(PromptKind::Search(dir));
                self.search.typing = Some(Typing {
                    regex: None,
                    origin_top: self.top,
                    origin_hscroll: self.hscroll,
                    origin_current: self.search.current,
                });
            }
            Action::StartCommand => {
                self.show_help = false;
                self.cmdline.open(PromptKind::Command);
            }
            Action::Prompt(key) => self.prompt_key(key),
            Action::SearchNext => self.search_again(false),
            Action::SearchPrev => self.search_again(true),
            Action::SwitchPane => self.set_focus(self.focus.other()),
            Action::FocusPane(pane) => self.set_focus(pane),
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

    /// Live value shown next to a command in the sidebar.
    pub fn command_value(&self, action: Action) -> Option<(&'static str, bool)> {
        match action {
            Action::SwitchPane => Some((self.focus.name(), false)),
            Action::ToggleLayout => Some(match self.layout {
                Layout::SideBySide => ("side-by-side", false),
                Layout::Inline => ("inline", true),
            }),
            _ => option_value(&self.session.options, action),
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
        self.forget_offscreen_match();
    }

    /// `n` continues from the current match only while it is on screen;
    /// otherwise from the top of the screen.
    fn forget_offscreen_match(&mut self) {
        if let Some(m) = self.search.current {
            let visible = self.top..self.top + self.viewport;
            if !self
                .display_line(m.row)
                .is_some_and(|l| visible.contains(&l))
            {
                self.search.current = None;
            }
        }
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
            self.forget_offscreen_match();
        }
    }

    /// Re-run the comparison after an option toggle, keeping the view near the
    /// same source line.
    fn recompute(&mut self) {
        let anchor = self.anchor();
        let s = &self.session;
        // Options only change through toggles of already-validated settings.
        self.diff = diff_text(&s.left, &s.right, &s.options).expect("toggled options stay valid");
        // Row indices change with the diff.
        self.search.current = None;
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

    fn set_focus(&mut self, pane: Pane) {
        if pane != self.focus {
            self.focus = pane;
            // The current match was in the other pane's text.
            self.search.current = None;
        }
    }

    /// Display line showing `row`'s text from the active pane.
    pub fn display_line(&self, row: usize) -> Option<usize> {
        let first = self.lines.partition_point(|dl| dl.row < row);
        (first..self.lines.len())
            .take_while(|&i| self.lines[i].row == row)
            .find(|&i| self.shows_focus(self.lines[i]))
    }

    /// Whether `dl` shows the active pane's text (the only text searches see).
    pub fn shows_focus(&self, dl: DisplayLine) -> bool {
        match dl.side {
            LineSide::Both => true,
            LineSide::Left => self.focus == Pane::Left,
            LineSide::Right => self.focus == Pane::Right,
        }
    }

    /// The pattern to highlight: the one being typed, else the last search
    /// unless `:noh` hid it.
    pub fn highlight_regex(&self) -> Option<&Regex> {
        match &self.search.typing {
            Some(t) => t.regex.as_ref(),
            None if self.search.highlight => self.search.last.as_ref().map(|l| &l.regex),
            None => None,
        }
    }

    /// Every match of `re` in `pane`'s text, in file order.
    fn matches_in(&self, pane: Pane, re: &Regex) -> Vec<Match> {
        let texts = self
            .diff
            .rows
            .iter()
            .enumerate()
            .filter_map(|(i, row)| pane.line(row).map(|l| (i, l.text.as_str())));
        search::collect(texts, re)
    }

    /// Where `n` continues from: the current match, else the top screen line.
    fn cursor(&self) -> Cursor {
        match self.search.current {
            Some(m) => Cursor {
                row: m.row,
                byte: m.start,
                on_match: true,
            },
            None => Cursor {
                row: self.lines.get(self.top).map_or(0, |dl| dl.row),
                byte: 0,
                on_match: false,
            },
        }
    }

    /// Make `m` the current match and scroll so it is on screen: vertically
    /// centred if it was off screen, horizontally only if it was cut off.
    fn go_to(&mut self, m: Match) {
        self.search.current = Some(m);
        let Some(line) = self.display_line(m.row) else {
            return;
        };
        if !(self.top..self.top + self.viewport).contains(&line) {
            self.top = line.saturating_sub(self.viewport / 2).min(self.max_top());
            self.current_change = None;
        }
        let text = self
            .focus
            .line(&self.diff.rows[m.row])
            .map_or("", |l| l.text.as_str());
        let start = width(&sanitize(&text[..m.start], 0));
        let end = start + width(&sanitize(&text[m.start..m.end], start));
        let cols = crate::view::text_width(self);
        if start < self.hscroll || end > self.hscroll + cols {
            self.hscroll = if end <= cols {
                0
            } else {
                start.saturating_sub(cols / 2)
            };
        }
    }

    /// Search `dir` for `re` from the cursor, report the result in the status bar.
    fn run_search(&mut self, re: &Regex, pattern: &str, dir: Direction) {
        let all = self.matches_in(self.focus, re);
        let Some((i, wrapped)) = search::find(&all, self.cursor(), dir) else {
            // Vim's wording; the hint points at the other pane when it has a match.
            let mut text = format!("E486: Pattern not found: {pattern}");
            if !self.matches_in(self.focus.other(), re).is_empty() {
                text.push_str(&format!(" (in {} pane: Tab)", self.focus.other().name()));
            }
            self.message = Some(Message::error(text));
            return;
        };
        self.go_to(all[i]);
        let what = match (wrapped, dir) {
            (false, _) => format!("{}{pattern}", dir.leader()),
            (true, Direction::Forward) => "search hit BOTTOM, continuing at TOP".to_owned(),
            (true, Direction::Backward) => "search hit TOP, continuing at BOTTOM".to_owned(),
        };
        self.message = Some(Message::info(format!("{what}  [{}/{}]", i + 1, all.len())));
    }

    /// `n` (`reverse` false) or `N`.
    fn search_again(&mut self, reverse: bool) {
        let Some(last) = &self.search.last else {
            self.message = Some(Message::error("E35: No previous regular expression"));
            return;
        };
        let (re, pattern) = (last.regex.clone(), last.pattern.clone());
        let dir = if reverse {
            last.direction.reverse()
        } else {
            last.direction
        };
        self.search.highlight = true;
        self.run_search(&re, &pattern, dir);
    }

    fn prompt_key(&mut self, key: PromptKey) {
        let Some(kind) = self.cmdline.prompt.as_ref().map(|p| p.kind) else {
            return;
        };
        match (self.cmdline.key(key), kind) {
            (PromptEvent::Edited, PromptKind::Search(dir)) => self.incremental(dir),
            (PromptEvent::Edited, PromptKind::Command) => {}
            (PromptEvent::Cancelled, _) => self.restore_origin(),
            (PromptEvent::Submitted(_, text), PromptKind::Search(dir)) => {
                self.submit_search(dir, text);
            }
            (PromptEvent::Submitted(_, text), PromptKind::Command) => self.run_command(&text),
        }
    }

    /// Go back to where the view was when the search line opened.
    fn restore_origin(&mut self) {
        if let Some(t) = self.search.typing.take() {
            self.top = t.origin_top;
            self.hscroll = t.origin_hscroll;
            self.search.current = t.origin_current;
        }
    }

    /// Jump to the first match of what is typed so far, searching from where
    /// the search started.
    fn incremental(&mut self, dir: Direction) {
        let Some(t) = &self.search.typing else {
            return;
        };
        let (top, hscroll, current) = (t.origin_top, t.origin_hscroll, t.origin_current);
        self.top = top;
        self.hscroll = hscroll;
        self.search.current = current;
        let input = self.cmdline.input();
        let regex = (!input.is_empty())
            .then(|| search::compile(input).ok())
            .flatten();
        if let Some(re) = &regex {
            let all = self.matches_in(self.focus, re);
            if let Some((i, _)) = search::find(&all, self.cursor(), dir) {
                self.go_to(all[i]);
            }
        }
        if let Some(t) = &mut self.search.typing {
            t.regex = regex;
        }
    }

    /// Enter on the search line. An empty pattern reuses the last one, in the new direction.
    fn submit_search(&mut self, dir: Direction, typed: String) {
        self.restore_origin();
        let pattern = if !typed.is_empty() {
            typed
        } else if let Some(last) = &self.search.last {
            last.pattern.clone()
        } else {
            self.message = Some(Message::error("E35: No previous regular expression"));
            return;
        };
        match search::compile(&pattern) {
            Ok(regex) => {
                self.search.last = Some(LastSearch {
                    pattern: pattern.clone(),
                    regex: regex.clone(),
                    direction: dir,
                });
                self.search.highlight = true;
                self.run_search(&regex, &pattern, dir);
            }
            Err(e) => self.message = Some(Message::error(e)),
        }
    }

    fn run_command(&mut self, line: &str) {
        match parse_command(line) {
            Ok(None) => {}
            Ok(Some(Command::Help)) => self.show_help = true,
            Ok(Some(Command::Sidebar(dock))) => self.sidebar.command(dock),
            Ok(Some(Command::Quit)) => self.should_quit = true,
            Ok(Some(Command::QuitAll)) => {
                self.should_quit = true;
                self.quit_all = true;
            }
            Ok(Some(Command::NoHighlight)) => self.search.highlight = false,
            Err(e) => self.message = Some(Message::error(e)),
        }
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
    use super::*;
    use crate::search::Direction;

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
        command(&mut app, "help");
        app.update(Action::Back);
        assert!(!app.show_help && !app.should_quit);
        app.update(Action::Back);
        assert!(app.should_quit);
    }

    #[test]
    fn tab_toggles_sidebar_and_back_leaves_it_open() {
        let mut app = long_app(10);
        assert!(app.sidebar.open, "open by default");
        app.update(Action::Back);
        assert!(
            app.sidebar.open && app.should_quit,
            "Esc quits, sidebar untouched"
        );
        let mut app = long_app(10);
        app.update(Action::ToggleSidebar);
        assert!(!app.sidebar.open);
        app.update(Action::ToggleSidebar);
        assert!(app.sidebar.open);
    }

    #[test]
    fn docking_opens_sidebar_on_that_side() {
        let mut app = long_app(10);
        app.update(Action::ToggleSidebar); // Tab closes it...
        app.update(Action::DockSidebar(Dock::Left)); // ...then ← reopens it on the left
        assert_eq!(
            app.sidebar,
            Sidebar {
                open: true,
                dock: Dock::Left
            }
        );
        app.update(Action::DockSidebar(Dock::Right));
        assert_eq!(app.sidebar.dock, Dock::Right);
    }

    #[test]
    fn sidebar_toggles_keep_scroll_position() {
        let mut app = long_app(10);
        app.update(Action::Down(40));
        app.update(Action::ToggleSidebar);
        app.update(Action::DockSidebar(Dock::Left));
        assert_eq!(app.top, 40);
    }

    #[test]
    fn command_values_follow_toggles() {
        let mut app = App::new(session("a\n", "b\n")).unwrap();
        assert_eq!(
            app.command_value(Action::CycleWhitespace),
            Some(("exact", false))
        );
        assert_eq!(app.command_value(Action::ToggleCase), Some(("off", false)));
        app.update(Action::CycleWhitespace);
        app.update(Action::ToggleCase);
        app.update(Action::ToggleLayout);
        assert_eq!(
            app.command_value(Action::CycleWhitespace),
            Some(("trailing", true))
        );
        assert_eq!(app.command_value(Action::ToggleCase), Some(("on", true)));
        assert_eq!(
            app.command_value(Action::ToggleLayout),
            Some(("inline", true))
        );
        assert_eq!(app.command_value(Action::NextChange), None);
    }

    #[test]
    fn no_changes_means_jumps_do_nothing() {
        let mut app = App::new(session("a\n", "a\n")).unwrap();
        app.update(Action::NextChange);
        assert_eq!((app.top, app.current_change), (0, None));
    }

    fn type_line(app: &mut App, open: Action, text: &str) {
        app.update(open);
        for c in text.chars() {
            app.update(Action::Prompt(PromptKey::Char(c)));
        }
        app.update(Action::Prompt(PromptKey::Submit));
    }

    fn search(app: &mut App, pattern: &str) {
        type_line(app, Action::StartSearch(Direction::Forward), pattern);
    }

    fn command(app: &mut App, line: &str) {
        type_line(app, Action::StartCommand, line);
    }

    fn message(app: &App) -> (&str, bool) {
        let m = app.message.as_ref().expect("a message");
        (m.text.as_str(), m.error)
    }

    /// Row and byte range of the current match.
    fn current(app: &App) -> Option<(usize, usize, usize)> {
        app.search.current.map(|m| (m.row, m.start, m.end))
    }

    #[test]
    fn slash_search_jumps_to_first_match_and_centres_it() {
        let mut app = long_app(10);
        search(&mut app, "line 7[0-9]");
        assert_eq!(current(&app), Some((69, 0, 7)), "line 70 is row 69");
        assert_eq!(app.top, 64, "centred: 69 - 10 / 2");
        assert_eq!(message(&app), ("/line 7[0-9]  [1/10]", false));
        assert!(app.search.highlight);
        assert_eq!(app.input_mode(), InputMode::Normal, "line closed");
    }

    #[test]
    fn match_already_on_screen_does_not_scroll() {
        let mut app = long_app(10);
        search(&mut app, "line 5$");
        assert_eq!((current(&app), app.top), (Some((4, 0, 6)), 0));
    }

    #[test]
    fn n_and_shift_n_walk_matches_and_wrap() {
        let mut app = long_app(10);
        search(&mut app, "line 9[0-9]");
        assert_eq!(
            current(&app).unwrap().0,
            89,
            "line 90 (changed rows still have a left side)"
        );
        app.update(Action::SearchNext);
        assert_eq!(current(&app).unwrap().0, 90);
        for _ in 0..9 {
            app.update(Action::SearchNext);
        }
        assert_eq!(current(&app).unwrap().0, 89, "wrapped to the first match");
        assert_eq!(
            message(&app),
            ("search hit BOTTOM, continuing at TOP  [1/10]", false)
        );
        app.update(Action::SearchPrev);
        assert_eq!(current(&app).unwrap().0, 98);
        assert_eq!(
            message(&app),
            ("search hit TOP, continuing at BOTTOM  [10/10]", false)
        );
    }

    #[test]
    fn question_mark_searches_backward_and_n_keeps_going_backward() {
        let mut app = long_app(10);
        app.update(Action::Bottom);
        type_line(&mut app, Action::StartSearch(Direction::Backward), "line 8");
        assert_eq!(
            current(&app).unwrap().0,
            88,
            "last match above the top line (90)"
        );
        assert_eq!(message(&app).0, "?line 8  [11/11]");
        app.update(Action::SearchNext);
        assert_eq!(current(&app).unwrap().0, 87, "n follows the ? direction");
        app.update(Action::SearchPrev);
        assert_eq!(current(&app).unwrap().0, 88, "N goes the other way");
    }

    #[test]
    fn search_only_looks_at_the_active_pane() {
        let mut app = long_app(10);
        search(&mut app, "fifty");
        assert_eq!(current(&app), None);
        assert_eq!(
            message(&app),
            ("E486: Pattern not found: fifty (in right pane: Tab)", true)
        );
        app.update(Action::SwitchPane);
        assert_eq!(app.focus, Pane::Right);
        app.update(Action::SearchNext);
        assert_eq!(current(&app), Some((49, 5, 10)));
        app.update(Action::FocusPane(Pane::Left));
        assert_eq!(current(&app), None, "the match belonged to the other pane");
        app.update(Action::SearchNext);
        assert!(message(&app).1, "not in the left pane");
    }

    #[test]
    fn active_pane_skips_lines_it_does_not_have() {
        let mut app = App::new(session("a\nold\nb\n", "a\nb\nold too\n")).unwrap();
        let text = |app: &App| {
            let row = &app.diff.rows[current(app).unwrap().0];
            app.focus.line(row).unwrap().text.clone()
        };
        search(&mut app, "old");
        assert_eq!(text(&app), "old");
        let first = current(&app);
        app.update(Action::SearchNext);
        assert_eq!(current(&app), first, "only one match on the left");
        app.update(Action::FocusPane(Pane::Right));
        app.update(Action::SearchNext);
        assert_eq!(text(&app), "old too");
    }

    #[test]
    fn backslash_c_ignores_case() {
        let mut app = App::new(session("Hello\n", "Hello\n")).unwrap();
        search(&mut app, "hello");
        assert!(message(&app).0.starts_with("E486"));
        search(&mut app, r"hello\c");
        assert_eq!(current(&app), Some((0, 0, 5)));
    }

    #[test]
    fn search_ignores_the_ignore_case_option() {
        let mut app = App::new(session("Hello\n", "Hello\n")).unwrap();
        app.update(Action::ToggleCase);
        search(&mut app, "hello");
        assert_eq!(current(&app), None, "use \\c to ignore case");
    }

    #[test]
    fn empty_pattern_repeats_the_last_one_in_the_new_direction() {
        let mut app = long_app(10);
        search(&mut app, "line 2[0-9]");
        app.update(Action::SearchNext);
        assert_eq!(current(&app).unwrap().0, 20);
        type_line(&mut app, Action::StartSearch(Direction::Backward), "");
        assert_eq!(current(&app).unwrap().0, 19);
        assert_eq!(
            app.search.last.as_ref().unwrap().direction,
            Direction::Backward
        );
    }

    #[test]
    fn n_without_a_search_reports_e35() {
        let mut app = long_app(10);
        app.update(Action::SearchNext);
        assert_eq!(message(&app), ("E35: No previous regular expression", true));
        search(&mut app, "");
        assert_eq!(message(&app), ("E35: No previous regular expression", true));
    }

    #[test]
    fn invalid_pattern_reports_e383_and_keeps_the_view() {
        let mut app = long_app(10);
        app.update(Action::Down(30));
        search(&mut app, "line (");
        assert_eq!(app.top, 30);
        let (text, error) = message(&app);
        assert!(
            error && text.starts_with("E383: Invalid search string: line ("),
            "{text}"
        );
        assert!(app.search.last.is_none());
    }

    #[test]
    fn typing_moves_the_view_and_escape_goes_back() {
        let mut app = long_app(10);
        app.update(Action::Down(20));
        app.update(Action::StartSearch(Direction::Forward));
        assert_eq!(app.input_mode(), InputMode::Prompt);
        for c in "line 8".chars() {
            app.update(Action::Prompt(PromptKey::Char(c)));
        }
        assert_eq!(
            current(&app).unwrap().0,
            79,
            "incremental: first match from line 21"
        );
        assert_eq!(app.top, 74);
        assert!(
            app.highlight_regex()
                .is_some_and(|re| re.as_str() == "line 8")
        );
        app.update(Action::Prompt(PromptKey::Char('(')));
        assert_eq!(
            (app.top, current(&app)),
            (20, None),
            "invalid so far: back to the start"
        );
        assert!(app.highlight_regex().is_none());
        app.update(Action::Prompt(PromptKey::Cancel));
        assert_eq!((app.top, current(&app)), (20, None));
        assert!(app.search.last.is_none() && app.message.is_none());
    }

    #[test]
    fn backspace_on_empty_search_cancels_and_restores() {
        let mut app = long_app(10);
        app.update(Action::StartSearch(Direction::Forward));
        app.update(Action::Prompt(PromptKey::Char('5')));
        assert_eq!(current(&app), Some((4, 5, 6)));
        app.update(Action::Prompt(PromptKey::Backspace));
        assert_eq!(current(&app), None, "empty pattern: back at the start");
        app.update(Action::Prompt(PromptKey::Backspace));
        assert_eq!(app.input_mode(), InputMode::Normal);
    }

    #[test]
    fn submit_searches_from_where_typing_started() {
        let mut app = long_app(10);
        search(&mut app, "line 1");
        assert_eq!(
            current(&app).unwrap().0,
            0,
            "not one past the incremental match"
        );
    }

    #[test]
    fn n_continues_from_the_top_of_the_screen_after_scrolling_away() {
        let mut app = long_app(10);
        search(&mut app, "line 1");
        assert_eq!(current(&app).unwrap().0, 0);
        app.update(Action::Down(5));
        assert_eq!(current(&app), None, "match scrolled off screen");
        app.update(Action::SearchNext);
        assert_eq!(
            current(&app).unwrap().0,
            9,
            "line 10, the first match at or below row 5"
        );
        assert_eq!(app.top, 5, "already on screen");
        app.update(Action::Down(2));
        assert!(
            current(&app).is_some(),
            "still on screen, so n continues from it"
        );
    }

    #[test]
    fn noh_hides_highlights_until_the_next_search() {
        let mut app = long_app(10);
        search(&mut app, "line");
        assert!(app.highlight_regex().is_some());
        command(&mut app, "noh");
        assert!(app.highlight_regex().is_none());
        assert!(app.message.is_none());
        app.update(Action::SearchNext);
        assert!(app.highlight_regex().is_some());
    }

    #[test]
    fn colon_help_opens_help_and_search_closes_it() {
        let mut app = long_app(10);
        command(&mut app, "h");
        assert!(app.show_help);
        app.update(Action::StartSearch(Direction::Forward));
        assert!(!app.show_help, "opening the search line closes help");
    }

    #[test]
    fn f1_toggles_only_the_sidebar() {
        let mut app = long_app(10);
        app.update(Action::ToggleSidebar);
        assert!(!app.sidebar.open && !app.show_help);
        app.update(Action::ToggleSidebar);
        assert!(app.sidebar.open && !app.show_help);
    }

    #[test]
    fn colon_sidebar_toggles_and_docks() {
        let mut app = long_app(10);
        command(&mut app, "sidebar");
        assert!(!app.sidebar.open);
        command(&mut app, "sidebar left");
        assert_eq!(
            app.sidebar,
            Sidebar {
                open: true,
                dock: Dock::Left
            }
        );
        command(&mut app, "sidebar right");
        assert_eq!(app.sidebar.dock, Dock::Right);
        command(&mut app, "sidebar middle");
        assert_eq!(
            message(&app),
            ("E475: Invalid argument: middle (left or right)", true)
        );
    }

    #[test]
    fn colon_quit_commands() {
        let mut app = long_app(10);
        command(&mut app, "q");
        assert!(app.should_quit && !app.quit_all);
        let mut app = long_app(10);
        command(&mut app, "qa!");
        assert!(app.should_quit && app.quit_all);
        let mut app = long_app(10);
        command(&mut app, "wq");
        assert!(!app.should_quit);
        assert_eq!(message(&app), ("E492: Not an editor command: wq", true));
    }

    #[test]
    fn messages_last_until_the_next_key_but_survive_redraws() {
        let mut app = long_app(10);
        search(&mut app, "line 3");
        app.update(Action::Resize {
            width: 80,
            height: 12,
        });
        assert!(app.message.is_some());
        app.update(Action::Down(1));
        assert!(app.message.is_none());
    }

    #[test]
    fn layout_toggle_keeps_the_current_match() {
        let mut app = long_app(10);
        app.update(Action::FocusPane(Pane::Right));
        search(&mut app, "ninety");
        app.update(Action::ToggleLayout);
        let m = app.search.current.unwrap();
        let line = app.display_line(m.row).unwrap();
        assert_eq!(app.lines[line].side, LineSide::Right, "inline: the + line");
        app.update(Action::FocusPane(Pane::Left));
        assert_eq!(
            app.display_line(m.row).map(|l| app.lines[l].side),
            Some(LineSide::Left)
        );
    }

    #[test]
    fn option_toggle_forgets_the_current_match() {
        let mut app = long_app(10);
        search(&mut app, "line 2");
        app.update(Action::CycleWhitespace);
        assert_eq!(current(&app), None);
        assert!(app.search.last.is_some());
    }

    #[test]
    fn far_right_match_scrolls_horizontally() {
        let long = format!("{}needle\n", "x".repeat(200));
        let mut app = App::new(session(&long, &long)).unwrap();
        app.update(Action::Resize {
            width: 80,
            height: 12,
        });
        let cols = crate::view::text_width(&app);
        search(&mut app, "needle");
        assert_eq!(
            app.hscroll,
            206 - cols,
            "centring would scroll past the line end, so the end sits at the right edge"
        );
        search(&mut app, "^x");
        assert_eq!(
            app.hscroll, 0,
            "fits without scrolling: back to the left edge"
        );
    }

    #[test]
    fn active_pane_shows_in_the_sidebar_value() {
        let mut app = long_app(10);
        assert_eq!(app.command_value(Action::SwitchPane), Some(("left", false)));
        app.update(Action::SwitchPane);
        assert_eq!(
            app.command_value(Action::SwitchPane),
            Some(("right", false))
        );
        app.update(Action::SwitchPane);
        assert_eq!(app.focus, Pane::Left);
    }

    /// One `width`-column line on the left, short lines elsewhere, in a 80x12 terminal.
    fn wide_app(left_line: &str) -> App {
        let mut app = App::new(session(&format!("{left_line}\nb\n"), "a\nb\n")).unwrap();
        app.update(Action::Resize {
            width: 80,
            height: 12,
        });
        app
    }

    fn scroll_right_until_stuck(app: &mut App) {
        for _ in 0..100 {
            app.update(Action::ScrollRight);
        }
    }

    #[test]
    fn horizontal_scroll_never_negative() {
        let mut app = wide_app(&"x".repeat(300));
        app.update(Action::ScrollLeft);
        assert_eq!(app.hscroll, 0);
        app.update(Action::ScrollRight);
        assert_eq!(app.hscroll, HSCROLL_STEP);
    }

    #[test]
    fn right_scroll_stops_when_the_widest_line_ends_at_the_edge() {
        let mut app = wide_app(&"x".repeat(300));
        let cols = crate::view::text_width(&app);
        scroll_right_until_stuck(&mut app);
        assert_eq!(app.hscroll, 300 - cols);
        app.update(Action::ScrollLeft);
        assert_eq!(app.hscroll, 300 - cols - HSCROLL_STEP);
    }

    #[test]
    fn lines_that_fit_do_not_scroll_right() {
        let mut app = long_app(10);
        app.update(Action::ScrollRight);
        assert_eq!(app.hscroll, 0);
    }

    #[rstest::rstest]
    #[case::tabs_expand_to_four_columns("\t".repeat(50), 200)]
    #[case::wide_characters_take_two_columns("日".repeat(100), 200)]
    fn right_scroll_limit_uses_display_width(#[case] line: String, #[case] cols: usize) {
        let mut app = wide_app(&line);
        assert_eq!(app.widest, cols);
        scroll_right_until_stuck(&mut app);
        assert_eq!(app.hscroll, cols - crate::view::text_width(&app));
    }

    #[test]
    fn right_side_lines_count_too() {
        let mut app = App::new(session("a\n", &format!("{}\n", "y".repeat(300)))).unwrap();
        app.update(Action::Resize {
            width: 80,
            height: 12,
        });
        scroll_right_until_stuck(&mut app);
        assert_eq!(app.hscroll, 300 - crate::view::text_width(&app));
    }

    #[rstest::rstest]
    #[case::wider_terminal(Action::Resize { width: 250, height: 12 })]
    #[case::inline_layout_is_wider(Action::ToggleLayout)]
    #[case::closing_the_sidebar(Action::ToggleSidebar)]
    fn more_room_pulls_the_view_back_to_the_line_end(#[case] action: Action) {
        let mut app = wide_app(&"x".repeat(150));
        // Wide enough that the sidebar takes room from the text.
        app.update(Action::Resize {
            width: 120,
            height: 12,
        });
        scroll_right_until_stuck(&mut app);
        let before = app.hscroll;
        app.update(action);
        let cols = crate::view::text_width(&app);
        assert_eq!(app.hscroll, 150usize.saturating_sub(cols));
        assert!(app.hscroll < before, "{before} -> {}", app.hscroll);
    }
}
