//! Viewer state remembered between runs: whether the sidebar is open and which
//! edge it is docked on.
//!
//! It lives in `$XDG_STATE_HOME/mydiff/state.toml` (usually
//! `~/.local/state/mydiff/state.toml`), apart from the config file, which the
//! app never rewrites. A missing or unreadable file means the defaults
//! (open, on the right), and a failed save is ignored: remembering the sidebar
//! must never fail a comparison.
//!
//! ```toml
//! [sidebar]
//! open = false
//! dock = "left"
//! ```

use std::io;
use std::path::{Path, PathBuf};

use etcetera::BaseStrategy;
use mydiff_tui::{Dock, Sidebar};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateFile {
    #[serde(default)]
    sidebar: SidebarState,
}

/// Every field is optional so a partial or older file still loads.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SidebarState {
    open: Option<bool>,
    dock: Option<DockName>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum DockName {
    Left,
    Right,
}

/// `$XDG_STATE_HOME/mydiff/state.toml`.
pub fn state_path() -> Option<PathBuf> {
    etcetera::choose_base_strategy()
        .ok()?
        .state_dir()
        .map(|d| d.join("mydiff").join("state.toml"))
}

/// The sidebar described by a state file; defaults for anything missing or invalid.
fn parse(text: &str) -> Sidebar {
    let default = Sidebar::default();
    let Ok(file) = toml::from_str::<StateFile>(text) else {
        return default;
    };
    Sidebar {
        open: file.sidebar.open.unwrap_or(default.open),
        dock: match file.sidebar.dock {
            Some(DockName::Left) => Dock::Left,
            Some(DockName::Right) => Dock::Right,
            None => default.dock,
        },
    }
}

fn render(sidebar: Sidebar) -> String {
    let file = StateFile {
        sidebar: SidebarState {
            open: Some(sidebar.open),
            dock: Some(match sidebar.dock {
                Dock::Left => DockName::Left,
                Dock::Right => DockName::Right,
            }),
        },
    };
    toml::to_string(&file).expect("plain values serialize")
}

fn load(path: &Path) -> Sidebar {
    std::fs::read_to_string(path).map_or_else(|_| Sidebar::default(), |t| parse(&t))
}

/// Write via a temporary file and a rename, so a second viewer quitting at the
/// same moment cannot leave a half-written file.
fn save(path: &Path, sidebar: Sidebar) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    std::fs::write(&tmp, render(sidebar))?;
    std::fs::rename(&tmp, path)
}

/// Run a viewer starting with the remembered sidebar, then remember how the
/// user left it.
pub fn with_sidebar(run: impl FnOnce(Sidebar) -> io::Result<Sidebar>) -> io::Result<()> {
    with_sidebar_at(state_path().as_deref(), run)
}

/// [`with_sidebar`] with an explicit state file (`None`: nothing is remembered).
/// The file is only written when the state changed.
fn with_sidebar_at(
    path: Option<&Path>,
    run: impl FnOnce(Sidebar) -> io::Result<Sidebar>,
) -> io::Result<()> {
    let before = path.map(load).unwrap_or_default();
    let after = run(before)?;
    if let Some(path) = path.filter(|_| after != before) {
        // Best effort: a read-only home must not turn a diff into an error.
        let _ = save(path, after);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    const CLOSED_LEFT: Sidebar = Sidebar {
        open: false,
        dock: Dock::Left,
    };

    #[rstest]
    #[case::empty("", Sidebar::default())]
    #[case::full("[sidebar]\nopen = false\ndock = \"left\"\n", CLOSED_LEFT)]
    #[case::only_open("[sidebar]\nopen = false\n", Sidebar { open: false, dock: Dock::Right })]
    #[case::only_dock("[sidebar]\ndock = \"left\"\n", Sidebar { open: true, dock: Dock::Left })]
    #[case::unknown_keys_ignored("[sidebar]\nopen = false\nwidth = 3\n[other]\nx = 1\n", Sidebar { open: false, dock: Dock::Right })]
    #[case::bad_dock_means_defaults("[sidebar]\ndock = \"up\"\n", Sidebar::default())]
    #[case::not_toml("{{{", Sidebar::default())]
    fn parse_state_file(#[case] text: &str, #[case] expected: Sidebar) {
        assert_eq!(parse(text), expected);
    }

    #[test]
    fn render_round_trips() {
        for s in [Sidebar::default(), CLOSED_LEFT] {
            assert_eq!(parse(&render(s)), s);
        }
        assert_eq!(
            render(CLOSED_LEFT),
            "[sidebar]\nopen = false\ndock = \"left\"\n"
        );
    }

    #[test]
    fn closing_the_sidebar_is_remembered_for_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/mydiff/state.toml");
        with_sidebar_at(Some(&path), |s| {
            assert_eq!(s, Sidebar::default(), "first run: defaults");
            Ok(CLOSED_LEFT)
        })
        .unwrap();
        with_sidebar_at(Some(&path), |s| {
            assert_eq!(s, CLOSED_LEFT, "second run starts as the first ended");
            Ok(s)
        })
        .unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, ["state.toml"], "no temporary file left behind");
    }

    #[test]
    fn unchanged_state_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        with_sidebar_at(Some(&path), Ok).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn unwritable_state_dir_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let blocker = dir.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let path = blocker.join("state.toml"); // parent is a file
        with_sidebar_at(Some(&path), |_| Ok(CLOSED_LEFT)).unwrap();
    }

    #[test]
    fn viewer_errors_still_surface() {
        let err = with_sidebar_at(None, |_| Err(io::Error::other("boom"))).unwrap_err();
        assert_eq!(err.to_string(), "boom");
    }

    #[test]
    fn no_state_dir_means_defaults_every_time() {
        with_sidebar_at(None, |s| {
            assert_eq!(s, Sidebar::default());
            Ok(CLOSED_LEFT)
        })
        .unwrap();
    }
}
