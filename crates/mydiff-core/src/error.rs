//! Error type shared by every core module.

use std::path::PathBuf;

/// Result alias used throughout `mydiff-core`.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong inside the core.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A user-supplied regular expression did not compile.
    #[error("invalid {kind} pattern `{pattern}`: {source}")]
    Pattern {
        /// Which option the pattern came from (`ignore`, `only`, `mask`).
        kind: &'static str,
        /// The offending pattern text.
        pattern: String,
        /// Underlying regex error.
        #[source]
        source: Box<regex::Error>,
    },

    /// A user-supplied glob did not compile.
    #[error("invalid glob `{pattern}`: {source}")]
    Glob {
        /// The offending glob text.
        pattern: String,
        /// Underlying globset error.
        #[source]
        source: Box<globset::Error>,
    },

    /// A configuration document could not be parsed.
    #[error("invalid configuration: {0}")]
    Config(String),

    /// A structured document (JSON/YAML/CSV) could not be parsed.
    #[error("could not parse {side} input as {format}: {message}")]
    Parse {
        /// `left` or `right`.
        side: &'static str,
        /// `json`, `yaml` or `csv`.
        format: &'static str,
        /// Parser message.
        message: String,
    },

    /// Filesystem access failed (folder mode and input loading only).
    #[error("{}: {source}", path.display())]
    Io {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    pub(crate) fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
