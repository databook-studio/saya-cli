//! The restore half of recovery: putting an interrupted commit's originals back
//! truthfully — a failing step is recorded and the rest still run, but the marker
//! and backups stay until every entry succeeded or was legitimately skipped.

use std::path::Path;

use super::atomic;
use super::recover::{self, MarkerEntry, PendingCommit};
use super::{MAX_FILE_BYTES, SetupError};

/// What restore did to one marker entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// The backup bytes were put back.
    Restored { file: String },
    /// A file the commit created is gone (deleted now, or already absent).
    Removed { file: String },
    /// The file was left untouched, with the reason.
    LeftUnchanged { file: String, reason: String },
}

/// What a restore did: one outcome per marker entry, in marker order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreReport {
    pub outcomes: Vec<RestoreOutcome>,
}

/// Restores an interrupted commit's originals byte-for-byte; idempotent.
pub fn restore(dir: &Path, pending: &PendingCommit) -> Result<RestoreReport, SetupError> {
    let mut outcomes = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    // Evidence is judged before any file is touched, not while restoring.
    let inconsistent = inconsistent_missing_backups(dir, pending);
    for entry in &pending.entries {
        match step(dir, entry, inconsistent.contains(&entry.file)) {
            Ok(outcome) => outcomes.push(outcome),
            Err(failure) => failures.push(failure),
        }
    }
    if failures.is_empty() {
        recover::clear(dir)?;
        Ok(RestoreReport { outcomes })
    } else {
        Err(SetupError::RestoreIncomplete { failures })
    }
}

/// One entry's restore step: perform it, reporting outcome or recorded failure.
fn step(dir: &Path, entry: &MarkerEntry, inconsistent: bool) -> Result<RestoreOutcome, String> {
    let file = &entry.file;
    let target = dir.join(file);
    let Some(backup) = &entry.backup else {
        // The commit made the file, so restore deletes it: already absent is
        // fine (idempotent); other failures are recorded, never ignored.
        if let Err(error) = atomic::refuse_symlink(&target) {
            return Err(format!(
                "{file}: could not remove the created file: {error}"
            ));
        }
        return match std::fs::remove_file(&target) {
            Ok(()) => Ok(RestoreOutcome::Removed { file: file.clone() }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok(RestoreOutcome::Removed { file: file.clone() })
            }
            Err(source) => Err(format!(
                "{file}: could not remove the created file: {source}"
            )),
        };
    };
    let backup_path = dir.join(recover::BACKUP_DIR).join(backup);
    match atomic::read_optional_bounded(&backup_path, MAX_FILE_BYTES) {
        Ok(Some(bytes)) => match atomic::publish(&target, &bytes) {
            Ok(()) => Ok(RestoreOutcome::Restored { file: file.clone() }),
            Err(error) => Err(format!(
                "{file}: could not put the original bytes back: {error}"
            )),
        },
        Ok(None) => {
            // The commit writes every backup before any publish, so an absent
            // backup means no publishing yet — unless other files prove it did.
            if inconsistent {
                Err(format!(
                    "{file}: backup absent but other files show publishing had started; \
                     the state is inconsistent"
                ))
            } else {
                Ok(RestoreOutcome::LeftUnchanged {
                    file: file.clone(),
                    reason: "backup absent; commit had not modified it".into(),
                })
            }
        }
        Err(error) => Err(format!("{file}: could not read the backup: {error}")),
    }
}

/// The entries whose absent backup is contradicted by other files proving
/// publishing had started: their targets may hold published bytes no backup
/// can undo, so they must fail instead of being declared untouched.
fn inconsistent_missing_backups(dir: &Path, pending: &PendingCommit) -> Vec<String> {
    let mut names = Vec::new();
    for entry in &pending.entries {
        let Some(backup) = &entry.backup else {
            continue;
        };
        let backup_path = dir.join(recover::BACKUP_DIR).join(backup);
        // Only an absent backup is judged; unreadable ones are their own step's error.
        if !matches!(
            atomic::read_optional_bounded(&backup_path, MAX_FILE_BYTES),
            Ok(None)
        ) || !publish_started(dir, pending, &entry.file)
        {
            continue;
        }
        names.push(entry.file.clone());
    }
    names
}

/// Whether any entry other than `absent` proves publishing had started: a
/// backed-up file whose current bytes differ from its backup, or a created
/// file that now exists. A file that cannot be read counts as evidence:
/// consistency cannot be proven, so the marker must stay.
fn publish_started(dir: &Path, pending: &PendingCommit, absent: &str) -> bool {
    for other in &pending.entries {
        if other.file == absent {
            continue;
        }
        let target = dir.join(&other.file);
        let Some(backup) = &other.backup else {
            // A created file that now exists: the commit had reached publish.
            if target.exists() {
                return true;
            }
            continue;
        };
        let backup_path = dir.join(recover::BACKUP_DIR).join(backup);
        let backup_bytes = match atomic::read_optional_bounded(&backup_path, MAX_FILE_BYTES) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => continue,
            Err(_) => return true,
        };
        match atomic::read_optional_bounded(&target, MAX_FILE_BYTES) {
            Ok(Some(target_bytes)) if target_bytes != backup_bytes => return true,
            Ok(_) => {}
            Err(_) => return true,
        }
    }
    false
}
