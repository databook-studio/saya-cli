//! The stage-or-reuse engine behind `saya open <FILE>`: the source file is
//! read and hashed exactly once by `saya-harness`'s staging (format detected
//! from that one read) into a private staging directory under the files root;
//! the snapshot directory is chosen from the content digest plus the parse
//! contract, an existing snapshot of the same content AND contract is reused
//! (its file untouched, the session built from its own stored metadata), and
//! otherwise the fresh staging — annotated with its contract and preview —
//! is placed there.

use std::{fs, path::Path, path::PathBuf};

use saya_harness::file_source::{self, CsvStageOptions, Preview, STAGED_DB_FILE, SourceFormat};

use super::contract;
use super::root::{TempDirGuard, ensure_root, fresh_temp_dir};
use super::snapshot::{SnapshotMeta, dir_name, valid_snapshot};
use super::stored;

/// One open file's session inputs: where the snapshot lives, what it holds,
/// and the preview the snapshot's own stored metadata describes.
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
    pub format: SourceFormat,
    pub reused: bool,
}

/// Stages (or reuses) the snapshot for `source` under `root`. `typed` joins
/// the parse contract, so a `--typed` open never reuses a plain snapshot.
pub(super) fn stage_or_reuse(
    source: &Path,
    root: &Path,
    options: CsvStageOptions,
    typed: bool,
    reset: bool,
) -> Result<StagedSession, String> {
    ensure_root(root)?;
    let temp = fresh_temp_dir(root)?;
    let guard = TempDirGuard::new(temp.clone());
    let receipt =
        file_source::stage_source(source, &temp, options).map_err(|error| error.to_string())?;
    let contract = contract::canonical(receipt.format, &receipt.preview, &receipt.table, typed);
    let name = dir_name(&receipt.sha256, &contract)
        .ok_or_else(|| "staging produced an unexpected digest".to_owned())?;
    let target = root.join(name);
    if !reset && reuses(&target, &receipt.sha256, &contract) {
        drop(guard);
        let meta = valid_snapshot(&target).ok_or("the reused snapshot lost its metadata")?;
        return reused_session(&target, &meta)
            .ok_or_else(|| "the reused snapshot's stored metadata is incomplete".to_owned());
    }
    stored::annotate(&temp.join(STAGED_DB_FILE), &contract, &receipt.preview)?;
    if let Ok(existing) = fs::symlink_metadata(&target) {
        if existing.is_symlink() || !existing.is_dir() {
            return Err(format!(
                "refusing to replace {}: not a saya-staged source directory",
                target.display()
            ));
        }
        match valid_snapshot(&target) {
            None => {
                return Err(format!(
                    "refusing to replace {}: it is not a valid saya-staged source; remove it yourself if unwanted",
                    target.display()
                ));
            }
            Some(existing) if existing.contract.as_deref() != Some(contract.as_str()) => {
                return Err(format!(
                    "refusing to replace {}: its stored contract differs from this open's",
                    target.display()
                ));
            }
            Some(_) => {}
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

/// Reuse means: a valid snapshot exists whose stored sha256 is this
/// content's sha256 and whose stored contract is this open's contract
/// byte-for-byte (the directory digest is only the short form), with its
/// preview recorded.
fn reuses(target: &Path, sha256: &str, contract: &str) -> bool {
    valid_snapshot(target).is_some_and(|meta| {
        meta.sha256 == sha256
            && meta.contract.as_deref() == Some(contract)
            && meta.preview.is_some()
    })
}

/// The session for a reused snapshot, built from the snapshot's own stored
/// metadata — never mixed with the receipt this run staged. `None` means the
/// stored state is unusable; the caller restages (never mutates).
fn reused_session(dir: &Path, meta: &SnapshotMeta) -> Option<StagedSession> {
    let contract = meta.contract.as_deref()?;
    let preview = stored::preview_from_json(meta.preview.as_deref()?)?;
    if preview.columns.len() as u64 != meta.columns {
        return None;
    }
    Some(StagedSession {
        dir: dir.to_path_buf(),
        db_path: dir.join(STAGED_DB_FILE),
        table: contract::table_of(contract)?.to_owned(),
        rows: usize::try_from(meta.rows).ok()?,
        bytes: meta.bytes,
        sha256: meta.sha256.clone(),
        file_name: meta.file_name.clone(),
        staged_unix_ms: meta.staged_unix_ms,
        preview,
        format: format_of(&meta.format)?,
        reused: true,
    })
}

fn format_of(label: &str) -> Option<SourceFormat> {
    match label {
        "csv" => Some(SourceFormat::Csv),
        "parquet" => Some(SourceFormat::Parquet),
        _ => None,
    }
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
        format: receipt.format,
        reused,
    }
}
