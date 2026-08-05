use std::{
    env,
    path::{Path, PathBuf},
};

pub(crate) fn state_db_path() -> PathBuf {
    if let Some(path) = env::var_os("SAYA_STATE_DB") {
        return PathBuf::from(path);
    }
    let xdg = env::var_os("XDG_DATA_HOME");
    let appdata = env::var_os("APPDATA");
    let home = env::var_os("HOME");
    platform_root(
        xdg.as_deref().map(Path::new),
        appdata.as_deref().map(Path::new),
        home.as_deref().map(Path::new),
    )
    .join("saya/state.sqlite3")
}

pub fn resolve_state_db_path(
    override_path: Option<&str>,
    xdg: Option<&str>,
    appdata: Option<&str>,
    home: Option<&str>,
) -> PathBuf {
    if let Some(path) = override_path {
        return path.into();
    }
    platform_root(
        xdg.map(Path::new),
        appdata.map(Path::new),
        home.map(Path::new),
    )
    .join("saya/state.sqlite3")
}

fn platform_root(xdg: Option<&Path>, appdata: Option<&Path>, home: Option<&Path>) -> PathBuf {
    if let Some(path) = xdg {
        return path.into();
    }
    if let Some(path) = appdata {
        return path.into();
    }
    let home = home.map(PathBuf::from).unwrap_or_else(env::temp_dir);
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support")
    }
    #[cfg(not(target_os = "macos"))]
    {
        home.join(".local/share")
    }
}
