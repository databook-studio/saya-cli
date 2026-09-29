//! The generated connections file: one read-only DuckDB profile for the
//! staged snapshot (`file_<table>`), written atomically beside the snapshot
//! database — the same private staged-sibling + fsync + rename pattern `saya
//! demo` uses, restated here because that module's helpers are private.

use std::{
    fs::{self, OpenOptions},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

/// Writes the one-profile read-only connections file beside the snapshot and
/// returns its path (the launch override and the printed launch command).
pub(super) fn write_connections(
    dir: &Path,
    profile: &str,
    db_path: &Path,
) -> Result<PathBuf, String> {
    let path = dir.join("connections.toml");
    let db = toml::Value::String(db_path.display().to_string()).to_string();
    let content =
        format!("[profiles.{profile}]\ntype = \"duckdb\"\npath = {db}\nread_only = true\n");
    publish_private(&path, &content)?;
    Ok(path)
}

/// Atomic publish of `content` at `path`: a private staged sibling (0600),
/// fsync, rename — refusing (never replacing) a symlink or directory target.
fn publish_private(path: &Path, content: &str) -> Result<(), String> {
    let shown = path.display().to_string();
    if fs::symlink_metadata(path).is_ok_and(|meta| meta.is_symlink() || meta.is_dir()) {
        return Err(format!(
            "refusing to replace the symlink or directory {shown}"
        ));
    }
    let staging = staged_sibling(path);
    stage_private_file(&staging)?;
    let filled = fs::write(&staging, content).and_then(|()| sync_file(&staging));
    if let Err(error) = filled {
        let _ = fs::remove_file(&staging);
        return Err(format!("write {}: {error}", shown));
    }
    fs::rename(&staging, path).map_err(|error| {
        let _ = fs::remove_file(&staging);
        format!("publish {}: {error}", shown)
    })
}

/// Stages a brand-new private sibling file at `path` (create_new, 0600).
fn stage_private_file(path: &Path) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    options
        .open(path)
        .map(|_| ())
        .map_err(|error| format!("stage {}: {error}", path.display()))
}

fn sync_file(path: &Path) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.sync_all()
}

/// A hidden, collision-free sibling staging path for `path`.
fn staged_sibling(path: &Path) -> PathBuf {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    path.with_file_name(format!(".{name}.tmp-{}-{nanos}", std::process::id()))
}
