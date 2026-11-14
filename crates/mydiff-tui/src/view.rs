//! Pure rendering: `render(&App, &mut Frame)`. Uses only the 16 basic colors so it
//! looks the same in xterm, `PuTTY` and Windows Terminal.

use std::ops::Range;

use mydiff_core::display::{sanitize, segments, width};
use mydiff_core::{LineRef, Row, RowKind};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::{App, Layout, LineSide};
use crate::keys::HELP;

/// Width of a line-number column including its trailing space.
const NUM_W: usize = 5;
/// Width of the side-by-side gutter (" ~ ").
const GUTTER_W: usize = 3;

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

pub fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

pub fn bar() -> Style {
    Style::new().add_modifier(Modifier::REVERSED)
}

pub fn render(app: &App, frame: &mut Frame) {
    let [header, body, status] = Split::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    render_header(app, frame, header);
    render_body(app, frame, body);
    render_status(app, frame, status);
    if app.show_help {
        render_help(frame);
    }
}

fn render_header(app: &App, frame: &mut Frame, area: Rect) {
    let s = &app.session;
    let w = usize::from(area.width);
    let text = match app.layout {
        Layout::SideBySide => {
            let pane = w.saturating_sub(GUTTER_W) / 2;
            format!(
                " {}{} {}",
                pad(
                    &tail(&s.left_label, pane.saturating_sub(1)),
                    pane.saturating_sub(1)
                ),
                " ".repeat(GUTTER_W),
                tail(&s.right_label, pane.saturating_sub(1))
            )
        }
        Layout::Inline => pair_title("", &s.left_label, &s.right_label, w),
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
        .map(|dl| {
            let row = &app.diff.rows[dl.row];
            match app.layout {
                Layout::SideBySide => side_by_side_line(row, w, app.hscroll),
                Layout::Inline => inline_line(row, dl.side, w, app.hscroll),
            }
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn side_by_side_line(row: &Row, w: usize, hscroll: usize) -> Line<'static> {
    let pane = w.saturating_sub(GUTTER_W) / 2;
    let text_w = pane.saturating_sub(NUM_W);
    let style = row_style(row.kind);
    let mut spans = Vec::new();
    spans.extend(numbered(row.left.as_ref(), style, text_w, hscroll));
    spans.push(Span::styled(format!(" {} ", row.kind.marker()), style));
    spans.extend(numbered(row.right.as_ref(), style, text_w, hscroll));
    Line::from(spans)
}

fn numbered(
    line: Option<&LineRef>,
    style: Style,
    text_w: usize,
    hscroll: usize,
) -> Vec<Span<'static>> {
    match line {
        Some(l) => {
            let mut v = vec![Span::styled(format!("{:>4} ", l.number), dim())];
            v.extend(text_spans(&l.text, &l.changed, style, hscroll, text_w));
            v
        }
        None => vec![Span::raw(" ".repeat(NUM_W + text_w))],
    }
}

fn inline_line(row: &Row, side: LineSide, w: usize, hscroll: usize) -> Line<'static> {
    let style = row_style(row.kind);
    let num =
        |l: Option<&LineRef>| l.map_or_else(|| "     ".to_owned(), |l| format!("{:>4} ", l.number));
    let (left_no, right_no, line) = match side {
        LineSide::Both => (
            row.left.as_ref(),
            row.right.as_ref(),
            row.left.as_ref().or(row.right.as_ref()),
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
        spans.extend(text_spans(&l.text, &l.changed, style, hscroll, text_w));
    }
    Line::from(spans)
}

/// Styled spans for one line's text: tabs expanded, `skip` columns hidden on the
/// left, clipped and padded to exactly `width` columns.
pub fn text_spans(
    text: &str,
    changed: &[Range<usize>],
    base: Style,
    skip: usize,
    width_cols: usize,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut col = 0; // display column in the expanded line
    let mut used = 0; // columns emitted
    for (seg, is_hot) in segments(text, changed) {
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
            out.push(Span::styled(piece, if is_hot { hot_style() } else { base }));
        }
    }
    if used < width_cols {
        out.push(Span::raw(" ".repeat(width_cols - used)));
    }
    out
}

fn render_status(app: &App, frame: &mut Frame, area: Rect) {
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
        "-{} +{} ~{}  {position}  ?:help q:quit ",
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

pub fn render_help(frame: &mut Frame) {
    let key_w = HELP.iter().map(|(k, _)| width(k)).max().unwrap_or(0);
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(k, d)| {
            Line::from(vec![
                Span::styled(pad(k, key_w + 2), Style::new().add_modifier(Modifier::BOLD)),
                Span::raw(*d),
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
        .collect();
    let area = frame.area();
    let h = (lines.len() as u16 + 2).min(area.height);
    let w = 72.min(area.width);
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
    use crate::app::{Action, TextSession};

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
        a.update(Action::ToggleHelp);
        insta::assert_snapshot!(screen(&a));
    }

    #[test]
    fn status_bar_truncates_left_half_first() {
        let s = screen(&app(LEFT, RIGHT, 50, 6));
        let status = s.lines().last().unwrap();
        assert!(
            status.contains("-1 +1 ~1  2 changes  ?:help q:quit"),
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
        for (w, h) in [(1, 1), (5, 3), (12, 4)] {
            let mut a = app(LEFT, RIGHT, w, h);
            screen(&a);
            a.update(Action::ToggleHelp);
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

    #[test]
    fn text_spans_clip_scroll_and_pad() {
        let plain = |spans: Vec<Span>| {
            spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        };
        assert_eq!(plain(text_spans("abcdef", &[], Style::new(), 0, 4)), "abcd");
        assert_eq!(plain(text_spans("abcdef", &[], Style::new(), 2, 3)), "cde");
        assert_eq!(plain(text_spans("ab", &[], Style::new(), 0, 4)), "ab  ");
        assert_eq!(plain(text_spans("\tx", &[], Style::new(), 0, 6)), "    x ");
        assert_eq!(plain(text_spans("日本", &[], Style::new(), 0, 3)), "日 ");
    }
}
