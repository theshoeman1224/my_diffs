//! Format-aware comparison of JSON, YAML and CSV.
//!
//! All formats are parsed into a [`serde_json::Value`] tree and compared
//! structurally:
//!
//! * **Objects** — key order is ignored; members are added, removed or recursed into.
//! * **Arrays** — elements are aligned with an LCS diff (an insertion does not
//!   shift every later element); replaced elements of the same container type are
//!   recursed into, others are reported as changed.
//! * **Strings** — compared through the text normalizer, so `whitespace`,
//!   `ignore_case` and `mask` apply to string values.
//! * **Numbers** — compared numerically (`1` == `1.0`).
//! * **CSV** — rows become objects keyed by header. With `key_columns` the table
//!   is an object keyed by `col=value` (rows matched by key); without, an array.
//!
//! [`canonical_text`] renders a tree with sorted keys and ignored paths removed,
//! so the line-based viewer can show structured files side by side.

pub mod path;

use serde::Serialize;
use serde_json::{Map, Value};
use similar::{Algorithm, DiffOp, capture_diff_slices};

use crate::normalize::LineNormalizer;
use crate::options::DiffOptions;
use crate::{Error, Result};
pub use path::{Path, PathPattern, Seg};

/// Supported structured formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Json,
    Yaml,
    /// Delimited table with a header row.
    Csv(u8),
}

impl Format {
    fn name(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
            Self::Csv(_) => "csv",
        }
    }
}

/// What happened at a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Added,
    Removed,
    Changed,
}

impl ChangeKind {
    pub fn marker(self) -> char {
        match self {
            Self::Added => '+',
            Self::Removed => '-',
            Self::Changed => '~',
        }
    }
}

/// One difference.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StructChange {
    /// Rendered path, e.g. `$.items[2].name`. Array indices refer to the left
    /// document for removals and to the right document otherwise.
    pub path: String,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub left: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub right: Option<Value>,
}

/// All differences between two documents, in document order.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct StructDiff {
    pub changes: Vec<StructChange>,
}

/// Counts per change kind.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct StructStats {
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
}

impl StructDiff {
    pub fn is_identical(&self) -> bool {
        self.changes.is_empty()
    }

    pub fn stats(&self) -> StructStats {
        let mut s = StructStats::default();
        for c in &self.changes {
            match c.kind {
                ChangeKind::Added => s.added += 1,
                ChangeKind::Removed => s.removed += 1,
                ChangeKind::Changed => s.changed += 1,
            }
        }
        s
    }

    /// `~ $.a: 1 → 2`, `- $.b: true`, `+ $.c[0]: "x"` per change.
    pub fn render_review(&self) -> String {
        let mut out = String::new();
        for c in &self.changes {
            out.push_str(&describe(c, 60));
            out.push('\n');
        }
        out
    }
}

/// One-line description of a change, values shortened to `max` characters.
pub fn describe(c: &StructChange, max: usize) -> String {
    let v = |x: &Option<Value>| x.as_ref().map(|x| short(x, max)).unwrap_or_default();
    match c.kind {
        ChangeKind::Added => format!("+ {}: {}", c.path, v(&c.right)),
        ChangeKind::Removed => format!("- {}: {}", c.path, v(&c.left)),
        ChangeKind::Changed => format!("~ {}: {} → {}", c.path, v(&c.left), v(&c.right)),
    }
}

/// Compact JSON, cut to `max` characters with `…`.
pub fn short(v: &Value, max: usize) -> String {
    let s = v.to_string();
    if s.chars().count() <= max {
        s
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Parse a document. `side` (`left`/`right`) is used in error messages.
pub fn parse(text: &str, format: Format, side: &'static str, opts: &DiffOptions) -> Result<Value> {
    let err = |message: String| Error::Parse {
        side,
        format: format.name(),
        message,
    };
    match format {
        Format::Json => serde_json::from_str(text).map_err(|e| err(e.to_string())),
        Format::Yaml => {
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_norway::from_str(text).map_err(|e| err(e.to_string()))
        }
        Format::Csv(delimiter) => parse_csv(text, delimiter, &opts.key_columns).map_err(err),
    }
}

fn parse_csv(text: &str, delimiter: u8, keys: &[String]) -> std::result::Result<Value, String> {
    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers: Vec<String> = rdr
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect();
    for k in keys {
        if !headers.contains(k) {
            return Err(format!(
                "key column `{k}` not in header ({})",
                headers.join(", ")
            ));
        }
    }
    let mut rows = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        let mut obj = Map::new();
        for (i, h) in headers.iter().enumerate() {
            obj.insert(
                h.clone(),
                Value::String(rec.get(i).unwrap_or("").to_owned()),
            );
        }
        rows.push(obj);
    }
    if keys.is_empty() {
        return Ok(Value::Array(rows.into_iter().map(Value::Object).collect()));
    }
    let mut table = Map::new();
    for row in rows {
        let base: Vec<String> = keys
            .iter()
            .map(|k| format!("{k}={}", row.get(k).and_then(Value::as_str).unwrap_or("")))
            .collect();
        let base = base.join(",");
        let mut key = base.clone();
        let mut n = 2;
        while table.contains_key(&key) {
            key = format!("{base}#{n}");
            n += 1;
        }
        table.insert(key, Value::Object(row));
    }
    Ok(Value::Object(table))
}

/// Parse both sides and compare.
pub fn diff_structured(
    left: &str,
    right: &str,
    format: Format,
    opts: &DiffOptions,
) -> Result<StructDiff> {
    let l = parse(left, format, "left", opts)?;
    let r = parse(right, format, "right", opts)?;
    diff_values(&l, &r, opts)
}

struct Ctx<'a> {
    norm: LineNormalizer,
    ignore: Vec<PathPattern>,
    out: &'a mut Vec<StructChange>,
}

impl Ctx<'_> {
    fn ignored(&self, p: &Path) -> bool {
        self.ignore.iter().any(|pat| pat.matches(p))
    }

    fn push(&mut self, path: &Path, kind: ChangeKind, left: Option<&Value>, right: Option<&Value>) {
        self.out.push(StructChange {
            path: path.to_string(),
            kind,
            left: left.cloned(),
            right: right.cloned(),
        });
    }

    fn scalar_eq(&self, a: &Value, b: &Value) -> bool {
        match (a, b) {
            (Value::String(x), Value::String(y)) => {
                self.norm.normalize(x).key == self.norm.normalize(y).key
            }
            (Value::Number(x), Value::Number(y)) => x == y || x.as_f64() == y.as_f64(),
            _ => a == b,
        }
    }

    /// Deep equality honoring options and ignored paths (used for array alignment).
    fn deep_eq(&self, path: &Path, a: &Value, b: &Value) -> bool {
        let mut scratch = Vec::new();
        let mut sub = Ctx {
            norm: self.norm.clone(),
            ignore: self.ignore.clone(),
            out: &mut scratch,
        };
        sub.walk(path, a, b);
        scratch.is_empty()
    }

    fn walk(&mut self, path: &Path, a: &Value, b: &Value) {
        if self.ignored(path) {
            return;
        }
        match (a, b) {
            (Value::Object(x), Value::Object(y)) => {
                let mut keys: Vec<&String> = x.keys().collect();
                keys.extend(y.keys().filter(|k| !x.contains_key(*k)));
                for k in keys {
                    let p = path.child(Seg::Key(k.clone()));
                    if self.ignored(&p) {
                        continue;
                    }
                    match (x.get(k), y.get(k)) {
                        (Some(va), Some(vb)) => self.walk(&p, va, vb),
                        (Some(va), None) => self.push(&p, ChangeKind::Removed, Some(va), None),
                        (None, Some(vb)) => self.push(&p, ChangeKind::Added, None, Some(vb)),
                        (None, None) => {}
                    }
                }
            }
            (Value::Array(x), Value::Array(y)) => self.walk_array(path, x, y),
            _ => {
                if !self.scalar_eq(a, b) {
                    self.push(path, ChangeKind::Changed, Some(a), Some(b));
                }
            }
        }
    }

    /// Class id per element: equal ids mean deep-equal elements.
    fn classify<'v>(
        &self,
        path: &Path,
        items: &'v [Value],
        classes: &mut Vec<&'v Value>,
    ) -> Vec<usize> {
        items
            .iter()
            .map(|v| {
                if let Some(i) = classes.iter().position(|c| self.deep_eq(path, c, v)) {
                    i
                } else {
                    classes.push(v);
                    classes.len() - 1
                }
            })
            .collect()
    }

    fn walk_array<'v>(&mut self, path: &Path, x: &'v [Value], y: &'v [Value]) {
        // Lists can be long; fall back to positional pairing beyond this size.
        let ops = if x.len() * y.len() <= 250_000 {
            let mut classes = Vec::new();
            let xs = self.classify(path, x, &mut classes);
            let ys = self.classify(path, y, &mut classes);
            capture_diff_slices(Algorithm::Myers, &xs, &ys)
        } else {
            let n = x.len().min(y.len());
            let mut ops = vec![DiffOp::Replace {
                old_index: 0,
                old_len: n,
                new_index: 0,
                new_len: n,
            }];
            if x.len() > n {
                ops.push(DiffOp::Delete {
                    old_index: n,
                    old_len: x.len() - n,
                    new_index: n,
                });
            } else if y.len() > n {
                ops.push(DiffOp::Insert {
                    old_index: n,
                    new_index: n,
                    new_len: y.len() - n,
                });
            }
            ops
        };
        for op in ops {
            let (_, o, n) = op.as_tag_tuple();
            match op {
                DiffOp::Equal { .. } => {}
                DiffOp::Delete { .. } => {
                    for i in o {
                        self.push(
                            &path.child(Seg::Index(i)),
                            ChangeKind::Removed,
                            Some(&x[i]),
                            None,
                        );
                    }
                }
                DiffOp::Insert { .. } => {
                    for j in n {
                        self.push(
                            &path.child(Seg::Index(j)),
                            ChangeKind::Added,
                            None,
                            Some(&y[j]),
                        );
                    }
                }
                DiffOp::Replace { .. } => {
                    let pairs = o.len().min(n.len());
                    for k in 0..pairs {
                        let (i, j) = (o.start + k, n.start + k);
                        // Report under the new index; recurse into same-shaped containers.
                        let p = path.child(Seg::Index(j));
                        if self.ignored(&p) {
                            continue;
                        }
                        let same_shape = matches!(
                            (&x[i], &y[j]),
                            (Value::Object(_), Value::Object(_))
                                | (Value::Array(_), Value::Array(_))
                        );
                        if same_shape {
                            self.walk(&p, &x[i], &y[j]);
                        } else if !self.scalar_eq(&x[i], &y[j]) {
                            self.push(&p, ChangeKind::Changed, Some(&x[i]), Some(&y[j]));
                        }
                    }
                    for (i, v) in x.iter().enumerate().take(o.end).skip(o.start + pairs) {
                        self.push(
                            &path.child(Seg::Index(i)),
                            ChangeKind::Removed,
                            Some(v),
                            None,
                        );
                    }
                    for (j, v) in y.iter().enumerate().take(n.end).skip(n.start + pairs) {
                        self.push(&path.child(Seg::Index(j)), ChangeKind::Added, None, Some(v));
                    }
                }
            }
        }
    }
}

pub(crate) fn compile_patterns(opts: &DiffOptions) -> Result<Vec<PathPattern>> {
    opts.ignore_paths
        .iter()
        .map(|p| PathPattern::parse(p))
        .collect()
}

/// Compare two parsed trees.
pub fn diff_values(left: &Value, right: &Value, opts: &DiffOptions) -> Result<StructDiff> {
    let mut changes = Vec::new();
    let mut ctx = Ctx {
        norm: LineNormalizer::new(opts)?,
        ignore: compile_patterns(opts)?,
        out: &mut changes,
    };
    ctx.walk(&Path::default(), left, right);
    Ok(StructDiff { changes })
}

/// Pretty JSON with object keys sorted and ignored paths removed — a stable
/// text form for line-based viewing.
pub fn canonical_text(v: &Value, opts: &DiffOptions) -> Result<String> {
    fn strip(v: &Value, path: &Path, ignore: &[PathPattern]) -> Value {
        match v {
            Value::Object(m) => {
                let mut keys: Vec<&String> = m.keys().collect();
                keys.sort();
                let mut out = Map::new();
                for k in keys {
                    let p = path.child(Seg::Key(k.clone()));
                    if !ignore.iter().any(|pat| pat.matches(&p)) {
                        out.insert(k.clone(), strip(&m[k], &p, ignore));
                    }
                }
                Value::Object(out)
            }
            Value::Array(a) => Value::Array(
                a.iter()
                    .enumerate()
                    .filter_map(|(i, x)| {
                        let p = path.child(Seg::Index(i));
                        (!ignore.iter().any(|pat| pat.matches(&p))).then(|| strip(x, &p, ignore))
                    })
                    .collect(),
            ),
            // Integral floats print as integers so `20.0` and `20` look equal, as they compare.
            Value::Number(n) => match n.as_f64() {
                Some(f) if n.is_f64() && f.fract() == 0.0 && f.abs() < 9_007_199_254_740_992.0 => {
                    Value::from(f as i64)
                }
                _ => v.clone(),
            },
            other => other.clone(),
        }
    }
    let ignore = compile_patterns(opts)?;
    let mut s = serde_json::to_string_pretty(&strip(v, &Path::default(), &ignore))
        .map_err(|e| Error::Config(e.to_string()))?;
    s.push('\n');
    Ok(s)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::options::{MaskRule, WhitespaceMode};

    fn review(a: &Value, b: &Value, opts: &DiffOptions) -> String {
        diff_values(a, b, opts).unwrap().render_review()
    }

    #[test]
    fn identical_ignoring_key_order() {
        let d = diff_values(
            &json!({"a": 1, "b": [1, 2]}),
            &json!({"b": [1, 2], "a": 1}),
            &DiffOptions::default(),
        );
        assert!(d.unwrap().is_identical());
    }

    #[test]
    fn objects_report_added_removed_changed() {
        let a = json!({"name": "x", "old": true, "n": {"deep": 1}});
        let b = json!({"name": "y", "new": [1], "n": {"deep": 2}});
        insta::assert_snapshot!(review(&a, &b, &DiffOptions::default()), @r#"
        ~ $.name: "x" → "y"
        - $.old: true
        ~ $.n.deep: 1 → 2
        + $.new: [1]
        "#);
    }

    #[test]
    fn array_insert_does_not_cascade() {
        let a = json!(["a", "b", "c"]);
        let b = json!(["a", "x", "b", "c"]);
        assert_eq!(review(&a, &b, &DiffOptions::default()), "+ $[1]: \"x\"\n");
    }

    #[test]
    fn replaced_objects_in_arrays_are_recursed() {
        let a = json!([{"id": 1, "v": "a"}]);
        let b = json!([{"id": 1, "v": "b"}]);
        assert_eq!(
            review(&a, &b, &DiffOptions::default()),
            "~ $[0].v: \"a\" → \"b\"\n"
        );
    }

    #[test]
    fn numbers_compare_numerically() {
        assert!(
            diff_values(&json!(1), &json!(1.0), &DiffOptions::default())
                .unwrap()
                .is_identical()
        );
    }

    #[test]
    fn type_change_is_changed() {
        assert_eq!(
            review(
                &json!({"a": 1}),
                &json!({"a": "1"}),
                &DiffOptions::default()
            ),
            "~ $.a: 1 → \"1\"\n"
        );
    }

    #[test]
    fn ignore_paths_skip_subtrees_and_wildcards() {
        let opts = DiffOptions {
            ignore_paths: vec!["$.meta".into(), "$.items[*].ts".into()],
            ..DiffOptions::default()
        };
        let a = json!({"meta": {"t": 1}, "items": [{"ts": 1, "v": 1}]});
        let b = json!({"meta": {"t": 2}, "items": [{"ts": 2, "v": 1}]});
        assert!(diff_values(&a, &b, &opts).unwrap().is_identical());
    }

    #[test]
    fn string_values_honor_text_options() {
        let opts = DiffOptions {
            whitespace: WhitespaceMode::Change,
            ignore_case: true,
            mask: vec![MaskRule {
                pattern: r"\d{4}-\d{2}-\d{2}".into(),
                replace: "D".into(),
            }],
            ..DiffOptions::default()
        };
        let a = json!({"s": "Hello  World", "when": "2024-01-01"});
        let b = json!({"s": "hello world", "when": "2026-11-13"});
        assert!(diff_values(&a, &b, &opts).unwrap().is_identical());
    }

    #[test]
    fn csv_by_position_and_by_key() {
        let a = "id,name,price\n1,apple,1.00\n2,pear,2.00\n";
        let b = "id,name,price\n0,fig,9.00\n2,pear,2.50\n1,apple,1.00\n";
        let by_pos = diff_structured(a, b, Format::Csv(b','), &DiffOptions::default()).unwrap();
        insta::assert_snapshot!(by_pos.render_review(), @r#"
        + $[0]: {"id":"0","name":"fig","price":"9.00"}
        + $[1]: {"id":"2","name":"pear","price":"2.50"}
        - $[1]: {"id":"2","name":"pear","price":"2.00"}
        "#);
        let opts = DiffOptions {
            key_columns: vec!["id".into()],
            ..DiffOptions::default()
        };
        let by_key = diff_structured(a, b, Format::Csv(b','), &opts).unwrap();
        insta::assert_snapshot!(by_key.render_review(), @r#"
        ~ $["id=2"].price: "2.00" → "2.50"
        + $["id=0"]: {"id":"0","name":"fig","price":"9.00"}
        "#);
    }

    #[test]
    fn csv_unknown_key_column_is_an_error() {
        let opts = DiffOptions {
            key_columns: vec!["sku".into()],
            ..DiffOptions::default()
        };
        let err = diff_structured("id\n1\n", "id\n1\n", Format::Csv(b','), &opts).unwrap_err();
        assert!(
            err.to_string().contains("key column `sku` not in header"),
            "{err}"
        );
    }

    #[test]
    fn csv_duplicate_keys_are_disambiguated() {
        let opts = DiffOptions {
            key_columns: vec!["id".into()],
            ..DiffOptions::default()
        };
        let v = parse("id,v\n1,a\n1,b\n", Format::Csv(b','), "left", &opts).unwrap();
        assert!(v.get("id=1").is_some() && v.get("id=1#2").is_some());
    }

    #[test]
    fn yaml_parses_into_the_same_tree() {
        let y = parse(
            "a: 1\nlist:\n  - x\n",
            Format::Yaml,
            "left",
            &DiffOptions::default(),
        )
        .unwrap();
        assert_eq!(y, json!({"a": 1, "list": ["x"]}));
        assert_eq!(
            parse("", Format::Yaml, "left", &DiffOptions::default()).unwrap(),
            Value::Null
        );
    }

    #[test]
    fn parse_errors_name_side_and_format() {
        let err = diff_structured("{}", "{", Format::Json, &DiffOptions::default()).unwrap_err();
        assert!(
            err.to_string()
                .starts_with("could not parse right input as json"),
            "{err}"
        );
    }

    #[test]
    fn canonical_text_sorts_and_strips() {
        let opts = DiffOptions {
            ignore_paths: vec!["$.secret".into()],
            ..DiffOptions::default()
        };
        let t = canonical_text(&json!({"b": 1, "a": [true], "secret": "x"}), &opts).unwrap();
        assert_eq!(t, "{\n  \"a\": [\n    true\n  ],\n  \"b\": 1\n}\n");
    }

    #[test]
    fn canonical_text_prints_integral_floats_as_integers() {
        let t = canonical_text(&json!([20.0, 1.5, 3]), &DiffOptions::default()).unwrap();
        assert_eq!(t, "[\n  20,\n  1.5,\n  3\n]\n");
    }

    #[test]
    fn short_truncates() {
        assert_eq!(short(&json!("abcdef"), 5), "\"abc…");
        assert_eq!(short(&json!(1), 5), "1");
    }
}
