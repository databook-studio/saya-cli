//! The private file an over-bound shell command is saved to: the exact bytes
//! the operator is asked to approve, under the session's state directory and
//! never inside the workspace. The prompt shows both ends of the command and
//! names this file; the answer is bound to the bytes it holds.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

/// The subdirectory of the session state directory the command files live
/// under, kept apart from the journal and the job's other records.
const APPROVAL_SUBDIR: &str = "approval";

/// The prefix and suffix of one saved command, so the stale sweep can find
/// only this module's files.
const FILE_PREFIX: &str = "approval-";
const FILE_SUFFIX: &str = ".cmd";

/// The most stale files one save sweeps. One approval is pending at a time,
/// so a larger directory is not this module's state; the cap keeps the sweep
/// bounded anyway.
const MAX_SWEPT_FILES: usize = 64;

/// The path one command's exact bytes are saved to. The name is the command's
/// SHA-256, so the answer check can recompute it without carrying state.
pub(super) fn path(state_dir: &Path, command: &str) -> PathBuf {
    state_dir.join(APPROVAL_SUBDIR).join(format!(
        "{FILE_PREFIX}{}{FILE_SUFFIX}",
        saya_types::sha256_hex(command.as_bytes())
    ))
}

/// Saves `command`'s exact bytes under `state_dir`, refusing a state
/// directory inside `workspace`. The file is created private (0600), read
/// back, and removed again if the bytes do not match. A failure is the
/// caller's cue to show the whole command rather than hide a tail behind a
/// file that does not hold it.
pub(super) fn save(
    state_dir: &Path,
    workspace: Option<&Path>,
    command: &str,
) -> io::Result<PathBuf> {
    // Refuse before creating anything: a state directory inside the
    // workspace must not gain even an empty directory. The check runs again
    // after creation, when both paths can be canonicalised.
    if !outside_workspace(state_dir, workspace) {
        return Err(refuse_inside_workspace());
    }
    fs::create_dir_all(state_dir)?;
    if !outside_workspace(state_dir, workspace) {
        return Err(refuse_inside_workspace());
    }
    let dir = state_dir.join(APPROVAL_SUBDIR);
    fs::create_dir_all(&dir)?;
    set_private_dir(&dir);
    sweep_stale(&dir);
    let path = path(state_dir, command);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&path)?;
    file.write_all(command.as_bytes())?;
    file.sync_all()?;
    if fs::read(&path)? != command.as_bytes() {
        let _ = fs::remove_file(&path);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "the saved command did not read back",
        ));
    }
    Ok(path)
}

/// Whether `path` still holds exactly `command`'s bytes.
pub(super) fn matches(path: &Path, command: &str) -> bool {
    fs::read(path).is_ok_and(|bytes| bytes == command.as_bytes())
}

/// Removes one saved command. Absence is not an error: the file is removed on
/// every answer and on the session's end, and either may come first.
pub(super) fn remove(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Removes every saved command under `state_dir`: the session-end sweep, so
/// an ask abandoned by a closed session leaves nothing behind.
pub(super) fn remove_all(state_dir: &Path) {
    let _ = fs::remove_dir_all(state_dir.join(APPROVAL_SUBDIR));
}

/// The saved commands currently under `state_dir`, for the tests that pin
/// the one-file lifecycle. Read-only and bounded by the directory's own
/// contents.
#[cfg(test)]
pub(crate) fn saved(state_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(state_dir.join(APPROVAL_SUBDIR)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| is_saved(path))
        .take(MAX_SWEPT_FILES)
        .collect()
}

fn is_saved(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with(FILE_PREFIX) && name.ends_with(FILE_SUFFIX))
}

fn sweep_stale(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten().take(MAX_SWEPT_FILES) {
        if is_saved(&entry.path()) {
            let _ = fs::remove_file(entry.path());
        }
    }
}

fn refuse_inside_workspace() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "the approval directory is inside the workspace",
    )
}

/// Whether `state_dir` is outside `workspace`. Canonical paths decide when
/// both exist; a path that cannot be canonicalised falls back to its lexical
/// form, so a missing workspace root never turns a containment refusal into a
/// write.
fn outside_workspace(state_dir: &Path, workspace: Option<&Path>) -> bool {
    let Some(workspace) = workspace else {
        return true;
    };
    let state = fs::canonicalize(state_dir).unwrap_or_else(|_| state_dir.to_path_buf());
    let root = fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    !state.starts_with(&root)
}

#[cfg(unix)]
fn set_private_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn set_private_dir(_path: &Path) {}
