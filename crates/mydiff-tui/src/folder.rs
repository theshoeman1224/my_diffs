//! Folder view: a list of compared paths; Enter opens a file diff.

use std::path::PathBuf;

use mydiff_core::detect::looks_binary;
use mydiff_core::folder::{EntryStatus, FolderDiff, diff_folders};
use mydiff_core::{DiffOptions, Result, RowKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{Action, App, CHROME_ROWS, Sidebar, TextSession, option_value};
use crate::cmdline::{CmdLine, Command, PromptEvent, PromptKey, PromptKind, parse_command};
use crate::hex::{HexApp, HexSession};
use crate::keys::{InputMode, ScreenKind};
use crate::{hex, view};

/// A file comparison opened from the folder list.
#[derive(Debug)]
pub enum Child {
    Text(Box<App>),
    Hex(Box<HexApp>),
}

impl Child {
    fn update(&mut self, action: Action) {
        match self {
            Self::Text(a) => a.update(action),
            Self::Hex(a) => a.update(action),
        }
    }

    fn input_mode(&self) -> InputMode {
        match self {
            Self::Text(a) => a.input_mode(),
            Self::Hex(a) => a.input_mode(),
        }
    }

    /// (`:q` closed it, `:qa` asked to quit everything).
    fn exit(&self) -> (bool, bool) {
        match self {
            Self::Text(a) => (a.should_quit, a.quit_all),
            Self::Hex(a) => (a.should_quit, a.quit_all),
        }
    }

    fn show_help(&self) -> bool {
        match self {
            Self::Text(a) => a.show_help,
            Self::Hex(a) => a.show_help,
        }
    }

    fn sidebar(&self) -> Sidebar {
        match self {
            Self::Text(a) => a.sidebar,
            Self::Hex(a) => a.sidebar,
        }
    }

    fn render(&self, frame: &mut Frame) {
        match self {
            Self::Text(a) => view::render(a, frame),
            Self::Hex(a) => hex::render(a, frame),
        }
    }
}

/// Inputs for an interactive folder comparison.
#[derive(Debug, Clone)]
pub struct FolderSession {
    pub left_root: PathBuf,
    pub right_root: PathBuf,
    /// Shown instead of the roots (e.g. `a/` and `b/` for git temp dirs).
    pub left_label: String,
    pub right_label: String,
    pub options: DiffOptions,
}

impl FolderSession {
    /// Compare two roots, labelled by their own paths.
    pub fn new(left_root: PathBuf, right_root: PathBuf, options: DiffOptions) -> Self {
        Self {
            left_label: left_root.display().to_string(),
            right_label: right_root.display().to_string(),
            left_root,
            right_root,
            options,
        }
    }

    /// Display label of `rel` under the left or right root.
    pub fn label(&self, right: bool, rel: &str) -> String {
        let root = if right {
            &self.right_label
        } else {
            &self.left_label
        };
        std::path::Path::new(root).join(rel).display().to_string()
    }
}

#[derive(Debug)]
pub struct FolderApp {
    pub session: FolderSession,
    pub diff: FolderDiff,
    /// Indices into `diff.entries` currently listed.
    pub visible: Vec<usize>,
    /// Index into `visible`.
    pub selected: usize,
    pub top: usize,
    pub viewport: usize,
    pub width: u16,
    pub show_same: bool,
    /// Open file diff, if any.
    pub child: Option<Child>,
    /// One-shot notice shown in the status bar.
    pub message: Option<String>,
    pub show_help: bool,
    /// Shared with an opened file diff: handed over on open, taken back on return.
    pub sidebar: Sidebar,
    /// The `:` line (no search in the folder list).
    pub cmdline: CmdLine,
    pub should_quit: bool,
}

/// Narrowest folder list that still leaves room for the sidebar.
pub const MIN_LIST_COLS: u16 = 50;

impl FolderApp {
    pub fn new(session: FolderSession) -> Result<Self> {
        let diff = diff_folders(&session.left_root, &session.right_root, &session.options)?;
        let mut app = Self {
            session,
            diff,
            visible: Vec::new(),
            selected: 0,
            top: 0,
            viewport: 20,
            width: 80,
            show_same: false,
            child: None,
            message: None,
            show_help: false,
            sidebar: Sidebar::default(),
            cmdline: CmdLine::default(),
            should_quit: false,
        };
        app.rebuild_visible(None);
        Ok(app)
    }

    pub fn update(&mut self, action: Action) {
        if let Action::Resize { width, height } = action {
            self.width = width;
            self.viewport = usize::from(height.saturating_sub(CHROME_ROWS)).max(1);
            if let Some(child) = &mut self.child {
                child.update(action);
            }
            self.keep_selection_visible();
            return;
        }
        if action == Action::ForceQuit {
            self.should_quit = true;
            return;
        }
        if self.child.is_some() {
            self.update_child(action);
            return;
        }
        self.message = None;
        // Nothing to search here, so n / N step through changes.
        let action = match action {
            Action::SearchNext => Action::NextChange,
            Action::SearchPrev => Action::PrevChange,
            a => a,
        };
        match action {
            Action::StartCommand => {
                self.show_help = false;
                self.cmdline.open(PromptKind::Command);
            }
            Action::Prompt(key) => self.prompt_key(key),
            Action::Quit => self.should_quit = true,
            Action::Back => {
                if self.show_help {
                    self.show_help = false;
                } else {
                    self.should_quit = true;
                }
            }
            Action::ToggleSidebar | Action::DockSidebar(_) => self.sidebar.update(action),
            Action::Up(n) => self.select(self.selected.saturating_sub(n)),
            Action::Down(n) => self.select(self.selected + n),
            Action::PageUp => self.select(self.selected.saturating_sub(self.viewport.max(2) - 1)),
            Action::PageDown => self.select(self.selected + self.viewport.max(2) - 1),
            Action::Top => self.select(0),
            Action::Bottom => self.select(usize::MAX),
            Action::NextChange => {
                if let Some(i) =
                    (self.selected + 1..self.visible.len()).find(|&i| self.is_change(i))
                {
                    self.select(i);
                }
            }
            Action::PrevChange => {
                if let Some(i) = (0..self.selected).rev().find(|&i| self.is_change(i)) {
                    self.select(i);
                }
            }
            Action::ToggleSame => {
                self.show_same = !self.show_same;
                let path = self.selected_path();
                self.rebuild_visible(path);
            }
            Action::CycleWhitespace => {
                self.session.options.whitespace = self.session.options.whitespace.cycle();
                self.refresh();
            }
            Action::ToggleCase => {
                self.session.options.ignore_case ^= true;
                self.refresh();
            }
            Action::ToggleBlankLines => {
                self.session.options.ignore_blank_lines ^= true;
                self.refresh();
            }
            Action::ToggleLineEndings => {
                self.session.options.ignore_line_endings ^= true;
                self.refresh();
            }
            Action::Open => self.open(),
            Action::ForceQuit
            | Action::Resize { .. }
            | Action::ScrollLeft
            | Action::ScrollRight
            | Action::ToggleLayout
            | Action::StartSearch(_)
            | Action::SearchNext
            | Action::SearchPrev
            | Action::SwitchPane
            | Action::FocusPane(_) => {}
        }
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Keys go to the open file diff; `q`, `Esc` and `:q` return to the list.
    fn update_child(&mut self, action: Action) {
        let Some(child) = &mut self.child else {
            return;
        };
        match action {
            Action::Quit => self.close_child(),
            Action::Back if !child.show_help() => self.close_child(),
            _ => {
                child.update(action);
                match child.exit() {
                    (_, true) => self.should_quit = true,
                    (true, false) => self.close_child(),
                    _ => {}
                }
            }
        }
    }

    fn prompt_key(&mut self, key: PromptKey) {
        if let PromptEvent::Submitted(_, line) = self.cmdline.key(key) {
            match parse_command(&line) {
                Ok(Some(Command::Help)) => self.show_help = true,
                Ok(Some(Command::Sidebar(dock))) => self.sidebar.command(dock),
                Ok(Some(Command::Quit | Command::QuitAll)) => self.should_quit = true,
                Ok(Some(Command::NoHighlight) | None) => {}
                Err(e) => self.message = Some(e),
            }
        }
    }

    /// The sidebar as the user sees it now: an open file diff holds it until
    /// it returns to the list.
    pub fn current_sidebar(&self) -> Sidebar {
        self.child.as_ref().map_or(self.sidebar, Child::sidebar)
    }

    /// Whether keys should edit a command line (this list's or the open file's).
    pub fn input_mode(&self) -> InputMode {
        match &self.child {
            Some(child) => child.input_mode(),
            None if self.cmdline.is_open() => InputMode::Prompt,
            None => InputMode::Normal,
        }
    }

    /// Live value shown next to a command in the sidebar.
    pub fn command_value(&self, action: Action) -> Option<(&'static str, bool)> {
        match action {
            Action::ToggleSame => Some(if self.show_same {
                ("shown", true)
            } else {
                ("hidden", false)
            }),
            _ => option_value(&self.session.options, action),
        }
    }

    fn close_child(&mut self) {
        if let Some(child) = self.child.take() {
            self.sidebar = child.sidebar();
        }
    }

    fn is_change(&self, visible_idx: usize) -> bool {
        self.diff.entries[self.visible[visible_idx]]
            .status
            .is_change()
    }

    fn selected_path(&self) -> Option<String> {
        self.visible
            .get(self.selected)
            .map(|&i| self.diff.entries[i].path.clone())
    }

    fn select(&mut self, i: usize) {
        self.selected = i.min(self.visible.len().saturating_sub(1));
        self.keep_selection_visible();
    }

    fn keep_selection_visible(&mut self) {
        if self.selected < self.top {
            self.top = self.selected;
        } else if self.selected >= self.top + self.viewport {
            self.top = self.selected + 1 - self.viewport;
        }
    }

    fn rebuild_visible(&mut self, keep: Option<String>) {
        self.visible = (0..self.diff.entries.len())
            .filter(|&i| self.show_same || self.diff.entries[i].status.is_change())
            .collect();
        let pos = keep.and_then(|p| {
            self.visible
                .iter()
                .position(|&i| self.diff.entries[i].path >= p)
        });
        self.top = 0;
        self.select(pos.unwrap_or(0));
    }

    /// Re-run the folder comparison after an option toggle.
    fn refresh(&mut self) {
        let s = &self.session;
        match diff_folders(&s.left_root, &s.right_root, &s.options) {
            Ok(d) => {
                let path = self.selected_path();
                self.diff = d;
                self.rebuild_visible(path);
            }
            Err(e) => self.message = Some(e.to_string()),
        }
    }

    fn open(&mut self) {
        let Some(&idx) = self.visible.get(self.selected) else {
            return;
        };
        let entry = &self.diff.entries[idx];
        if entry.is_dir {
            self.message = Some(format!("{}/ exists on one side only", entry.path));
            return;
        }
        if entry.status == EntryStatus::TypeMismatch {
            self.message = Some(format!(
                "{} is a file on one side, a directory on the other",
                entry.path
            ));
            return;
        }
        let read = |root: &PathBuf, present: bool| -> std::io::Result<Vec<u8>> {
            if present {
                std::fs::read(root.join(&entry.path))
            } else {
                Ok(Vec::new())
            }
        };
        let left = read(
            &self.session.left_root,
            entry.status != EntryStatus::OnlyRight,
        );
        let right = read(
            &self.session.right_root,
            entry.status != EntryStatus::OnlyLeft,
        );
        let (left, right) = match (left, right) {
            (Ok(l), Ok(r)) => (l, r),
            (Err(e), _) | (_, Err(e)) => {
                self.message = Some(format!("{}: {e}", entry.path));
                return;
            }
        };
        let size = Action::Resize {
            width: self.width,
            height: u16::try_from(self.viewport)
                .unwrap_or(u16::MAX)
                .saturating_add(CHROME_ROWS),
        };
        if looks_binary(&left) || looks_binary(&right) {
            let mut child = HexApp::new(HexSession {
                left_label: self.session.label(false, &entry.path),
                right_label: self.session.label(true, &entry.path),
                diff: mydiff_core::binary::diff_bytes(left, right),
            });
            child.update(size);
            child.sidebar = self.sidebar;
            self.child = Some(Child::Hex(Box::new(child)));
            return;
        }
        let session = TextSession {
            left_label: self.session.label(false, &entry.path),
            right_label: self.session.label(true, &entry.path),
            left: String::from_utf8_lossy(&left).into_owned(),
            right: String::from_utf8_lossy(&right).into_owned(),
            options: self.session.options.clone(),
        };
        match App::new(session) {
            Ok(mut child) => {
                child.update(size);
                child.sidebar = self.sidebar;
                self.child = Some(Child::Text(Box::new(child)));
            }
            Err(e) => self.message = Some(e.to_string()),
        }
    }
}

fn status_style(s: &EntryStatus) -> Style {
    match s {
        EntryStatus::Same => view::row_style(RowKind::Equal),
        EntryStatus::Different => view::row_style(RowKind::Modified),
        EntryStatus::OnlyLeft => view::row_style(RowKind::Removed),
        EntryStatus::OnlyRight => view::row_style(RowKind::Added),
        EntryStatus::TypeMismatch => Style::new().fg(Color::Magenta),
        EntryStatus::Error(_) => Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
    }
}

pub fn render(app: &FolderApp, frame: &mut Frame) {
    if let Some(child) = &app.child {
        child.render(frame);
        return;
    }
    let (main, sidebar) = view::split_sidebar(frame.area(), app.sidebar, MIN_LIST_COLS);
    let [header, body, status] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(main);
    let w = usize::from(header.width);
    let title = format!(
        " {}  ⇄  {}",
        app.session.left_label, app.session.right_label
    );
    frame.render_widget(
        Paragraph::new(view::pad(&title, w)).style(view::bar()),
        header,
    );
    render_list(app, frame, body);
    render_status(app, frame, status);
    if let Some(area) = sidebar {
        view::render_sidebar(frame, area, app.sidebar.dock, ScreenKind::Folder, &|a| {
            app.command_value(a)
        });
    }
    if app.show_help {
        view::render_help(frame, ScreenKind::Folder);
    }
}

fn render_list(app: &FolderApp, frame: &mut Frame, area: Rect) {
    let w = usize::from(area.width);
    let lines: Vec<Line> = if app.visible.is_empty() {
        vec![Line::styled(
            "  no differences (press s to show identical files)",
            view::dim(),
        )]
    } else {
        app.visible
            .iter()
            .enumerate()
            .skip(app.top)
            .take(usize::from(area.height))
            .map(|(vi, &ei)| {
                let e = &app.diff.entries[ei];
                let slash = if e.is_dir { "/" } else { "" };
                let mut text = format!(" {} {}{slash}", e.status.marker(), e.path);
                if let EntryStatus::Error(msg) = &e.status {
                    text.push_str(&format!("  ({msg})"));
                }
                let mut style = status_style(&e.status);
                if vi == app.selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                Line::from(Span::styled(view::pad(&text, w), style))
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_status(app: &FolderApp, frame: &mut Frame, area: Rect) {
    if let Some(prompt) = &app.cmdline.prompt {
        view::render_prompt(frame, area, prompt);
        return;
    }
    let o = &app.session.options;
    let s = app.diff.stats();
    let left = app.message.clone().unwrap_or_else(|| {
        format!(
            " [s]ame:{} [w]s:{} [i]case:{}",
            if app.show_same { "shown" } else { "hidden" },
            o.whitespace.label(),
            if o.ignore_case { "on" } else { "off" },
        )
    });
    let right = format!(
        "~{} -{} +{} ={}  enter:open F1:keys :help q:quit ",
        s.different,
        s.only_left,
        s.only_right + s.type_mismatch + s.errors,
        s.same
    );
    view::render_status_line(frame, area, &left, &right);
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;

    struct Fixture {
        _l: tempfile::TempDir,
        _r: tempfile::TempDir,
        app: FolderApp,
    }

    fn write(root: &Path, rel: &str, body: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn fixture() -> Fixture {
        let l = tempfile::tempdir().unwrap();
        let r = tempfile::tempdir().unwrap();
        write(l.path(), "a_changed.txt", b"one\ntwo\n");
        write(r.path(), "a_changed.txt", b"one\ntwo!\n");
        write(l.path(), "b_same.txt", b"same\n");
        write(r.path(), "b_same.txt", b"same\n");
        write(l.path(), "c_only_left.txt", b"bye\n");
        write(r.path(), "d_only_right/x.txt", b"hi\n");
        write(l.path(), "e_bin", b"\0\x01");
        write(r.path(), "e_bin", b"\0\x02");
        write(l.path(), "f_ws.txt", b"a  b\n");
        write(r.path(), "f_ws.txt", b"a b\n");
        let mut app = FolderApp::new(FolderSession::new(
            l.path().to_path_buf(),
            r.path().to_path_buf(),
            DiffOptions::default(),
        ))
        .unwrap();
        app.update(Action::Resize {
            width: 60,
            height: 10,
        });
        Fixture { _l: l, _r: r, app }
    }

    fn paths(app: &FolderApp) -> Vec<&str> {
        app.visible
            .iter()
            .map(|&i| app.diff.entries[i].path.as_str())
            .collect()
    }

    #[test]
    fn lists_only_changes_by_default() {
        let f = fixture();
        assert_eq!(
            paths(&f.app),
            [
                "a_changed.txt",
                "c_only_left.txt",
                "d_only_right",
                "e_bin",
                "f_ws.txt"
            ]
        );
    }

    #[test]
    fn toggle_same_keeps_selection() {
        let mut f = fixture();
        f.app.update(Action::Down(1)); // c_only_left.txt
        f.app.update(Action::ToggleSame);
        assert!(paths(&f.app).contains(&"b_same.txt"));
        assert_eq!(f.app.selected_path().as_deref(), Some("c_only_left.txt"));
    }

    #[test]
    fn whitespace_toggle_refreshes_listing() {
        let mut f = fixture();
        for _ in 0..2 {
            f.app.update(Action::CycleWhitespace); // trailing, then change
        }
        assert!(!paths(&f.app).contains(&"f_ws.txt"));
    }

    #[test]
    fn open_text_file_then_back() {
        let mut f = fixture();
        f.app.update(Action::Open);
        let Some(Child::Text(child)) = f.app.child.as_ref() else {
            panic!("text diff opened")
        };
        assert_eq!(child.diff.stats().modified, 1);
        assert_eq!(
            child.viewport, f.app.viewport,
            "child sized like the parent"
        );
        f.app.update(Action::Down(1));
        f.app.update(Action::Back);
        assert!(f.app.child.is_none() && !f.app.should_quit);
    }

    #[test]
    fn open_one_sided_file_shows_all_removed() {
        let mut f = fixture();
        f.app.update(Action::Down(1));
        f.app.update(Action::Open);
        let Some(Child::Text(child)) = f.app.child.as_ref() else {
            panic!("text diff opened")
        };
        assert_eq!(child.diff.stats().removed, 1);
        f.app.update(Action::Quit);
        assert!(
            f.app.child.is_none() && !f.app.should_quit,
            "q in a file diff goes back"
        );
    }

    #[test]
    fn open_dir_shows_message_and_binary_opens_hex() {
        let mut f = fixture();
        f.app.update(Action::Down(2));
        f.app.update(Action::Open);
        assert!(f.app.child.is_none());
        assert!(f.app.message.as_deref().unwrap().contains("one side only"));
        f.app.update(Action::Down(1));
        f.app.update(Action::Open);
        let Some(Child::Hex(hex)) = f.app.child.as_ref() else {
            panic!("hex view opened")
        };
        assert_eq!(hex.session.diff.stats().differing_bytes, 1);
    }

    #[test]
    fn labels_replace_roots_in_header_and_opened_files() {
        let mut f = fixture();
        f.app.session.left_label = "a/".into();
        f.app.session.right_label = "b/".into();
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        t.draw(|fr| render(&f.app, fr)).unwrap();
        let header = t.backend().to_string().lines().next().unwrap().to_owned();
        assert!(header.contains("a/  ⇄  b/"), "{header}");
        f.app.update(Action::Open);
        let Some(Child::Text(child)) = f.app.child.as_ref() else {
            panic!("text diff opened")
        };
        assert_eq!(child.session.left_label, "a/a_changed.txt");
        assert_eq!(child.session.right_label, "b/a_changed.txt");
    }

    #[test]
    fn sidebar_state_carries_into_file_and_back() {
        let mut f = fixture();
        f.app.update(Action::DockSidebar(crate::app::Dock::Left));
        f.app.update(Action::Open);
        let Some(Child::Text(child)) = f.app.child.as_ref() else {
            panic!("text diff opened")
        };
        assert_eq!(
            child.sidebar, f.app.sidebar,
            "file view inherits the sidebar"
        );
        f.app.update(Action::ToggleSidebar); // hide it inside the file view
        assert!(
            f.app.sidebar.open,
            "the folder's copy is untouched while a file is open"
        );
        f.app.update(Action::Back);
        assert!(f.app.child.is_none());
        assert!(!f.app.sidebar.open, "hidden in the file view stays hidden");
        assert_eq!(f.app.sidebar.dock, crate::app::Dock::Left);
    }

    #[test]
    fn sidebar_carries_into_hex_view() {
        let mut f = fixture();
        f.app.update(Action::ToggleSidebar);
        f.app.update(Action::Down(3)); // e_bin
        f.app.update(Action::Open);
        let Some(Child::Hex(hex)) = f.app.child.as_ref() else {
            panic!("hex view opened")
        };
        assert!(!hex.sidebar.open);
        f.app.update(Action::ToggleSidebar);
        f.app.update(Action::Quit);
        assert!(f.app.sidebar.open);
    }

    #[test]
    fn folder_sidebar_lists_folder_commands_with_values() {
        let mut f = fixture();
        f.app.update(Action::ToggleSame);
        f.app.update(Action::Resize {
            width: 90,
            height: 24,
        });
        let mut t = Terminal::new(TestBackend::new(90, 24)).unwrap();
        t.draw(|fr| render(&f.app, fr)).unwrap();
        // The header row holds temp-dir roots; snapshot everything below it.
        let text = t.backend().to_string();
        let body: Vec<&str> = text.lines().skip(1).collect();
        insta::assert_snapshot!(body.join("\n"));
    }

    #[test]
    fn force_quit_from_child() {
        let mut f = fixture();
        f.app.update(Action::Open);
        f.app.update(Action::ForceQuit);
        assert!(f.app.should_quit);
    }

    #[test]
    fn selection_clamps_and_scrolls() {
        let mut f = fixture();
        f.app.update(Action::Resize {
            width: 60,
            height: 4,
        }); // 2 body rows
        f.app.update(Action::Bottom);
        assert_eq!((f.app.selected, f.app.top), (4, 3));
        f.app.update(Action::Top);
        assert_eq!((f.app.selected, f.app.top), (0, 0));
    }

    #[test]
    fn screen() {
        let f = fixture();
        let mut t = Terminal::new(TestBackend::new(60, 10)).unwrap();
        t.draw(|fr| render(&f.app, fr)).unwrap();
        let text = t.backend().to_string();
        // Roots are temp dirs; check the stable parts.
        let body: Vec<&str> = text.lines().skip(1).take(6).collect();
        insta::assert_snapshot!(body.join("\n"), @r#"
        " ~ a_changed.txt                                            "
        " - c_only_left.txt                                          "
        " + d_only_right/                                            "
        " ~ e_bin                                                    "
        " ~ f_ws.txt                                                 "
        "                                                            "
        "#);
        assert!(text.lines().last().unwrap().contains("~3 -1 +1 =1"));
    }

    fn command(app: &mut FolderApp, line: &str) {
        app.update(Action::StartCommand);
        assert_eq!(app.input_mode(), InputMode::Prompt);
        for c in line.chars() {
            app.update(Action::Prompt(PromptKey::Char(c)));
        }
        app.update(Action::Prompt(PromptKey::Submit));
    }

    #[test]
    fn colon_q_in_an_opened_file_returns_to_the_list() {
        let mut f = fixture();
        f.app.update(Action::Open);
        command(&mut f.app, "q");
        assert!(f.app.child.is_none() && !f.app.should_quit);
        command(&mut f.app, "q");
        assert!(f.app.should_quit, ":q on the list quits");
    }

    #[test]
    fn colon_qa_in_an_opened_file_quits_everything() {
        let mut f = fixture();
        f.app.update(Action::Open);
        command(&mut f.app, "qa");
        assert!(f.app.should_quit);
    }

    #[test]
    fn opened_file_gets_search_keys() {
        let mut f = fixture();
        f.app.update(Action::Open);
        f.app
            .update(Action::StartSearch(crate::search::Direction::Forward));
        assert_eq!(
            f.app.input_mode(),
            InputMode::Prompt,
            "the file's search line"
        );
        f.app.update(Action::Prompt(PromptKey::Char('t')));
        f.app.update(Action::Prompt(PromptKey::Cancel));
        assert_eq!(f.app.input_mode(), InputMode::Normal);
        assert!(
            f.app.child.is_some(),
            "Esc cancelled the search, not the file"
        );
    }

    #[test]
    fn n_steps_through_changes_in_the_list() {
        let mut f = fixture();
        f.app.update(Action::SearchNext);
        assert_eq!(f.app.selected_path().as_deref(), Some("c_only_left.txt"));
        f.app.update(Action::SearchPrev);
        assert_eq!(f.app.selected_path().as_deref(), Some("a_changed.txt"));
        f.app
            .update(Action::StartSearch(crate::search::Direction::Forward));
        assert_eq!(
            f.app.input_mode(),
            InputMode::Normal,
            "no search in the list"
        );
    }

    #[test]
    fn colon_help_and_errors_in_the_list() {
        let mut f = fixture();
        command(&mut f.app, "h");
        assert!(f.app.show_help);
        command(&mut f.app, "x");
        assert_eq!(
            f.app.message.as_deref(),
            Some("E492: Not an editor command: x")
        );
    }
}
