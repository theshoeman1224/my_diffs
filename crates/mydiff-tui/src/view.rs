//! Pure rendering: `render(&App, &mut Frame)`. Uses only the 16 basic colors so it
//! looks the same in xterm, `PuTTY` and Windows Terminal.

use std::ops::Range;

use mydiff_core::display::{sanitize, width};
use mydiff_core::{LineRef, Row, RowKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{Action, App, DisplayLine, Dock, Layout, LineSide, Pane, Sidebar};
use crate::cmdline::{Message, Prompt};
use crate::keys::{Group, SHORT_KEYS_W, ScreenKind, commands};

/// Width of a line-number column including its trailing space.
const NUM_W: usize = 5;
/// Width of the side-by-side gutter (" ~ ").
const GUTTER_W: usize = 3;
/// Width of the command sidebar, including its border.
pub const SIDEBAR_W: u16 = 34;
/// Narrowest text diff that still leaves room for the sidebar.
pub const MIN_TEXT_COLS: u16 = 60;

pub fn row_style(kind: RowKind) -> Style {
    match kind {
        RowKind::Equal => Style::new(),
        RowKind::Removed => Style::new().fg(Color::Red),
        RowKind::Added => Style::new().fg(Color::Green),
        RowKind::Modified => Style::new().fg(Color::Yellow),
        RowKind::Ignored => Style::new().fg(Color::DarkGray),
    }
}

/// Changed part of a modified line.
pub fn hot_style() -> Style {
    Style::new()
        .fg(Color::Yellow)
        .add_modifier(Modifier::REVERSED | Modifier::BOLD)
}

/// A search match. Underlined so it shows without color too.
pub fn match_style() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Cyan)
        .add_modifier(Modifier::UNDERLINED)
}

/// The match the view jumped to (`n` continues from it).
pub fn current_match_style() -> Style {
    Style::new()
        .fg(Color::Black)
        .bg(Color::Magenta)
        .add_modifier(Modifier::UNDERLINED | Modifier::BOLD)
}

/// An error message in the status bar (vim's `E…` messages).
pub fn error_style() -> Style {
    Style::new()
        .fg(Color::White)
        .bg(Color::Red)
        .remove_modifier(Modifier::REVERSED)
}

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn bar() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn render(app: &App, frame: &mut Frame) {
    let (main, sidebar) = split_sidebar(frame.area(), app.sidebar, MIN_TEXT_COLS);
    let [header, body, status] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(main);
    render_header(app, frame, header);
    render_body(app, frame, body);
    render_status(app, frame, status);
    if let Some(area) = sidebar {
        render_sidebar(frame, area, app.sidebar.dock, ScreenKind::Text, &|a| {
            app.command_value(a)
        });
    }
    if app.show_help {
        render_help(frame, ScreenKind::Text);
    }
}

/// Split a full-height sidebar column off `area` when the sidebar is open and
/// at least `min_main` columns remain for the screen itself.
pub fn split_sidebar(area: Rect, sidebar: Sidebar, min_main: u16) -> (Rect, Option<Rect>) {
    if !sidebar.open || area.width < min_main.saturating_add(SIDEBAR_W) {
        return (area, None);
    }
    let main_w = area.width - SIDEBAR_W;
    let (main_x, side_x) = match sidebar.dock {
        Dock::Left => (area.x + SIDEBAR_W, area.x),
        Dock::Right => (area.x, area.x + main_w),
    };
    let main = Rect {
        x: main_x,
        width: main_w,
        ..area
    };
    let side = Rect {
        x: side_x,
        width: SIDEBAR_W,
        ..area
    };
    (main, Some(side))
}

/// Live value of a command: (text, whether it is switched on).
pub type ValueFn<'a> = dyn Fn(Action) -> Option<(&'static str, bool)> + 'a;

/// The command sidebar: a title bar in line with the header, then every
/// command of `screen` by group, with live values right-aligned.
pub fn render_sidebar(
    frame: &mut Frame,
    area: Rect,
    dock: Dock,
    screen: ScreenKind,
    value: &ValueFn,
) {
    let [title, rest] = Split::vertical([Constraint::Length(1), Constraint::Min(0)]).areas(area);
    let w = usize::from(title.width);
    frame.render_widget(Paragraph::new(pad(" commands", w)).style(bar()), title);
    let block = Block::default().borders(match dock {
        Dock::Left => Borders::RIGHT,
        Dock::Right => Borders::LEFT,
    });
    let inner = block.inner(rest);
    frame.render_widget(block, rest);

    let iw = usize::from(inner.width);
    let bold = Style::new().add_modifier(Modifier::BOLD);
    let mut lines: Vec<Line> = Vec::new();
    for group in Group::ALL {
        let mut items = commands(screen, group).peekable();
        if items.peek().is_none() {
            continue;
        }
        if !lines.is_empty() {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(
            pad(&format!(" {}", group.title()), iw),
            bold.add_modifier(Modifier::UNDERLINED),
        ));
        for c in items {
            let key = format!(" {} ", pad(c.short, SHORT_KEYS_W));
            let (val, on) = c
                .actions
                .first()
                .and_then(|&a| value(a))
                .unwrap_or(("", false));
            // The value and one trailing space are right-aligned; the label fills the rest.
            let val_w = if val.is_empty() { 0 } else { width(val) + 1 };
            let label_w = iw.saturating_sub(width(&key) + val_w);
            lines.push(Line::from(vec![
                Span::styled(key, bold),
                Span::raw(pad(c.label, label_w)),
                Span::styled(val, if on { bold } else { Style::new() }),
            ]));
        }
    }
    let h = usize::from(inner.height);
    if lines.len() > h && h > 0 {
        lines.truncate(h - 1);
        lines.push(Line::styled(" …", dim()));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Columns of text a pane shows (after line numbers and markers).
pub fn text_width(app: &App) -> usize {
    let (main, _) = split_sidebar(Rect::new(0, 0, app.width, 1), app.sidebar, MIN_TEXT_COLS);
    let w = usize::from(main.width);
    match app.layout {
        Layout::SideBySide => (w.saturating_sub(GUTTER_W) / 2).saturating_sub(NUM_W),
        Layout::Inline => w.saturating_sub(2 * NUM_W + 2),
    }
}

/// Marks the active pane's label in the header.
pub const ACTIVE_MARK: &str = "▶";

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let s = &app.session;
    let w = usize::from(area.width);
    let mark = |pane: Pane| if app.focus == pane { ACTIVE_MARK } else { " " };
    let text = match app.layout {
        Layout::SideBySide => {
            let pane = w.saturating_sub(GUTTER_W) / 2;
            format!(
                "{}{}{}{}{}",
                mark(Pane::Left),
                pad(
                    &tail(&s.left_label, pane.saturating_sub(1)),
                    pane.saturating_sub(1)
                ),
                " ".repeat(GUTTER_W),
                mark(Pane::Right),
                tail(&s.right_label, pane.saturating_sub(1))
            )
        }
        Layout::Inline => {
            let each = w.saturating_sub(4) / 2;
            format!(
                "{}{} →{}{}",
                mark(Pane::Left),
                tail(&s.left_label, each),
                mark(Pane::Right),
                tail(&s.right_label, each)
            )
        }
    };
    frame.render_widget(Paragraph::new(pad(&text, w)).style(bar()), area);
}

/// ` {prefix}{left} → {right}` with both labels tail-truncated to fit `w`.
pub fn pair_title(prefix: &str, left: &str, right: &str, w: usize) -> String {
    let each = w.saturating_sub(width(prefix) + 4) / 2;
    format!(" {prefix}{} → {}", tail(left, each), tail(right, each))
}

/// Keep the end of `s` (file name, mode tag) when it is wider than `w`.
pub fn tail(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_owned();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1; // the ellipsis
    for c in s.chars().rev() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if used + cw > w {
            break;
        }
        used += cw;
        kept.push(c);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

fn render_body(app: &App, frame: &mut Frame, area: Rect) {
    let w = usize::from(area.width);
    let lines: Vec<Line> = app
        .lines
        .iter()
        .skip(app.top)
        .take(usize::from(area.height))
        .map(|&dl| {
            let row = &app.diff.rows[dl.row];
            let marks = marks_for(app, dl);
            match app.layout {
                Layout::SideBySide => {
                    let none = Marks::default();
                    let (left, right) = match app.focus {
                        Pane::Left => (&marks, &none),
                        Pane::Right => (&none, &marks),
                    };
                    side_by_side_line(row, w, app.hscroll, left, right)
                }
                Layout::Inline => inline_line(row, dl.side, app.focus, w, app.hscroll, &marks),
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Search highlights on one line of text, as byte ranges.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Marks {
    pub matches: Vec<Range<usize>>,
    pub current: Option<Range<usize>>,
}

/// Search highlights for display line `dl`, on the active pane's text only.
fn marks_for(app: &App, dl: DisplayLine) -> Marks {
    let Some(re) = app.highlight_regex() else {
        return Marks::default();
    };
    let Some(line) = app
        .shows_focus(dl)
        .then(|| app.focus.line(&app.diff.rows[dl.row]))
        .flatten()
    else {
        return Marks::default();
    };
    Marks {
        matches: re.find_iter(&line.text).map(|m| m.range()).collect(),
        current: app
            .search
            .current
            .filter(|m| m.row == dl.row)
            .map(|m| m.start..m.end),
    }
}

fn side_by_side_line(
    row: &Row,
    w: usize,
    hscroll: usize,
    left_marks: &Marks,
    right_marks: &Marks,
) -> Line<'static> {
    let pane = w.saturating_sub(GUTTER_W) / 2;
    let text_w = pane.saturating_sub(NUM_W);
    let style = row_style(row.kind);
    let mut spans = Vec::new();
    spans.extend(numbered(
        row.left.as_ref(),
        style,
        text_w,
        hscroll,
        left_marks,
    ));
    spans.push(Span::styled(format!(" {} ", row.kind.marker()), style));
    spans.extend(numbered(
        row.right.as_ref(),
        style,
        text_w,
        hscroll,
        right_marks,
    ));
    Line::from(spans)
}

fn numbered(
    line: Option<&LineRef>,
    style: Style,
    text_w: usize,
    hscroll: usize,
    marks: &Marks,
) -> Vec<Span<'static>> {
    match line {
        Some(l) => {
            let mut v = vec![Span::styled(format!("{:>4} ", l.number), dim())];
            v.extend(text_spans(
                &l.text, &l.changed, marks, style, hscroll, text_w,
            ));
            v
        }
        None => vec![Span::raw(" ".repeat(NUM_W + text_w))],
    }
}

fn inline_line(
    row: &Row,
    side: LineSide,
    focus: Pane,
    w: usize,
    hscroll: usize,
    marks: &Marks,
) -> Line<'static> {
    let style = row_style(row.kind);
    let num =
        |l: Option<&LineRef>| l.map_or_else(|| "     ".to_owned(), |l| format!("{:>4} ", l.number));
    let (left_no, right_no, line) = match side {
        // Rows that compare equal can still differ (ignored whitespace or case);
        // show the active pane's version, which is the text searches look at.
        LineSide::Both => (
            row.left.as_ref(),
            row.right.as_ref(),
            focus.line(row).or(focus.other().line(row)),
        ),
        LineSide::Left => (row.left.as_ref(), None, row.left.as_ref()),
        LineSide::Right => (None, row.right.as_ref(), row.right.as_ref()),
    };
    let marker = match (row.kind, side) {
        (RowKind::Modified, LineSide::Left) => '-',
        (RowKind::Modified, LineSide::Right) => '+',
        (k, _) => k.marker(),
    };
    let text_w = w.saturating_sub(2 * NUM_W + 2);
    let mut spans = vec![
        Span::styled(num(left_no), dim()),
        Span::styled(num(right_no), dim()),
        Span::styled(format!("{marker} "), style),
    ];
    if let Some(l) = line {
        spans.extend(text_spans(
            &l.text, &l.changed, marks, style, hscroll, text_w,
        ));
    }
    Line::from(spans)
}

/// Styled spans for one line's text: tabs expanded, `skip` columns hidden on the
/// left, clipped and padded to exactly `width` columns. Search marks win over
/// changed-word highlights, and the current match over other matches.
pub fn text_spans(
    text: &str,
    changed: &[Range<usize>],
    marks: &Marks,
    base: Style,
    skip: usize,
    width_cols: usize,
) -> Vec<Span<'static>> {
    let mut cuts = vec![0, text.len()];
    for r in changed
        .iter()
        .chain(&marks.matches)
        .chain(marks.current.as_ref())
    {
        cuts.extend([r.start.min(text.len()), r.end.min(text.len())]);
    }
    cuts.sort_unstable();
    cuts.dedup();
    let within = |ranges: &[Range<usize>], at: usize| ranges.iter().any(|r| r.contains(&at));
    let mut out = Vec::new();
    let mut col = 0; // display column in the expanded line
    let mut used = 0; // columns emitted
    for pair in cuts.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        let seg = &text[start..end];
        let style = if marks.current.as_ref().is_some_and(|r| r.contains(&start)) {
            current_match_style()
        } else if within(&marks.matches, start) {
            match_style()
        } else if within(changed, start) {
            hot_style()
        } else {
            base
        };
        let clean = sanitize(seg, col);
        let mut piece = String::new();
        for c in clean.chars() {
            let cw = width(c.encode_utf8(&mut [0; 4]));
            let start = col;
            col += cw;
            if start < skip {
                continue;
            }
            if used + cw > width_cols {
                break;
            }
            piece.push(c);
            used += cw;
        }
        if !piece.is_empty() {
            out.push(Span::styled(piece, style));
        }
    }
    if used < width_cols {
        out.push(Span::raw(" ".repeat(width_cols - used)));
    }
    out
}

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(prompt) = &app.cmdline.prompt {
        render_prompt(frame, area, prompt);
        return;
    }
    if let Some(message) = &app.message {
        render_message(frame, area, message);
        return;
    }
    let o = &app.session.options;
    let on = |b: bool| if b { "on" } else { "off" };
    let left = format!(
        " [w]s:{} [i]case:{} [b]lank:{} [e]ol:{}",
        o.whitespace.label(),
        on(o.ignore_case),
        on(o.ignore_blank_lines),
        if o.ignore_line_endings {
            "ignore"
        } else {
            "strict"
        },
    );
    let s = app.diff.stats();
    let position = match (app.current_change, app.change_lines.len()) {
        (_, 0) => "identical".to_owned(),
        (Some(i), n) => format!("change {}/{n}", i + 1),
        (None, n) => format!("{n} change{}", if n == 1 { "" } else { "s" }),
    };
    let right = format!(
        "-{} +{} ~{}  {position}  F1:keys :help q:quit ",
        s.removed, s.added, s.modified
    );
    render_status_line(frame, area, &left, &right);
}

/// A status bar with `left` and right-aligned `right`; the right half wins when
/// space is short and a cut left half ends in `…`.
pub fn render_status_line(frame: &mut Frame, area: Rect, left: &str, right: &str) {
    let w = usize::from(area.width);
    let budget = w.saturating_sub(width(right));
    let left = if width(left) + 1 > budget {
        format!("{}… ", pad(left, budget.saturating_sub(2)))
    } else {
        pad(left, budget)
    };
    let line = format!("{left}{right}");
    frame.render_widget(Paragraph::new(pad(&line, w)).style(bar()), area);
}

/// A message across the whole status bar; errors in white on red.
pub fn render_message(frame: &mut Frame, area: Rect, message: &Message) {
    let style = if message.error { error_style() } else { bar() };
    let text = format!(" {}", sanitize(&message.text, 1));
    let w = usize::from(area.width);
    let text = if width(&text) > w {
        format!("{}…", pad(&text, w.saturating_sub(1)))
    } else {
        pad(&text, w)
    };
    frame.render_widget(Paragraph::new(pad(&text, w)).style(style), area);
}

/// The open `/`, `?` or `:` line, like vim's command line: plain colors, the
/// terminal cursor after the text, and the start cut off when it is too long.
pub fn render_prompt(frame: &mut Frame, area: Rect, prompt: &Prompt) {
    let w = usize::from(area.width);
    let text = format!("{}{}", prompt.kind.leader(), sanitize(&prompt.input, 1));
    let shown = tail(&text, w.saturating_sub(1));
    let cursor = u16::try_from(width(&shown)).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(pad(&shown, w)), area);
    if area.width > 0 {
        frame.set_cursor_position((area.x + cursor.min(area.width - 1), area.y));
    }
}

/// Full key reference for `screen`, centered over everything.
pub fn render_help(frame: &mut Frame, screen: ScreenKind) {
    let search_legend = (screen == ScreenKind::Text).then(|| {
        Line::from(vec![
            Span::raw("search: "),
            Span::styled("match", match_style()),
            Span::raw(" "),
            Span::styled("current match", current_match_style()),
            Span::raw(format!("  active pane marked {ACTIVE_MARK}")),
        ])
    });
    let listed: Vec<_> = Group::ALL
        .into_iter()
        .flat_map(|g| commands(screen, g))
        .collect();
    let key_w = listed.iter().map(|c| width(c.keys)).max().unwrap_or(0);
    let lines: Vec<Line> = listed
        .iter()
        .map(|c| {
            Line::from(vec![
                Span::styled(
                    pad(c.keys, key_w + 2),
                    Style::new().add_modifier(Modifier::BOLD),
                ),
                Span::raw(if c.note.is_empty() {
                    c.label.to_owned()
                } else {
                    format!("{} ({})", c.label, c.note)
                }),
            ])
        })
        .chain([
            Line::raw(""),
            Line::from(vec![
                Span::styled(" removed ", row_style(RowKind::Removed)),
                Span::styled(" added ", row_style(RowKind::Added)),
                Span::styled(" modified ", row_style(RowKind::Modified)),
                Span::styled("changed text", hot_style()),
                Span::styled(" ignored ", row_style(RowKind::Ignored)),
            ]),
        ])
        .chain(search_legend)
        .chain([Line::from(vec![
            Span::styled(":help", Style::new().add_modifier(Modifier::BOLD)),
            Span::raw(" this list  "),
            Span::styled(
                ":sidebar [left|right]",
                Style::new().add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(":noh :q :qa", Style::new().add_modifier(Modifier::BOLD)),
        ])])
        .collect();
    let area = frame.area();
    let h = (lines.len() as u16 + 2).min(area.height);
    let content_w = lines.iter().map(Line::width).max().unwrap_or(0) + 2;
    let w = u16::try_from(content_w).unwrap_or(u16::MAX).min(area.width);
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" keys ")),
        popup,
    );
}

/// Pad or cut `s` to exactly `w` display columns.
pub fn pad(s: &str, w: usize) -> String {
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = width(c.encode_utf8(&mut [0; 4]));
        if used + cw > w {
            break;
        }
        out.push(c);
        used += cw;
    }
    out.push_str(&" ".repeat(w - used));
    out
}

#[cfg(test)]
mod tests {
    use mydiff_core::DiffOptions;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use super::*;
    use crate::app::TextSession;

    fn app(left: &str, right: &str, w: u16, h: u16) -> App {
        let mut app = App::new(TextSession {
            left_label: "left.txt".into(),
            right_label: "right.txt".into(),
            left: left.into(),
            right: right.into(),
            options: DiffOptions::default(),
        })
        .unwrap();
        app.update(Action::Resize {
            width: w,
            height: h,
        });
        app
    }

    fn draw(app: &App) -> Terminal<TestBackend> {
        let mut t = Terminal::new(TestBackend::new(app.width, app.viewport as u16 + 2)).unwrap();
        t.draw(|f| render(app, f)).unwrap();
        t
    }

    fn screen(app: &App) -> String {
        draw(app).backend().to_string()
    }

    const LEFT: &str = "alpha\nbeta\ngamma\ndelta\nepsilon\n";
    const RIGHT: &str = "alpha\ngamma\ndelta changed\nzeta\nepsilon\n";

    #[test]
    fn side_by_side_screen() {
        insta::assert_snapshot!(screen(&app(LEFT, RIGHT, 60, 9)));
    }

    #[test]
    fn inline_screen() {
        let mut a = app(LEFT, RIGHT, 60, 10);
        a.update(Action::ToggleLayout);
        insta::assert_snapshot!(screen(&a));
    }

    #[test]
    fn help_overlay_screen() {
        let mut a = app(LEFT, RIGHT, 80, 24);
        help(&mut a);
        insta::assert_snapshot!(screen(&a));
    }

    /// Narrowest terminal that shows the sidebar next to a text diff.
    const WIDE: u16 = MIN_TEXT_COLS + SIDEBAR_W;

    #[test]
    fn sidebar_screen() {
        insta::assert_snapshot!(screen(&app(LEFT, RIGHT, 100, 24)));
    }

    #[test]
    fn sidebar_docked_left_screen() {
        let mut a = app(LEFT, RIGHT, 100, 24);
        a.update(Action::ToggleSidebar);
        a.update(Action::DockSidebar(Dock::Left));
        insta::assert_snapshot!(screen(&a));
    }

    #[test]
    fn tab_hides_sidebar_and_diff_takes_full_width() {
        let mut a = app(LEFT, RIGHT, 100, 12);
        a.update(Action::ToggleSidebar);
        let s = screen(&a);
        assert!(!s.contains("commands"), "{s}");
        let header = s.lines().next().unwrap().trim_matches('"');
        // Right label starts past the middle of the full 100 columns.
        let col = header.find("right.txt").map(|i| width(&header[..i]));
        assert_eq!(col, Some(52), "{header}");
    }

    #[test]
    fn sidebar_hides_when_terminal_is_too_narrow() {
        assert!(!screen(&app(LEFT, RIGHT, WIDE - 1, 12)).contains("commands"));
        let s = screen(&app(LEFT, RIGHT, WIDE, 12));
        assert!(s.contains("commands"), "{s}");
        let a = app(LEFT, RIGHT, WIDE - 1, 12);
        assert!(a.sidebar.open, "hidden by size, not by the user");
    }

    #[test]
    fn sidebar_shows_live_values_bold_when_on() {
        let mut a = app(LEFT, RIGHT, 100, 24);
        a.update(Action::CycleWhitespace);
        a.update(Action::ToggleCase);
        let t = draw(&a);
        let buf = t.backend().buffer();
        let text = t.backend().to_string();
        let row_of = |needle: &str| {
            u16::try_from(text.lines().position(|l| l.contains(needle)).unwrap()).unwrap()
        };
        // Values end one column before the right edge.
        let last = |y: u16| &buf[(98, y)];
        let ws = row_of("whitespace");
        assert!(text.lines().nth(ws.into()).unwrap().contains("trailing"));
        assert!(last(ws).modifier.contains(Modifier::BOLD), "on → bold");
        let blank = row_of("ignore blank lines");
        assert!(text.lines().nth(blank.into()).unwrap().contains("off"));
        assert!(
            !last(blank).modifier.contains(Modifier::BOLD),
            "off → plain"
        );
        assert!(
            text.lines()
                .nth(row_of("ignore case").into())
                .unwrap()
                .contains(" on ")
        );
    }

    #[test]
    fn short_sidebar_ends_with_ellipsis() {
        let s = screen(&app(LEFT, RIGHT, 100, 8));
        let last_line = s.lines().last().unwrap();
        assert!(last_line.contains("│ …"), "{s}");
    }

    #[test]
    fn help_lists_only_this_screens_commands() {
        let mut a = app(LEFT, RIGHT, 80, 30);
        help(&mut a);
        let s = screen(&a);
        assert!(s.contains("Tab Ctrl-w w"), "{s}");
        assert!(s.contains(":sidebar [left|right]"), "{s}");
        assert!(!s.contains("open file diff"), "folder-only command: {s}");
    }

    #[test]
    fn status_bar_truncates_left_half_first() {
        let s = screen(&app(LEFT, RIGHT, 50, 6));
        let status = s.lines().last().unwrap();
        assert!(
            status.contains("-1 +1 ~1  2 changes  F1:keys :help q:quit"),
            "{status}"
        );
    }

    #[test]
    fn status_after_toggles_and_jump() {
        let mut a = app(LEFT, RIGHT, 80, 9);
        a.update(Action::CycleWhitespace);
        a.update(Action::ToggleCase);
        a.update(Action::NextChange);
        let s = screen(&a);
        let status = s.lines().last().unwrap();
        assert!(status.contains("[w]s:trailing [i]case:on"), "{status}");
        assert!(status.contains("change 1/2"), "{status}");
    }

    #[test]
    fn tiny_terminal_does_not_panic() {
        for (w, h) in [(1, 1), (5, 3), (12, 4), (94, 1), (94, 2), (94, 3)] {
            let mut a = app(LEFT, RIGHT, w, h);
            screen(&a);
            help(&mut a);
            screen(&a);
        }
    }

    #[test]
    fn colors_follow_row_kinds() {
        let a = app("keep\ngone\n", "keep\nnew stuff here\n", 40, 6);
        let t = draw(&a);
        let buf = t.backend().buffer();
        // Row 2 of the body (y = 2): removed "gone" on the left, added text on the right.
        let fg_at = |x: u16, y: u16| buf[(x, y)].fg;
        assert_eq!(fg_at(5, 2), Color::Red, "removed text is red");
        assert_eq!(fg_at(5, 1), Color::Reset, "equal text is default");
        let added_x = (0..40).find(|&x| buf[(x, 3)].symbol() == "n").unwrap();
        assert_eq!(fg_at(added_x, 3), Color::Green, "added text is green");
    }

    #[test]
    fn modified_rows_are_yellow_with_reversed_changes() {
        let a = app("the quick fox\n", "the slow fox\n", 60, 5);
        let t = draw(&a);
        let buf = t.backend().buffer();
        let x_of = |sym: &str| (0..60).find(|&x| buf[(x, 1)].symbol() == sym).unwrap();
        let t_cell = &buf[(x_of("t"), 1)];
        assert_eq!(t_cell.fg, Color::Yellow);
        assert!(!t_cell.modifier.contains(Modifier::REVERSED));
        let q_cell = &buf[(x_of("q"), 1)];
        assert!(
            q_cell.modifier.contains(Modifier::REVERSED),
            "changed word is highlighted"
        );
    }

    #[test]
    fn long_labels_keep_their_tail() {
        assert_eq!(tail("a/very/long/path/file.json", 11), "…/file.json");
        assert_eq!(tail("short", 12), "short");
        let a = App::new(TextSession {
            left_label: "some/deep/dir/left.json (json, canonical)".into(),
            right_label: "some/deep/dir/right.json (json, canonical)".into(),
            left: String::new(),
            right: String::new(),
            options: DiffOptions::default(),
        })
        .unwrap();
        let header = screen(&a).lines().next().unwrap().to_owned();
        assert!(header.contains("canonical)"), "{header}");
    }

    fn type_line(a: &mut App, open: Action, text: &str, submit: bool) {
        a.update(open);
        for c in text.chars() {
            a.update(Action::Prompt(crate::cmdline::PromptKey::Char(c)));
        }
        if submit {
            a.update(Action::Prompt(crate::cmdline::PromptKey::Submit));
        }
    }

    fn help(a: &mut App) {
        type_line(a, Action::StartCommand, "help", true);
    }

    fn search(a: &mut App, pattern: &str) {
        type_line(
            a,
            Action::StartSearch(crate::search::Direction::Forward),
            pattern,
            true,
        );
    }

    /// Columns of row `y` whose symbol starts `needle`.
    fn cols_of(t: &Terminal<TestBackend>, y: u16, needle: &str) -> Vec<u16> {
        let line = t
            .backend()
            .to_string()
            .lines()
            .nth(y.into())
            .unwrap()
            .trim_matches('"')
            .to_owned();
        let mut out = Vec::new();
        let mut from = 0;
        while let Some(i) = line[from..].find(needle) {
            out.push(u16::try_from(width(&line[..from + i])).unwrap());
            from += i + needle.len();
        }
        out
    }

    #[test]
    fn search_highlights_only_the_active_pane() {
        let mut a = app("one apple\nkeep\n", "two apple\nkeep\n", 60, 6);
        search(&mut a, "apple");
        let t = draw(&a);
        let buf = t.backend().buffer();
        let [left, right] = cols_of(&t, 1, "apple")[..] else {
            panic!("apple on both sides")
        };
        assert_eq!(buf[(left, 1)].bg, Color::Magenta, "current match");
        assert!(buf[(left, 1)].modifier.contains(Modifier::UNDERLINED));
        assert_ne!(buf[(right, 1)].bg, Color::Magenta);
        assert_ne!(
            buf[(right, 1)].bg,
            Color::Cyan,
            "right pane is not searched"
        );

        a.update(Action::SwitchPane);
        let t = draw(&a);
        let buf = t.backend().buffer();
        assert_eq!(
            buf[(right, 1)].bg,
            Color::Cyan,
            "other match, not the current one"
        );
        assert_ne!(buf[(left, 1)].bg, Color::Cyan);
    }

    #[test]
    fn every_match_is_highlighted_and_noh_clears_them() {
        let mut a = app("ab ab ab\n", "x\n", 60, 5);
        search(&mut a, "ab");
        let t = draw(&a);
        let buf = t.backend().buffer();
        let cols = cols_of(&t, 1, "ab");
        assert_eq!(cols.len(), 3);
        assert_eq!(buf[(cols[0], 1)].bg, Color::Magenta);
        assert_eq!(buf[(cols[1], 1)].bg, Color::Cyan);
        assert_eq!(buf[(cols[2] + 1, 1)].bg, Color::Cyan);
        assert_eq!(
            buf[(cols[0] + 2, 1)].bg,
            Color::Reset,
            "space between matches"
        );
        type_line(&mut a, Action::StartCommand, "noh", true);
        let t = draw(&a);
        let buf = t.backend().buffer();
        assert!(cols.iter().all(|&x| buf[(x, 1)].bg == Color::Reset));
    }

    #[test]
    fn search_marks_win_over_changed_words() {
        let mut a = app("the quick fox\n", "the slow fox\n", 60, 5);
        search(&mut a, "uic");
        let t = draw(&a);
        let buf = t.backend().buffer();
        let q = cols_of(&t, 1, "quick")[0];
        assert!(
            buf[(q, 1)].modifier.contains(Modifier::REVERSED),
            "q still a changed word"
        );
        assert_eq!(buf[(q + 1, 1)].bg, Color::Magenta, "u is the match");
    }

    #[test]
    fn header_marks_the_active_pane() {
        let mut a = app(LEFT, RIGHT, 60, 6);
        let header = |a: &App| {
            screen(a)
                .lines()
                .next()
                .unwrap()
                .trim_matches('"')
                .to_owned()
        };
        assert!(header(&a).starts_with("▶left.txt"), "{}", header(&a));
        assert!(header(&a).contains(" right.txt"));
        a.update(Action::FocusPane(Pane::Right));
        assert!(header(&a).contains("▶right.txt"), "{}", header(&a));
        assert!(header(&a).starts_with(" left.txt"));
        a.update(Action::ToggleLayout);
        assert!(
            header(&a).starts_with(" left.txt →▶right.txt"),
            "{}",
            header(&a)
        );
    }

    #[test]
    fn inline_equal_rows_show_the_active_panes_text() {
        let mut a = app("a  b\n", "a b\n", 60, 5);
        a.update(Action::CycleWhitespace);
        a.update(Action::CycleWhitespace); // change: rows compare equal
        a.update(Action::ToggleLayout);
        assert!(a.diff.is_identical());
        let body = |a: &App| screen(a).lines().nth(1).unwrap().to_owned();
        assert!(body(&a).contains("a  b"), "{}", body(&a));
        a.update(Action::SwitchPane);
        assert!(body(&a).contains("a b "), "{}", body(&a));
    }

    #[test]
    fn prompt_replaces_the_status_bar_with_a_cursor() {
        let mut a = app(LEFT, RIGHT, 60, 8);
        type_line(
            &mut a,
            Action::StartSearch(crate::search::Direction::Backward),
            r"del\c",
            false,
        );
        let mut t = draw(&a);
        let status = t.backend().to_string().lines().last().unwrap().to_owned();
        assert_eq!(status.trim_matches('"').trim_end(), r"?del\c");
        assert_eq!(
            t.get_cursor_position().unwrap(),
            ratatui::layout::Position { x: 6, y: 7 }
        );
        assert!(
            !t.backend().buffer()[(0, 7)]
                .modifier
                .contains(Modifier::REVERSED),
            "plain, like vim's command line"
        );
        insta::assert_snapshot!(screen(&a));
    }

    #[test]
    fn long_prompt_keeps_its_end() {
        let mut a = app(LEFT, RIGHT, 20, 6);
        type_line(&mut a, Action::StartCommand, &"x".repeat(40), false);
        let status = screen(&a)
            .lines()
            .last()
            .unwrap()
            .trim_matches('"')
            .to_owned();
        assert!(
            status.starts_with('…') && status.trim_end().ends_with('x'),
            "{status}"
        );
    }

    #[test]
    fn search_result_and_errors_show_in_the_status_bar() {
        let mut a = app(LEFT, RIGHT, 80, 8);
        search(&mut a, "delta");
        let s = screen(&a);
        assert!(s.lines().last().unwrap().contains("/delta  [1/1]"), "{s}");
        search(&mut a, "nope");
        let t = draw(&a);
        let s = t.backend().to_string();
        assert!(
            s.lines()
                .last()
                .unwrap()
                .contains("E486: Pattern not found: nope "),
            "{s}"
        );
        assert_eq!(
            t.backend().buffer()[(2, 7)].bg,
            Color::Red,
            "errors are red"
        );
    }

    #[test]
    fn other_pane_hint_fits_next_to_the_sidebar() {
        let mut a = app(LEFT, "zeta-gamma\n", 100, 12);
        search(&mut a, "zeta-gamma");
        let s = screen(&a);
        assert!(
            s.lines()
                .last()
                .unwrap()
                .contains("E486: Pattern not found: zeta-gamma (in right pane: Tab)"),
            "{s}"
        );
    }

    #[test]
    fn far_right_match_is_visible_after_the_jump() {
        let long = format!("{}needle\n", "x".repeat(200));
        let mut a = app(&long, &long, 80, 6);
        search(&mut a, "needle");
        let t = draw(&a);
        let [left, right] = cols_of(&t, 1, "needle")[..] else {
            panic!("panes scroll together: {}", t.backend())
        };
        assert_eq!(t.backend().buffer()[(left, 1)].bg, Color::Magenta);
        assert_eq!(t.backend().buffer()[(right, 1)].bg, Color::Reset);
    }

    #[test]
    fn help_shows_the_search_legend_on_text_screens_only() {
        let mut a = app(LEFT, RIGHT, 100, 40);
        help(&mut a);
        let s = screen(&a);
        assert!(s.contains("current match"), "{s}");
        assert!(s.contains(r"\c ignores case"), "{s}");
    }

    fn text_spans_plain(
        text: &str,
        changed: &[Range<usize>],
        base: Style,
        skip: usize,
        w: usize,
    ) -> Vec<Span<'static>> {
        text_spans(text, changed, &Marks::default(), base, skip, w)
    }

    #[test]
    fn text_spans_clip_scroll_and_pad() {
        let plain = |spans: Vec<Span>| {
            spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(
            plain(text_spans_plain("abcdef", &[], Style::new(), 0, 4)),
            "abcd"
        );
        assert_eq!(
            plain(text_spans_plain("abcdef", &[], Style::new(), 2, 3)),
            "cde"
        );
        assert_eq!(
            plain(text_spans_plain("ab", &[], Style::new(), 0, 4)),
            "ab  "
        );
        assert_eq!(
            plain(text_spans_plain("\tx", &[], Style::new(), 0, 6)),
            "    x "
        );
        assert_eq!(
            plain(text_spans_plain("日本", &[], Style::new(), 0, 3)),
            "日 "
        );
    }
}
