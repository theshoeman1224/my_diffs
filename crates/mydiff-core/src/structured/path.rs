//! JSON-path-like addressing: concrete paths for reporting, and patterns for
//! `ignore_paths`.
//!
//! Pattern syntax (a small `JSONPath` subset):
//! `$` root · `.key` or `["key"]` object member · `[3]` array index ·
//! `[*]` any index · `.*` any member. A pattern matching a node also ignores
//! everything below it.

use std::fmt;

use crate::{Error, Result};

/// One step of a concrete path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Seg {
    Key(String),
    Index(usize),
}

/// A concrete location in a document.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Path(pub Vec<Seg>);

impl Path {
    #[must_use]
    pub fn child(&self, seg: Seg) -> Self {
        let mut v = self.0.clone();
        v.push(seg);
        Self(v)
    }
}

fn is_ident(k: &str) -> bool {
    let mut chars = k.chars();
    chars.next().is_some_and(|c| c.is_alphabetic() || c == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("$")?;
        for s in &self.0 {
            match s {
                Seg::Key(k) if is_ident(k) => write!(f, ".{k}")?,
                Seg::Key(k) => write!(f, "[{}]", serde_json::Value::String(k.clone()))?,
                Seg::Index(i) => write!(f, "[{i}]")?,
            }
        }
        Ok(())
    }
}

/// One step of a pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PatSeg {
    Key(String),
    AnyKey,
    Index(usize),
    AnyIndex,
}

/// A compiled `ignore_paths` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPattern(Vec<PatSeg>);

impl PathPattern {
    pub fn parse(src: &str) -> Result<Self> {
        let err = |why: &str| Error::Config(format!("invalid path pattern `{src}`: {why}"));
        let rest = src
            .strip_prefix('$')
            .ok_or_else(|| err("must start with `$`"))?;
        let chars: Vec<char> = rest.chars().collect();
        let mut i = 0;
        let mut segs = Vec::new();
        while i < chars.len() {
            match chars[i] {
                '.' => {
                    let start = i + 1;
                    let mut j = start;
                    while j < chars.len() && chars[j] != '.' && chars[j] != '[' {
                        j += 1;
                    }
                    let name: String = chars[start..j].iter().collect();
                    match name.as_str() {
                        "" => return Err(err("empty member name")),
                        "*" => segs.push(PatSeg::AnyKey),
                        _ => segs.push(PatSeg::Key(name)),
                    }
                    i = j;
                }
                '[' => {
                    let close = chars[i..]
                        .iter()
                        .position(|&c| c == ']')
                        .map(|p| p + i)
                        .ok_or_else(|| err("missing `]`"))?;
                    let inner: String = chars[i + 1..close].iter().collect();
                    let seg = if inner == "*" {
                        PatSeg::AnyIndex
                    } else if let Ok(n) = inner.parse::<usize>() {
                        PatSeg::Index(n)
                    } else if inner.starts_with('"') {
                        let key: String =
                            serde_json::from_str(&inner).map_err(|_| err("bad quoted key"))?;
                        PatSeg::Key(key)
                    } else {
                        return Err(err("expected an index, `*` or a quoted key inside `[]`"));
                    };
                    segs.push(seg);
                    i = close + 1;
                }
                c => return Err(err(&format!("unexpected `{c}`"))),
            }
        }
        Ok(Self(segs))
    }

    /// Does this pattern match `path` exactly?
    pub fn matches(&self, path: &Path) -> bool {
        self.0.len() == path.0.len()
            && self.0.iter().zip(&path.0).all(|(p, s)| match (p, s) {
                (PatSeg::Key(a), Seg::Key(b)) => a == b,
                (PatSeg::AnyKey, Seg::Key(_)) | (PatSeg::AnyIndex, Seg::Index(_)) => true,
                (PatSeg::Index(a), Seg::Index(b)) => a == b,
                _ => false,
            })
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn path(segs: &[Seg]) -> Path {
        Path(segs.to_vec())
    }

    fn k(s: &str) -> Seg {
        Seg::Key(s.into())
    }

    #[test]
    fn display() {
        assert_eq!(path(&[]).to_string(), "$");
        assert_eq!(
            path(&[k("a"), Seg::Index(2), k("b_c")]).to_string(),
            "$.a[2].b_c"
        );
        assert_eq!(
            path(&[k("id=3"), k("with space")]).to_string(),
            r#"$["id=3"]["with space"]"#
        );
    }

    #[rstest]
    #[case::exact("$.a.b", &[k("a"), k("b")], true)]
    #[case::too_short("$.a", &[k("a"), k("b")], false)]
    #[case::any_index("$.items[*].id", &[k("items"), Seg::Index(7), k("id")], true)]
    #[case::index("$.items[1]", &[k("items"), Seg::Index(2)], false)]
    #[case::any_key("$.*.ts", &[k("x"), k("ts")], true)]
    #[case::quoted(r#"$["id=3"].price"#, &[k("id=3"), k("price")], true)]
    #[case::key_vs_index("$.0", &[Seg::Index(0)], false)]
    fn matching(#[case] pat: &str, #[case] segs: &[Seg], #[case] expected: bool) {
        assert_eq!(
            PathPattern::parse(pat).unwrap().matches(&path(segs)),
            expected
        );
    }

    #[rstest]
    #[case::no_root("a.b")]
    #[case::empty_member("$..a")]
    #[case::unclosed("$.a[1")]
    #[case::bad_bracket("$[abc]")]
    fn invalid(#[case] pat: &str) {
        assert!(PathPattern::parse(pat).is_err());
    }
}
