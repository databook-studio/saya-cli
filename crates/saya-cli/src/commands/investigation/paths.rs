//! Where saved-investigation documents live (D2): `SAYA_INVESTIGATIONS_DIR`
//! when set, else `<platform data dir>/saya/investigations` — the directory
//! beside the state database, sharing its resolution chain.

use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// The investigations root for this invocation. The env override is read
/// here and nowhere else, so every operation sees the same root.
pub(super) fn investigations_root() -> PathBuf {
    root_from(
        env::var_os("SAYA_INVESTIGATIONS_DIR").as_deref(),
        &crate::state_path::state_db_path(),
    )
}

/// Pure root resolution: the env override wins; otherwise the documents sit
/// beside the state database. Split from [`investigations_root`] so tests
/// never touch process env.
pub(super) fn root_from(env: Option<&OsStr>, state_db: &Path) -> PathBuf {
    if let Some(path) = env {
        return PathBuf::from(path);
    }
    state_db
        .parent()
        .map(|dir| dir.join("investigations"))
        .unwrap_or_else(|| PathBuf::from("investigations"))
}
