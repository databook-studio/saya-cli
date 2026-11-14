//! Crash recovery for private checkpoints: one bounded pass that converts
//! every `Prepared` record to `Uncertain` — the edit's outcome is unknown,
//! never silently committed or discarded — and removes only orphan
//! before-images (no record) and dot-prefixed staging temps, as regular
//! files and never following links. Foreign and invalid entries are left
//! alone and counted as skipped; repeating passes converge.

use super::files::{dir_path, image_path, record_path, write_record};
use super::scan::{Class, SCAN_BOUND, classify};
use super::{CheckpointStoreError, RecoveryReport, gated_session, invalid};
use crate::FsSessionStore;
use crate::filesystem::stamp;
use crate::private_file::io_error;
use crate::session_lifecycle::delete::remove_path_no_follow;
use saya_types::{CheckpointId, CheckpointState};
use std::{fs, io::ErrorKind, path::Path};

/// The `updated` stamp a recovery rewrite carries: now, but never before
/// `created`.
fn updated_after(created_unix_ms: i64) -> i64 {
    i64::try_from(stamp())
        .unwrap_or(i64::MAX)
        .max(created_unix_ms)
}

/// Removes one regular file as an orphan; a symlink, a directory, or a
/// failed removal is left alone and counted as skipped.
fn remove_orphan(path: &Path, report: &mut RecoveryReport) {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => match remove_path_no_follow(path) {
            Ok(()) => report.orphans_removed += 1,
            Err(_) => report.skipped += 1,
        },
        _ => report.skipped += 1,
    }
}

/// Converts one `Prepared` record to `Uncertain` and rewrites it; a record
/// that fails to load is left alone and counted as skipped, and only a
/// failing rewrite propagates.
fn convert_or_skip(
    path: &Path,
    id: &CheckpointId,
    session_id: &str,
    dir: &Path,
    report: &mut RecoveryReport,
) -> Result<(), CheckpointStoreError> {
    let record = match super::records::read_record_file(path, id, session_id) {
        Ok(record) => record,
        Err(CheckpointStoreError::NotFound | CheckpointStoreError::Corrupt) => {
            report.skipped += 1;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if record.state != CheckpointState::Prepared {
        return Ok(());
    }
    let mut uncertain = record;
    uncertain.state = CheckpointState::Uncertain;
    uncertain.updated_unix_ms = updated_after(uncertain.created_unix_ms);
    let bytes = uncertain.to_json_bytes().map_err(|_| invalid())?;
    write_record(dir, id.as_str(), &bytes)?;
    report.marked_uncertain += 1;
    Ok(())
}

fn sweep(dir: &Path, session_id: &str) -> Result<RecoveryReport, CheckpointStoreError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RecoveryReport::default()),
        Err(error) => return Err(CheckpointStoreError::from(io_error(error))),
    };
    let mut report = RecoveryReport::default();
    let mut examined = 0usize;
    for entry in entries.filter_map(Result::ok) {
        let Ok(name) = entry.file_name().into_string() else {
            report.skipped += 1;
            continue;
        };
        match classify(&name) {
            Class::Record(id) => {
                if examined >= SCAN_BOUND {
                    break;
                }
                examined += 1;
                convert_or_skip(&entry.path(), &id, session_id, dir, &mut report)?;
            }
            Class::Image(id) => {
                match fs::symlink_metadata(record_path(dir, id.as_str())) {
                    Ok(_) => continue,
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(_) => {
                        report.skipped += 1;
                        continue;
                    }
                }
                remove_orphan(&image_path(dir, id.as_str()), &mut report);
            }
            Class::Temp => remove_orphan(&entry.path(), &mut report),
            Class::Foreign => report.skipped += 1,
        }
    }
    Ok(report)
}

impl FsSessionStore {
    /// One recovery pass over the session's checkpoints. At most
    /// [`SCAN_BOUND`] record files are examined per pass; a missing
    /// directory reports nothing to recover.
    pub fn recover_checkpoints(
        &self,
        session_id: &str,
    ) -> Result<RecoveryReport, CheckpointStoreError> {
        gated_session(self, session_id)?;
        sweep(&dir_path(self.sessions_root(), session_id), session_id)
    }
}
