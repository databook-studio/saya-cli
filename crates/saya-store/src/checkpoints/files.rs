//! On-disk layout and write primitives for private checkpoint recovery
//! data: `<sessions>/<id>/checkpoints/<ck-id>.before` holds the exact
//! before-image bytes (only when one exists) beside `<ck-id>.json`, the
//! checkpoint record. Directories are created 0700 only when absent, never
//! through a symlink or over a non-directory; images and records are
//! written 0600 through the shared private staging primitives — fsync, then
//! an exclusive hard-link publish for the image and an atomic rename for
//! the record; removals never follow links.

use super::CheckpointStoreError;
use crate::private_file::{io_error, stage_and_publish, stage_and_publish_no_replace};
use crate::replace::AtomicReplace;
use crate::session_lifecycle::delete::remove_path_no_follow;
use crate::{StoreError, filesystem::is_valid_id};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use crate::private_file::set_mode;

static MINT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The checkpoints directory of one session: `<sessions>/<id>/checkpoints`.
pub(super) fn dir_path(root: &Path, session_id: &str) -> PathBuf {
    root.join(session_id).join("checkpoints")
}

/// One checkpoint's record file: `<ck-id>.json`.
pub(super) fn record_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.json"))
}

/// One checkpoint's before-image file: `<ck-id>.before`.
pub(super) fn image_path(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.before"))
}

/// Creates `<sessions>/<id>` and its `checkpoints` directory when absent —
/// 0700, never through a symlink, never over a non-directory — and returns
/// the checkpoints directory. The sessions root is created 0700 when
/// absent, matching the session store's own root.
pub(super) fn ensure_dirs(root: &Path, session_id: &str) -> Result<PathBuf, CheckpointStoreError> {
    debug_assert!(is_valid_id(session_id));
    fs::create_dir_all(root).map_err(io_error)?;
    #[cfg(unix)]
    set_mode(root, 0o700).map_err(CheckpointStoreError::from)?;
    let session_dir = root.join(session_id);
    ensure_private_dir(&session_dir)?;
    let dir = session_dir.join("checkpoints");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

fn ensure_private_dir(dir: &Path) -> Result<(), CheckpointStoreError> {
    match fs::symlink_metadata(dir) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            fs::create_dir(dir).map_err(io_error)?;
            #[cfg(unix)]
            set_mode(dir, 0o700).map_err(CheckpointStoreError::from)?;
            Ok(())
        }
        Err(error) => Err(CheckpointStoreError::from(io_error(error))),
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            Err(StoreError::Invalid.into())
        }
        Ok(_) => Ok(()),
    }
}

/// Whether anything — file, directory, or link — already sits at this id's
/// record or image path: the mint collision check.
pub(super) fn id_taken(dir: &Path, id: &str) -> Result<bool, CheckpointStoreError> {
    for path in [record_path(dir, id), image_path(dir, id)] {
        match fs::symlink_metadata(&path) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(CheckpointStoreError::from(io_error(error))),
        }
    }
    Ok(false)
}

/// Mints `ck-` + 32 lowercase hex: sha256 over nanos-since-epoch, the
/// process id, and a process-wide counter, cut to 16 bytes. The caller
/// still checks the minted id is free and retries the near-impossible
/// collision.
pub(super) fn mint_id() -> String {
    let mut hash = Sha256::new();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    hash.update(nanos.to_be_bytes());
    hash.update(std::process::id().to_be_bytes());
    hash.update(MINT_COUNTER.fetch_add(1, Ordering::Relaxed).to_be_bytes());
    let digest = hash.finalize();
    let mut id = String::from("ck-");
    for byte in &digest[..16] {
        id.push_str(&format!("{byte:02x}"));
    }
    id
}

/// The lowercase hex sha256 of `bytes` — the digest a before-image is
/// verified against.
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Publishes the exact before-image bytes at `<ck-id>.before` — 0600,
/// fsynced, exclusive: an image that appears after the caller's checks is
/// a conflict, never an overwrite.
pub(super) fn write_image(dir: &Path, id: &str, bytes: &[u8]) -> Result<(), CheckpointStoreError> {
    stage_and_publish_no_replace(&image_path(dir, id), bytes).map_err(CheckpointStoreError::from)
}

/// Publishes the record bytes at `<ck-id>.json` — 0600, fsynced, atomic
/// rename.
pub(super) fn write_record(dir: &Path, id: &str, bytes: &[u8]) -> Result<(), CheckpointStoreError> {
    stage_and_publish(&record_path(dir, id), bytes, &AtomicReplace)
        .map_err(CheckpointStoreError::from)
}

/// Removes one checkpoint's record and image files without following
/// links, record first so a crash between the two leaves an orphan image
/// that recovery removes. Absent files count as removed (idempotent).
pub(super) fn remove_checkpoint_files(dir: &Path, id: &str) -> Result<(), CheckpointStoreError> {
    remove_path_no_follow(&record_path(dir, id)).map_err(CheckpointStoreError::from)?;
    remove_path_no_follow(&image_path(dir, id)).map_err(CheckpointStoreError::from)?;
    Ok(())
}
