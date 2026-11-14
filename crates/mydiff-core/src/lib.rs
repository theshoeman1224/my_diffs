//! `mydiff-core`: the comparison engine behind `mydiff`.
//!
//! Everything here is deterministic and (outside of the folder and input-loading modules)
//! free of I/O: text or bytes in, a result model out. Front-ends (CLI, TUI) only
//! render these models.

pub mod binary;
pub mod config;
pub mod detect;
pub mod display;
pub mod error;
pub mod filter;
pub mod folder;
pub mod inline;
pub mod model;
pub mod normalize;
pub mod options;
pub mod structured;
pub mod text;

pub use error::{Error, Result};
pub use model::{LineRef, Row, RowKind, Stats, TextDiff};
pub use options::{DiffOptions, WhitespaceMode};
pub use text::diff_text;
