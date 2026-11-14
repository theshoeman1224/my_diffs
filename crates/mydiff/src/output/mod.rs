//! Non-interactive renderers. Each takes a core result model and writes text.
//! Colors are optional so the same code produces plain text for tests and pipes.

pub mod folder;
pub mod hex;
pub mod json;
pub mod side_by_side;
pub mod structured;
pub mod summary;
pub mod unified;

use std::ops::Range;

use anstyle::{AnsiColor, Effects, Style};
use mydiff_core::RowKind;
pub use mydiff_core::display::{sanitize, segments, width};

/// Shared color scheme: red = removed, green = added, yellow = modified.
/// Uses only the 16 basic ANSI colors so it renders in xterm, `PuTTY` and
/// Windows Terminal alike.
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    /// Emit ANSI escapes?
    pub color: bool,
}

impl Theme {
    #[cfg(test)]
    pub const PLAIN: Self = Self { color: false };

    pub fn row(kind: RowKind) -> Style {
        match kind {
            RowKind::Equal => Style::new(),
            RowKind::Removed => AnsiColor::Red.on_default(),
            RowKind::Added => AnsiColor::Green.on_default(),
            RowKind::Modified => AnsiColor::Yellow.on_default(),
            RowKind::Ignored => Style::new().effects(Effects::DIMMED),
        }
    }

    /// Style for the changed part of a modified line.
    pub fn changed_span() -> Style {
        AnsiColor::Yellow
            .on_default()
            .effects(Effects::INVERT | Effects::BOLD)
    }

    pub fn header() -> Style {
        Style::new().effects(Effects::BOLD)
    }

    pub fn hunk() -> Style {
        AnsiColor::Cyan.on_default()
    }

    pub fn dim() -> Style {
        Style::new().effects(Effects::DIMMED)
    }

    /// `text` wrapped in `style` (or unchanged when color is off).
    pub fn paint(self, style: Style, text: &str) -> String {
        if self.color && style != Style::new() && !text.is_empty() {
            format!("{style}{text}{style:#}")
        } else {
            text.to_owned()
        }
    }

    /// Paint a line in `base`, emphasizing `changed` byte ranges.
    pub fn paint_line(self, base: Style, text: &str, changed: &[Range<usize>]) -> String {
        segments(text, changed)
            .into_iter()
            .map(|(seg, hot)| self.paint(if hot { Theme::changed_span() } else { base }, seg))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_theme_never_emits_escapes() {
        let t = Theme::PLAIN;
        assert_eq!(
            t.paint_line(Theme::row(RowKind::Modified), "a b", &[2..3]),
            "a b"
        );
    }

    #[test]
    fn color_theme_emphasizes_changed_span() {
        let t = Theme { color: true };
        let s = t.paint_line(Theme::row(RowKind::Modified), "a b", &[2..3]);
        assert!(s.contains("\u{1b}[") && s.contains('b'), "{s:?}");
    }
}
