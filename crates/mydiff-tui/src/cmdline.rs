//! The vim-style command line on the bottom row: `/` and `?` searches and
//! `:` commands, each with its own history.
//!
//! Editing keys while it is open (see `keys::prompt_action`):
//!
//! | Key | Effect |
//! |---|---|
//! | Enter | run it |
//! | Esc | cancel |
//! | Backspace / Ctrl-h | delete a character; on an empty line, cancel |
//! | Ctrl-w | delete the word before the cursor |
//! | Ctrl-u | delete everything typed |
//! | ↑ ↓ / Ctrl-p Ctrl-n | older / newer history entry starting with what is typed |

use crate::app::Dock;
use crate::search::Direction;

/// Most entries kept per history.
pub const HISTORY_LEN: usize = 100;

/// What the line is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    Search(Direction),
    Command,
}

impl PromptKind {
    /// The character shown before the input.
    pub fn leader(self) -> char {
        match self {
            Self::Search(dir) => dir.leader(),
            Self::Command => ':',
        }
    }
}

/// An editing key, sent as `Action::Prompt` while the line is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKey {
    Char(char),
    Backspace,
    DeleteWord,
    Clear,
    HistoryPrev,
    HistoryNext,
    Submit,
    Cancel,
}

/// The line being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub kind: PromptKind,
    pub input: String,
    /// What was typed before browsing history; filters it and comes back after the newest entry.
    typed: String,
    /// Index into the history while browsing it.
    browsing: Option<usize>,
}

/// What a key did to the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptEvent {
    /// The input changed (or a key did nothing).
    Edited,
    Cancelled,
    /// Enter: the line closed with this input.
    Submitted(PromptKind, String),
}

/// A status-bar message, shown until the next key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

impl Message {
    pub fn info(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: false,
        }
    }

    pub fn error(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            error: true,
        }
    }
}

#[derive(Debug, Default)]
pub struct CmdLine {
    pub prompt: Option<Prompt>,
    search_history: Vec<String>,
    command_history: Vec<String>,
}

impl CmdLine {
    pub fn open(&mut self, kind: PromptKind) {
        self.prompt = Some(Prompt {
            kind,
            input: String::new(),
            typed: String::new(),
            browsing: None,
        });
    }

    pub fn is_open(&self) -> bool {
        self.prompt.is_some()
    }

    /// Current input, or `""` when closed.
    pub fn input(&self) -> &str {
        self.prompt.as_ref().map_or("", |p| p.input.as_str())
    }

    /// Apply an editing key. Does nothing (`Edited`) when the line is closed.
    pub fn key(&mut self, key: PromptKey) -> PromptEvent {
        let Some(p) = &mut self.prompt else {
            return PromptEvent::Edited;
        };
        let history = match p.kind {
            PromptKind::Search(_) => &mut self.search_history,
            PromptKind::Command => &mut self.command_history,
        };
        match key {
            PromptKey::Char(c) => edit(p, |s| s.push(c)),
            PromptKey::Backspace if p.input.is_empty() => {
                self.prompt = None;
                return PromptEvent::Cancelled;
            }
            PromptKey::Backspace => edit(p, |s| {
                s.pop();
            }),
            PromptKey::DeleteWord => edit(p, delete_word),
            PromptKey::Clear => edit(p, String::clear),
            PromptKey::HistoryPrev => browse(p, history, true),
            PromptKey::HistoryNext => browse(p, history, false),
            PromptKey::Cancel => {
                self.prompt = None;
                return PromptEvent::Cancelled;
            }
            PromptKey::Submit => {
                let p = self.prompt.take().expect("checked above");
                if !p.input.is_empty() {
                    history.retain(|h| *h != p.input);
                    history.push(p.input.clone());
                    if history.len() > HISTORY_LEN {
                        history.remove(0);
                    }
                }
                return PromptEvent::Submitted(p.kind, p.input);
            }
        }
        PromptEvent::Edited
    }
}

/// Typing ends history browsing; the new text becomes the history filter.
fn edit(p: &mut Prompt, f: impl FnOnce(&mut String)) {
    f(&mut p.input);
    p.typed.clone_from(&p.input);
    p.browsing = None;
}

/// Delete trailing spaces, then the word (or run of punctuation) before them.
fn delete_word(s: &mut String) {
    let trimmed = s.trim_end().len();
    s.truncate(trimmed);
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let Some(last) = s.chars().last() else {
        return;
    };
    let keep = s
        .char_indices()
        .rev()
        .find(|&(_, c)| c.is_whitespace() || is_word(c) != is_word(last))
        .map_or(0, |(i, c)| i + c.len_utf8());
    s.truncate(keep);
}

/// Step to the next older (`back`) or newer history entry that starts with
/// what was typed. Past the newest entry the typed text comes back.
fn browse(p: &mut Prompt, history: &[String], back: bool) {
    let fits = |i: &usize| history[*i].starts_with(&p.typed);
    let next = if back {
        let end = p.browsing.unwrap_or(history.len());
        (0..end).rev().find(fits)
    } else {
        let Some(from) = p.browsing else {
            return;
        };
        (from + 1..history.len()).find(fits)
    };
    match next {
        Some(i) => {
            p.browsing = Some(i);
            p.input.clone_from(&history[i]);
        }
        None if !back => {
            p.browsing = None;
            p.input.clone_from(&p.typed);
        }
        None => {}
    }
}

/// A `:` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// `:h[elp]`: open the full key reference.
    Help,
    /// `:sideb[ar]`: show or hide the sidebar (like F1);
    /// `:sidebar left|right`: open it on that edge.
    Sidebar(Option<Dock>),
    /// `:q[uit][!]`: close this view (a file opened from a folder returns to the list).
    Quit,
    /// `:qa[ll][!]`, `:quita[ll][!]`: quit the program from anywhere.
    QuitAll,
    /// `:noh[lsearch]`: hide search highlights until the next search.
    NoHighlight,
}

/// Every `:` command: (shortest abbreviation, full name, command).
pub const COMMAND_NAMES: &[(&str, &str, Command)] = &[
    ("h", "help", Command::Help),
    ("q", "quit", Command::Quit),
    ("qa", "qall", Command::QuitAll),
    ("quita", "quitall", Command::QuitAll),
    ("noh", "nohlsearch", Command::NoHighlight),
    ("sideb", "sidebar", Command::Sidebar(None)),
];

/// Parse a `:` line like vim does: any abbreviation from the shortest form up
/// to the full name, an optional `!` on quits. `:help` ignores an argument;
/// `:sidebar` takes `left` or `right`. `Ok(None)` for an empty line.
pub fn parse_command(line: &str) -> Result<Option<Command>, String> {
    let line = line.trim().trim_start_matches(':').trim();
    if line.is_empty() {
        return Ok(None);
    }
    let name_end = line
        .find(|c: char| !c.is_ascii_alphabetic())
        .unwrap_or(line.len());
    let (name, rest) = line.split_at(name_end);
    let found = COMMAND_NAMES
        .iter()
        .find(|(short, full, _)| name.len() >= short.len() && full.starts_with(name))
        .map(|&(.., cmd)| cmd);
    let Some(cmd) = found else {
        return Err(format!("E492: Not an editor command: {line}"));
    };
    let rest = rest.trim();
    let ok = match cmd {
        Command::Help => true,
        Command::Quit | Command::QuitAll => rest.is_empty() || rest == "!",
        Command::NoHighlight => rest.is_empty(),
        Command::Sidebar(_) => {
            return match rest {
                "" => Ok(Some(cmd)),
                "left" => Ok(Some(Command::Sidebar(Some(Dock::Left)))),
                "right" => Ok(Some(Command::Sidebar(Some(Dock::Right)))),
                _ => Err(format!("E475: Invalid argument: {rest} (left or right)")),
            };
        }
    };
    if ok {
        Ok(Some(cmd))
    } else {
        Err(format!("E488: Trailing characters: {rest}"))
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn typed(kind: PromptKind, text: &str) -> CmdLine {
        let mut c = CmdLine::default();
        c.open(kind);
        for ch in text.chars() {
            c.key(PromptKey::Char(ch));
        }
        c
    }

    const SEARCH: PromptKind = PromptKind::Search(Direction::Forward);

    #[test]
    fn typing_and_submitting() {
        let mut c = typed(SEARCH, "foo");
        assert_eq!(c.input(), "foo");
        assert_eq!(
            c.key(PromptKey::Submit),
            PromptEvent::Submitted(SEARCH, "foo".into())
        );
        assert!(!c.is_open());
    }

    #[test]
    fn backspace_on_empty_line_cancels() {
        let mut c = typed(SEARCH, "a");
        assert_eq!(c.key(PromptKey::Backspace), PromptEvent::Edited);
        assert!(c.is_open());
        assert_eq!(c.key(PromptKey::Backspace), PromptEvent::Cancelled);
        assert!(!c.is_open());
    }

    #[test]
    fn escape_cancels_without_history() {
        let mut c = typed(SEARCH, "abc");
        assert_eq!(c.key(PromptKey::Cancel), PromptEvent::Cancelled);
        c.open(SEARCH);
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "", "cancelled lines are not remembered");
    }

    #[rstest]
    #[case::word("foo bar", "foo ")]
    #[case::trailing_spaces("foo bar  ", "foo ")]
    #[case::punctuation_run("foo.*", "foo")]
    #[case::word_after_punctuation(r"\<word", r"\<")]
    #[case::only_word("foo", "")]
    #[case::empty("", "")]
    fn ctrl_w_deletes_a_word(#[case] before: &str, #[case] after: &str) {
        let mut s = before.to_owned();
        delete_word(&mut s);
        assert_eq!(s, after);
    }

    #[test]
    fn ctrl_u_clears() {
        let mut c = typed(PromptKind::Command, "help");
        c.key(PromptKey::Clear);
        assert_eq!(c.input(), "");
        assert!(c.is_open());
    }

    fn submit(c: &mut CmdLine, kind: PromptKind, text: &str) {
        c.open(kind);
        for ch in text.chars() {
            c.key(PromptKey::Char(ch));
        }
        c.key(PromptKey::Submit);
    }

    #[test]
    fn history_walks_back_and_forward_then_restores_typed_text() {
        let mut c = CmdLine::default();
        for s in ["alpha", "beta", "gamma"] {
            submit(&mut c, SEARCH, s);
        }
        c.open(SEARCH);
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "gamma");
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "beta");
        c.key(PromptKey::HistoryPrev);
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "alpha", "stops at the oldest");
        c.key(PromptKey::HistoryNext);
        c.key(PromptKey::HistoryNext);
        assert_eq!(c.input(), "gamma");
        c.key(PromptKey::HistoryNext);
        assert_eq!(c.input(), "", "back to what was typed");
    }

    #[test]
    fn history_is_filtered_by_typed_prefix() {
        let mut c = CmdLine::default();
        for s in ["foo1", "bar", "foo2"] {
            submit(&mut c, SEARCH, s);
        }
        let mut c2 = c;
        c2.open(SEARCH);
        c2.key(PromptKey::Char('f'));
        c2.key(PromptKey::HistoryPrev);
        assert_eq!(c2.input(), "foo2");
        c2.key(PromptKey::HistoryPrev);
        assert_eq!(c2.input(), "foo1");
        c2.key(PromptKey::HistoryNext);
        c2.key(PromptKey::HistoryNext);
        assert_eq!(c2.input(), "f");
    }

    #[test]
    fn history_drops_duplicates_and_keeps_kinds_apart() {
        let mut c = CmdLine::default();
        submit(&mut c, SEARCH, "x");
        submit(&mut c, SEARCH, "y");
        submit(&mut c, PromptKind::Search(Direction::Backward), "x");
        submit(&mut c, PromptKind::Command, "noh");
        c.open(SEARCH);
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "x", "? and / share one history");
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "y");
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "y", "x was moved to the end, not duplicated");
        c.open(PromptKind::Command);
        c.key(PromptKey::HistoryPrev);
        assert_eq!(c.input(), "noh");
    }

    #[test]
    fn history_is_capped() {
        let mut c = CmdLine::default();
        for i in 0..=HISTORY_LEN {
            submit(&mut c, SEARCH, &i.to_string());
        }
        assert_eq!(c.search_history.len(), HISTORY_LEN);
        assert_eq!(c.search_history[0], "1");
    }

    #[test]
    fn keys_on_a_closed_line_do_nothing() {
        let mut c = CmdLine::default();
        assert_eq!(c.key(PromptKey::Submit), PromptEvent::Edited);
        assert_eq!(c.input(), "");
    }

    #[rstest]
    #[case::help("help", Ok(Some(Command::Help)))]
    #[case::h("h", Ok(Some(Command::Help)))]
    #[case::help_with_topic("help search", Ok(Some(Command::Help)))]
    #[case::q("q", Ok(Some(Command::Quit)))]
    #[case::quit_bang("quit!", Ok(Some(Command::Quit)))]
    #[case::qa("qa", Ok(Some(Command::QuitAll)))]
    #[case::qall_bang("qall!", Ok(Some(Command::QuitAll)))]
    #[case::quitall("quitall", Ok(Some(Command::QuitAll)))]
    #[case::quita("quita", Ok(Some(Command::QuitAll)))]
    #[case::noh("noh", Ok(Some(Command::NoHighlight)))]
    #[case::nohlsearch("nohlsearch", Ok(Some(Command::NoHighlight)))]
    #[case::sidebar_toggles("sidebar", Ok(Some(Command::Sidebar(None))))]
    #[case::sidebar_short("sideb", Ok(Some(Command::Sidebar(None))))]
    #[case::sidebar_left("sidebar left", Ok(Some(Command::Sidebar(Some(Dock::Left)))))]
    #[case::sidebar_right("sideb  right", Ok(Some(Command::Sidebar(Some(Dock::Right)))))]
    #[case::sidebar_bad_side("sidebar up", Err("E475: Invalid argument: up (left or right)".into()))]
    #[case::side_is_too_short("side", Err("E492: Not an editor command: side".into()))]
    #[case::extra_colon_and_spaces(" :q ", Ok(Some(Command::Quit)))]
    #[case::empty("", Ok(None))]
    #[case::too_short("no", Err("E492: Not an editor command: no".into()))]
    #[case::unknown("wq", Err("E492: Not an editor command: wq".into()))]
    #[case::trailing("q now", Err("E488: Trailing characters: now".into()))]
    fn commands(#[case] line: &str, #[case] expected: Result<Option<Command>, String>) {
        assert_eq!(parse_command(line), expected);
    }

    #[test]
    fn leaders() {
        assert_eq!(PromptKind::Command.leader(), ':');
        assert_eq!(SEARCH.leader(), '/');
    }
}
