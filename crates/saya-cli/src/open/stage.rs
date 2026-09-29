//! The stage-or-reuse engine behind `saya open <FILE>`: the source file is
//! read and hashed exactly once by `saya-harness`'s `stage_csv` into a
//! private staging directory under the files root; the snapshot directory is
//! chosen from the digest; an existing snapshot of the same content is
//! reused (its file untouched), otherwise the fresh staging is placed there.

use std::{fs, path::Path, path::PathBuf};

use saya_harness::file_source::{self, CsvStageOptions, Preview, STAGED_DB_FILE};

use super::root::{TempDirGuard, ensure_root, fresh_temp_dir};
use super::snapshot::{SnapshotMeta, dir_name, valid_snapshot};

/// One open file's session inputs: where the snapshot lives, what it holds,
/// and the preview that was parsed this run (identical for reused content).
#[derive(Debug, Clone)]
pub(super) struct StagedSession {
    pub dir: PathBuf,
    pub db_path: PathBuf,
    pub table: String,
    pub rows: usize,
    pub bytes: u64,
    pub sha256: String,
    pub file_name: String,
    pub staged_unix_ms: u64,
    pub preview: Preview,
    pub reused: bool,
}

/// Stages (or reuses) the snapshot for `source` under `root`.
pub(super) fn stage_or_reuse(
    source: &Path,
    root: &Path,
    options: CsvStageOptions,
    reset: bool,
) -> Result<StagedSession, String> {
    ensure_root(root)?;
    let temp = fresh_temp_dir(root)?;
    let guard = TempDirGuard::new(temp.clone());
    let receipt =
        file_source::stage_csv(source, &temp, options).map_err(|error| error.to_string())?;
    let name = dir_name(&receipt.sha256)
        .ok_or_else(|| "staging produced an unexpected digest".to_owned())?;
    let target = root.join(name);
    if !reset && reuses(&target, &receipt.sha256) {
        drop(guard);
        let meta = valid_snapshot(&target).ok_or("the reused snapshot lost its metadata")?;
        return Ok(session(&target, &receipt, &meta, true));
    }
    if let Ok(existing) = fs::symlink_metadata(&target) {
        if existing.is_symlink() || !existing.is_dir() {
            return Err(format!(
                "refusing to replace {}: not a saya-staged source directory",
                target.display()
            ));
        }
        if valid_snapshot(&target).is_none() {
            return Err(format!(
                "refusing to replace {}: it is not a valid saya-staged source; remove it yourself if unwanted",
                target.display()
            ));
        }
        fs::remove_dir_all(&target)
            .map_err(|error| format!("clear {}: {error}", target.display()))?;
    }
    guard.disarm();
    fs::rename(&temp, &target).map_err(|error| format!("place staged source: {error}"))?;
    let meta = valid_snapshot(&target)
        .ok_or_else(|| "the staged snapshot did not validate after placement".to_owned())?;
    Ok(session(&target, &receipt, &meta, false))
}

/// Reuse means: a valid snapshot exists whose full metadata sha256 is this
/// content's sha256.
fn reuses(target: &Path, sha256: &str) -> bool {
    valid_snapshot(target).is_some_and(|meta| meta.sha256 == sha256)
}

fn session(
    dir: &Path,
    receipt: &file_source::StagedSource,
    meta: &SnapshotMeta,
    reused: bool,
) -> StagedSession {
    StagedSession {
        dir: dir.to_path_buf(),
        db_path: dir.join(STAGED_DB_FILE),
        table: receipt.table.clone(),
        rows: receipt.rows,
        bytes: receipt.bytes,
        sha256: receipt.sha256.clone(),
        file_name: meta.file_name.clone(),
        staged_unix_ms: meta.staged_unix_ms,
        preview: receipt.preview.clone(),
        reused,
    }
}
