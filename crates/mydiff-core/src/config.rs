//! Config files and layered option resolution (pure: strings and tables in,
//! [`DiffOptions`] out; file discovery lives in the binary).
//!
//! ```toml
//! [defaults]                 # applied first
//! whitespace = "trailing"
//! exclude = ["target/**"]
//!
//! [profile.logs]             # applied on top when `--profile logs` is given
//! ignore = ['^DEBUG ']
//! ```
//!
//! Layers merge in order. Within a merge, scalars are replaced and **arrays are
//! appended**, so a profile adds ignore patterns to the defaults rather than
//! replacing them.

use std::collections::BTreeMap;

use serde::Deserialize;
use toml::{Table, Value};

use crate::options::DiffOptions;
use crate::{Error, Result};

/// A parsed configuration document.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigFile {
    /// Options applied whenever this file is loaded.
    #[serde(default)]
    pub defaults: Table,
    /// Named option overlays.
    #[serde(default)]
    pub profile: BTreeMap<String, Table>,
}

impl ConfigFile {
    /// Parse and validate a config document. Every table must contain only known
    /// option keys with valid types.
    pub fn parse(text: &str) -> Result<Self> {
        let file: Self = toml::from_str(text).map_err(|e| Error::Config(e.to_string()))?;
        validate(&file.defaults).map_err(|e| Error::Config(format!("[defaults]: {e}")))?;
        for (name, table) in &file.profile {
            validate(table).map_err(|e| Error::Config(format!("[profile.{name}]: {e}")))?;
        }
        Ok(file)
    }
}

fn validate(table: &Table) -> std::result::Result<(), String> {
    options_from_table(table.clone())
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn options_from_table(table: Table) -> Result<DiffOptions> {
    Value::Table(table)
        .try_into()
        .map_err(|e: toml::de::Error| Error::Config(e.message().to_owned()))
}

/// Merge `over` into `base`: scalars replace, arrays append, tables recurse.
pub fn merge_tables(base: &mut Table, over: &Table) {
    for (k, v) in over {
        match (base.get_mut(k), v) {
            (Some(Value::Array(a)), Value::Array(b)) => a.extend(b.iter().cloned()),
            (Some(Value::Table(a)), Value::Table(b)) => merge_tables(a, b),
            _ => {
                base.insert(k.clone(), v.clone());
            }
        }
    }
}

/// Resolve final options.
///
/// Order (later wins): built-in defaults → each file's `[defaults]` (in the
/// order given) → each file's `[profile.<name>]` → `overrides` (CLI flags).
pub fn resolve(
    files: &[ConfigFile],
    profile: Option<&str>,
    overrides: &Table,
) -> Result<DiffOptions> {
    let mut merged = Table::new();
    for f in files {
        merge_tables(&mut merged, &f.defaults);
    }
    if let Some(name) = profile {
        let mut found = false;
        for f in files {
            if let Some(p) = f.profile.get(name) {
                merge_tables(&mut merged, p);
                found = true;
            }
        }
        if !found {
            let known: Vec<&str> = files
                .iter()
                .flat_map(|f| f.profile.keys().map(String::as_str))
                .collect();
            return Err(Error::Config(format!(
                "profile `{name}` not found (known: {})",
                if known.is_empty() {
                    "none".to_owned()
                } else {
                    known.join(", ")
                }
            )));
        }
    }
    merge_tables(&mut merged, overrides);
    options_from_table(merged)
}

/// Commented starter config written by `mydiff config init`.
pub const SAMPLE_CONFIG: &str = r#"# mydiff configuration.
# Loaded from (later overrides earlier):
#   $XDG_CONFIG_HOME/mydiff/config.toml  ->  ./.mydiff.toml  ->  --config FILE  ->  CLI flags
# Scalars are replaced by later layers; lists are appended.

[defaults]
# whitespace = "exact"        # exact | trailing | change | all
# ignore_case = false
# ignore_blank_lines = false
# ignore_line_endings = true  # CRLF == LF
# context = 3                 # unified-output context lines
# modify_threshold = 0.5      # 0..1; how similar two lines must be to count as "modified"
# algorithm = "myers"         # myers | patience

# Lines matching any `ignore` regex are not compared.
# ignore = ['^\s*#']
# If set, ONLY lines matching one of these regexes are compared.
# only = []

# Replace text before comparing (displayed text is unchanged).
# [[defaults.mask]]
# pattern = '\d{4}-\d{2}-\d{2}T[\d:.]+Z?'
# replace = "<TIMESTAMP>"

# Folder mode.
# include = []
# exclude = [".git/**", "target/**"]

# Structured data.
# key_columns = ["id"]                  # CSV rows matched by these columns
# ignore_paths = ["$.metadata.updated"] # JSON/YAML paths to skip

# Select with: mydiff --profile logs a.log b.log
# [profile.logs]
# whitespace = "change"
# ignore = ['^DEBUG ']
# [[profile.logs.mask]]
# pattern = '^\S+ \S+'
# replace = "<TIME>"
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::WhitespaceMode;

    fn table(s: &str) -> Table {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn merge_replaces_scalars_appends_arrays_recurses_tables() {
        let mut base = table("a = 1\nlist = ['x']\n[t]\nk = 1\nkeep = true");
        merge_tables(&mut base, &table("a = 2\nlist = ['y']\n[t]\nk = 2"));
        assert_eq!(
            base,
            table("a = 2\nlist = ['x', 'y']\n[t]\nk = 2\nkeep = true")
        );
    }

    #[test]
    fn sample_config_parses_and_equals_defaults() {
        let f = ConfigFile::parse(SAMPLE_CONFIG).unwrap();
        assert_eq!(
            resolve(&[f], None, &Table::new()).unwrap(),
            DiffOptions::default()
        );
    }

    #[test]
    fn layering_order() {
        let user =
            ConfigFile::parse("[defaults]\nwhitespace = 'all'\nignore = ['^#']\ncontext = 5")
                .unwrap();
        let project = ConfigFile::parse(
            "[defaults]\nwhitespace = 'trailing'\n[profile.p]\nignore = ['^//']\nignore_case = true",
        )
        .unwrap();
        let cli = table("context = 1");
        let o = resolve(&[user, project], Some("p"), &cli).unwrap();
        assert_eq!(o.whitespace, WhitespaceMode::Trailing, "project beats user");
        assert_eq!(o.ignore, ["^#", "^//"], "lists append across layers");
        assert!(o.ignore_case, "profile applied");
        assert_eq!(o.context, 1, "cli beats files");
    }

    #[test]
    fn mask_tables_append() {
        let f = ConfigFile::parse(
            "[[defaults.mask]]\npattern = 'a'\n[[profile.p.mask]]\npattern = 'b'\nreplace = 'B'",
        )
        .unwrap();
        let o = resolve(&[f], Some("p"), &Table::new()).unwrap();
        assert_eq!(o.mask.len(), 2);
        assert_eq!(o.mask[1].replace, "B");
    }

    #[test]
    fn unknown_profile_lists_known_ones() {
        let f = ConfigFile::parse("[profile.logs]\n[profile.code]").unwrap();
        let err = resolve(&[f], Some("nope"), &Table::new())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("`nope` not found (known: code, logs)"),
            "{err}"
        );
    }

    #[test]
    fn unknown_option_key_reported_with_section() {
        let err = ConfigFile::parse("[profile.x]\nignore_whitespace = true")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("[profile.x]") && err.contains("ignore_whitespace"),
            "{err}"
        );
    }

    #[test]
    fn unknown_top_level_section_rejected() {
        assert!(ConfigFile::parse("[folder]\nexclude = []").is_err());
    }

    #[test]
    fn wrong_type_rejected() {
        let err = ConfigFile::parse("[defaults]\ncontext = 'three'")
            .unwrap_err()
            .to_string();
        assert!(err.contains("[defaults]"), "{err}");
    }
}
