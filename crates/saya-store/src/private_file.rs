//! Shared private-file plumbing for the store's file-backed repositories:
//! bounded reads, unique staging names, unix permission enforcement, and the
//! single stage-and-publish sequence. Both the session store and the
//! investigation repository write through here, so "private temp + fsync +
//! atomic rename in the same directory" stays one implementation instead of
//! two copies that can drift.

use crate::StoreError;
use crate::replace::{Replacer, publish_staged, publish_staged_no_replace};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn io_error(_: std::io::Error) -> StoreError {
    StoreError::unavailable()
}

/// A unique staging path beside `target`: dot-prefixed so id-stem scans
/// ignore it, pid- and counter-tagged for uniqueness, `.tmp`-suffixed.
pub(crate) fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("session.json");
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), sequence))
}

/// Reads at most `max` bytes, taking one extra so oversize is detected
/// rather than silently truncated into a half-record; `Ok(None)` when the
/// file does not exist.
pub(crate) fn bounded_read(path: &Path, max: usize) -> Result<Option<Vec<u8>>, StoreError> {
    let file = match fs::File::open(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    let mut bytes = Vec::new();
    file.take(max as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() > max {
        return Err(StoreError::LimitExceeded);
    }
    Ok(Some(bytes))
}

#[cfg(unix)]
pub(crate) fn set_mode(path: &Path, mode: u32) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io_error)
}

/// Stages `data` at `temp` — created `create_new`, mode 0600 on unix,
/// fully written and fsynced. On failure the partial temp is removed.
fn stage(temp: &Path, data: &[u8]) -> Result<(), StoreError> {
    let result = (|| {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(temp).map_err(io_error)?;
        file.write_all(data).map_err(io_error)?;
        file.sync_all().map_err(io_error)?;
        #[cfg(unix)]
        set_mode(temp, 0o600)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

/// Stages `data` at a private temp beside `target` — created `create_new`,
/// mode 0600 on unix, fully written and fsynced — then publishes it through
/// `replacer`, whose rename either installs the complete new file or leaves
/// the existing target untouched. On any failure the temp is removed and any
/// existing target is byte-for-byte unchanged.
pub(crate) fn stage_and_publish(
    target: &Path,
    data: &[u8],
    replacer: &dyn Replacer,
) -> Result<(), StoreError> {
    let temp = temporary_path(target);
    let result = stage(&temp, data).and_then(|()| publish_staged(&temp, target, replacer));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Stages `data` as [`stage`] does, then publishes it at `target` without
/// replacement via [`publish_staged_no_replace`] — the exclusive publish
/// create uses, so a target that appears after the caller's checks is a
/// conflict, never an overwrite. The temp link is removed on every outcome:
/// a successful exclusive publish leaves it beside the newly linked target.
pub(crate) fn stage_and_publish_no_replace(target: &Path, data: &[u8]) -> Result<(), StoreError> {
    let temp = temporary_path(target);
    let result = stage(&temp, data).and_then(|()| publish_staged_no_replace(&temp, target));
    let _ = fs::remove_file(&temp);
    result
}
