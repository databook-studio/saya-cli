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

/// Creates (or re-enters) `sessions/<id>/` at 0700 — the session's engine
/// state: the scratch DuckDB, the single-writer lock, (from U2 on) the
/// session journal. Ids are filename-safe (the store's own rule); the id
/// guard here is the same one the store applies to its own paths.
pub(crate) fn create_state_dir(sessions_root: &Path, id: &str) -> Result<PathBuf, String> {
    if id.is_empty() || id.contains('/') || id.contains('\\') || id == "." || id == ".." {
        return Err(format!("session id {id:?} is not path-safe"));
    }
    let dir = sessions_root.join(id);
    std::fs::create_dir_all(&dir).map_err(|error| {
        format!(
            "could not create the session state dir {}: {error}",
            dir.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(
            |error| format!("could not restrict the session state dir to 0700: {error}"),
        )?;
    }
    Ok(dir)
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
