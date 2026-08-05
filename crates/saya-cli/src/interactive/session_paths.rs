use std::{
    env,
    path::{Path, PathBuf},
};

/// Returns the path to the interactive session input history file.
pub fn default_history_file() -> PathBuf {
    let session_dir = default_session_dir();
    match session_dir.parent() {
        Some(parent) => parent.join("input_history"),
        None => session_dir.join("input_history"),
    }
}

pub fn default_session_dir() -> PathBuf {
    let override_dir = env::var_os("SAYA_SESSION_DIR").map(PathBuf::from);
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

pub fn resolve_session_dir(
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
        return path.join("saya/sessions");
    }
    if let Some(path) = appdata {
        return path.join("saya/sessions");
    }
    home.map(|path| path.join(".local/share/saya/sessions"))
        .unwrap_or_else(|| PathBuf::from(".local/share/saya/sessions"))
}
