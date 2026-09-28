//! Commits a plan: private backups, an interruption marker, atomic publishes,
//! and a reload check whose failure rolls everything back.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use super::recover::{self, MarkerEntry, PendingCommit};
use super::{MAX_FILE_BYTES, SetupError, SetupPlan, atomic};

/// What a successful commit did, by plain file name.
#[derive(Debug)]
pub struct CommitReport {
    pub written: Vec<String>,
}

/// Applies `plan` inside `dir`.
///
/// 1. Creates the directory (0700 when we create it).
/// 2. Writes the marker before touching any target.
/// 3. Copies each existing target byte-for-byte into `.setup-backup/`.
/// 4. Publishes each target atomically (private temp + fsync + rename, 0600).
/// 5. Calls `reload()`; on success the backups and marker are removed, on
///    failure every backup is restored, created files deleted, and
///    [`SetupError::ReloadFailed`] returned.
///
/// A plan with zero writes is a no-op: no directory, no marker, no reload.
/// Refuses to start while an interrupted commit is pending.
pub fn commit(
    dir: &Path,
    plan: &SetupPlan,
    reload: impl FnOnce() -> Result<(), String>,
) -> Result<CommitReport, SetupError> {
    if plan.writes.is_empty() {
        return Ok(CommitReport {
            written: Vec::new(),
        });
    }
    if recover::pending(dir)?.is_some() {
        return Err(SetupError::CommitPending {
            path: dir.to_path_buf(),
        });
    }
    atomic::ensure_private_dir(dir)?;
    let started = now_unix_ms();
    let entries = planned_entries(dir, plan)?;
    recover::write_marker(dir, started, &entries)?;
    for entry in &entries {
        if let Some(backup) = &entry.backup {
            copy_backup(dir, &entry.file, backup)?;
        }
    }
    for write in &plan.writes {
        atomic::publish(&dir.join(&write.file), write.content.as_bytes())?;
    }
    match reload() {
        Ok(()) => {
            recover::clear(dir)?;
            Ok(CommitReport {
                written: plan.writes.iter().map(|write| write.file.clone()).collect(),
            })
        }
        Err(message) => {
            let pending = PendingCommit {
                started_unix_ms: started,
                entries,
            };
            let _ = recover::restore(dir, &pending);
            Err(SetupError::ReloadFailed(message))
        }
    }
}

fn planned_entries(dir: &Path, plan: &SetupPlan) -> Result<Vec<MarkerEntry>, SetupError> {
    let mut entries = Vec::new();
    for write in &plan.writes {
        let target = dir.join(&write.file);
        atomic::refuse_symlink(&target)?;
        let exists = target.exists();
        entries.push(MarkerEntry {
            file: write.file.clone(),
            backup: exists.then(|| write.file.clone()),
            created: !exists,
        });
    }
    Ok(entries)
}

fn copy_backup(dir: &Path, file: &str, backup: &str) -> Result<(), SetupError> {
    let backup_dir = dir.join(recover::BACKUP_DIR);
    atomic::refuse_symlink(&backup_dir)?;
    atomic::ensure_private_dir(&backup_dir)?;
    let bytes = atomic::read_bounded(&dir.join(file), MAX_FILE_BYTES)?;
    atomic::publish(&backup_dir.join(backup), &bytes)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
