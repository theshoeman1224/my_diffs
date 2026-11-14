//! Machine-readable output. One JSON document per invocation.

use serde::Serialize;

/// Envelope shared by every mode: `{ "mode", "left", "right", "identical", ... }`.
#[derive(Debug, Serialize)]
pub struct Report<'a, T: Serialize> {
    pub mode: &'static str,
    pub left: &'a str,
    pub right: &'a str,
    pub identical: bool,
    #[serde(flatten)]
    pub body: T,
}

pub fn render<T: Serialize>(report: &Report<'_, T>) -> String {
    let mut s = serde_json::to_string_pretty(report).expect("report types always serialize");
    s.push('\n');
    s
}

#[cfg(test)]
mod tests {
    use mydiff_core::{DiffOptions, diff_text};

    use super::*;

    #[derive(Serialize)]
    struct TextBody<'a> {
        stats: mydiff_core::Stats,
        rows: &'a [mydiff_core::Row],
    }

    #[test]
    fn text_report_shape() {
        let d = diff_text("a\nb\n", "a\nc\n", &DiffOptions::default()).unwrap();
        let out = render(&Report {
            mode: "text",
            left: "l",
            right: "r",
            identical: d.is_identical(),
            body: TextBody {
                stats: d.stats(),
                rows: &d.rows,
            },
        });
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["mode"], "text");
        assert_eq!(v["identical"], false);
        assert_eq!(v["stats"]["equal"], 1);
        assert_eq!(v["rows"][1]["kind"], "removed");
        assert_eq!(v["rows"][1]["left"]["text"], "b");
    }
}
