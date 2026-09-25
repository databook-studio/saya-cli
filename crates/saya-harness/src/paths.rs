//! Where run directories live on disk, and the env var that moves them.
//!
//! The runs root resolves like the interactive session directory's root: a
//! `SAYA_RUNS_DIR` override first, then the platform data home, so runs and
//! sessions sit beside each other by default.

use std::{
    env,
    path::{Path, PathBuf},
};

/// The env var that overrides the runs root. When unset, the root follows
/// `XDG_DATA_HOME`, then `APPDATA`, then `HOME`, exactly like the session
/// directory's resolution.
pub const RUNS_DIR_ENV: &str = "SAYA_RUNS_DIR";

/// Returns the runs root, honouring [`RUNS_DIR_ENV`] when set.
pub fn default_runs_dir() -> PathBuf {
    let override_dir = env::var_os(RUNS_DIR_ENV).map(PathBuf::from);
    let xdg = env::var_os("XDG_DATA_HOME").map(PathBuf::from);
    let appdata = env::var_os("APPDATA").map(PathBuf::from);
    let home = env::var_os("HOME").map(PathBuf::from);
    resolve_paths(
        override_dir.as_deref(),
        xdg.as_deref(),
        appdata.as_deref(),
        home.as_deref(),
    )
}

/// The testable form of [`default_runs_dir`]: resolves the runs root from
/// explicitly supplied candidates in override → XDG → APPDATA → HOME order.
pub fn resolve_runs_dir(
    override_dir: Option<&str>,
    xdg: Option<&str>,
    appdata: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    resolve_paths(
        override_dir.map(Path::new),
        xdg.map(Path::new),
        appdata.map(Path::new),
        home.map(Path::new),
    )
}

fn resolve_paths(
    override_dir: Option<&Path>,
    xdg: Option<&Path>,
    appdata: Option<&Path>,
    home: Option<&Path>,
) -> PathBuf {
    if let Some(path) = override_dir {
        return path.into();
    }
    if let Some(path) = xdg {
        return path.join("saya/runs");
    }
    if let Some(path) = appdata {
        return path.join("saya/runs");
    }
    home.map(|path| path.join(".local/share/saya/runs"))
        .unwrap_or_else(|| PathBuf::from(".local/share/saya/runs"))
}
