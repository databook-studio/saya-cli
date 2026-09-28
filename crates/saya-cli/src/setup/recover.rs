//! The interruption marker: what an interrupted commit leaves behind, how a
//! later run finds it, and the restore/finish recovery it offers.
//!
//! The marker (`<dir>/.setup-commit.json`) is written before any target is
//! touched and removed only after the commit (or its rollback) is done, so a
//! crash anywhere in between leaves a truthful record.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{CONFIG_FILE, CONNECTIONS_FILE, MAX_FILE_BYTES, SetupError, atomic};

pub(crate) const MARKER_FILE: &str = ".setup-commit.json";
pub(crate) const BACKUP_DIR: &str = ".setup-backup";
const MARKER_MAX_BYTES: u64 = 64 * 1024;

/// One marker entry. `created: true` means the commit made the file (no
/// backup, it is deleted on restore); otherwise `backup` names the file in
/// `.setup-backup/` that holds the pre-commit bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerEntry {
    pub file: String,
    pub backup: Option<String>,
    pub created: bool,
}

/// A parsed marker: a commit that may or may not have completed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingCommit {
    pub started_unix_ms: u64,
    pub entries: Vec<MarkerEntry>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarkerV1 {
    version: u32,
    started_unix_ms: u64,
    entries: Vec<MarkerEntry>,
}

/// Finds a pending (possibly interrupted) commit, or `None` when no marker
/// exists. Corrupt or unsupported markers are reported, never ignored.
pub fn pending(dir: &Path) -> Result<Option<PendingCommit>, SetupError> {
    let path = dir.join(MARKER_FILE);
    let Some(bytes) = atomic::read_optional_bounded(&path, MARKER_MAX_BYTES)? else {
        return Ok(None);
    };
    let marker: MarkerV1 = serde_json::from_slice(&bytes)
        .map_err(|error| SetupError::Marker(format!("corrupt marker: {error}")))?;
    if marker.version != 1 {
        return Err(SetupError::Marker(format!(
            "unsupported marker version {}",
            marker.version
        )));
    }
    for entry in &marker.entries {
        validate_entry(entry)?;
    }
    Ok(Some(PendingCommit {
        started_unix_ms: marker.started_unix_ms,
        entries: marker.entries,
    }))
}

/// Writes the marker for a commit that is about to start. 0600, atomic.
pub(crate) fn write_marker(
    dir: &Path,
    started_unix_ms: u64,
    entries: &[MarkerEntry],
) -> Result<(), SetupError> {
    let marker = MarkerV1 {
        version: 1,
        started_unix_ms,
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&marker)
        .map_err(|error| SetupError::Marker(format!("marker serialize: {error}")))?;
    atomic::publish(&dir.join(MARKER_FILE), &bytes)
}

fn validate_entry(entry: &MarkerEntry) -> Result<(), SetupError> {
    known_file(&entry.file)?;
    if let Some(backup) = &entry.backup {
        known_file(backup)?;
    }
    if entry.created == entry.backup.is_some() {
        return Err(SetupError::Marker(
            "marker entry mixes backup and created".into(),
        ));
    }
    Ok(())
}

fn known_file(name: &str) -> Result<(), SetupError> {
    if name != CONFIG_FILE && name != CONNECTIONS_FILE {
        return Err(SetupError::Marker(format!(
            "marker names unknown file {name:?}"
        )));
    }
    if Path::new(name)
        .file_name()
        .is_none_or(|found| found != name)
    {
        return Err(SetupError::Marker(format!(
            "marker names a non-plain path {name:?}"
        )));
    }
    Ok(())
}

/// Restores an interrupted commit's originals: every backed-up file goes back
/// byte-for-byte, files the commit created are deleted, then marker and
/// backups are removed. Idempotent.
pub fn restore(dir: &Path, pending: &PendingCommit) -> Result<(), SetupError> {
    for entry in &pending.entries {
        let target = dir.join(&entry.file);
        if let Some(backup) = &entry.backup {
            let backup_path = dir.join(BACKUP_DIR).join(backup);
            if let Some(bytes) = atomic::read_optional_bounded(&backup_path, MAX_FILE_BYTES)? {
                atomic::refuse_symlink(&target)?;
                atomic::publish(&target, &bytes)?;
            }
        } else if entry.created {
            atomic::refuse_symlink(&target)?;
            let _ = std::fs::remove_file(&target);
        }
    }
    clear(dir)
}

/// Keeps the current files and removes the marker and backups. Idempotent.
pub fn finish(dir: &Path, _pending: &PendingCommit) -> Result<(), SetupError> {
    clear(dir)
}

/// Removes the marker and the backups directory, refusing symlinks.
pub(crate) fn clear(dir: &Path) -> Result<(), SetupError> {
    atomic::remove_if_present(&dir.join(MARKER_FILE), false)?;
    atomic::remove_if_present(&dir.join(BACKUP_DIR), true)
}
