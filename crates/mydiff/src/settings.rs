//! Config file discovery and option resolution (the I/O half of
//! `mydiff_core::config`).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use etcetera::BaseStrategy;
use mydiff_core::DiffOptions;
use mydiff_core::config::{ConfigFile, resolve};

use crate::args::{CompareArgs, ConfigArgs};

/// Name of the per-project config file looked up in the current directory.
pub const LOCAL_CONFIG: &str = ".mydiff.toml";

/// `$XDG_CONFIG_HOME/mydiff/config.toml` (usually `~/.config/mydiff/config.toml`).
pub fn user_config_path() -> Option<PathBuf> {
    etcetera::choose_base_strategy()
        .ok()
        .map(|s| s.config_dir().join("mydiff").join("config.toml"))
}

/// Config files to load, lowest precedence first.
pub fn config_paths(cfg: &ConfigArgs) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !cfg.no_config {
        out.extend(user_config_path().filter(|p| p.is_file()));
        let local = Path::new(LOCAL_CONFIG);
        if local.is_file() {
            out.push(local.to_path_buf());
        }
    }
    out.extend(cfg.config.clone());
    out
}

fn load_file(path: &Path) -> Result<ConfigFile> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading config {}", path.display()))?;
    ConfigFile::parse(&text).with_context(|| format!("in config {}", path.display()))
}

/// Discover, parse and merge config files, then apply the profile and CLI flags.
pub fn load_options(cfg: &ConfigArgs, cmp: &CompareArgs) -> Result<DiffOptions> {
    let files = config_paths(cfg)
        .iter()
        .map(|p| load_file(p))
        .collect::<Result<Vec<_>>>()?;
    let opts = resolve(&files, cfg.profile.as_deref(), &cmp.to_layer())?;
    opts.validate()?;
    Ok(opts)
}
