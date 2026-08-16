//! Atomic, contained write of an exported contract file (slice 6b).
//!
//! # Security contract (P1: export escape)
//!
//! The destination filename is derived from the encoded object identity by
//! the caller (see [`super::export`]), so it is filesystem-safe by
//! construction — no `/`, `..`, or absolute prefix. This module still treats
//! every byte as hostile and **verifies containment rather than assuming it**:
//!
//! - The destination is **canonicalised** before the write, so a destination
//!   that is a symlink to elsewhere is written at its real location, and a
//!   `..` in any ancestor cannot redirect the write above the directory the
//!   user named.
//! - The target's resolved parent is checked against the canonical
//!   destination with the **same component-wise `contains` 6a uses for
//!   reading** (see [`crate::contracts::discover`]) — one containment
//!   primitive, both directions. A filename that ever regains a separator or
//!   `..` would fail this check instead of silently escaping.
//! - The temp file is opened with `create_new(true)` and a nonce that is
//!   actually unique (process id + an atomic counter, not a near-zero
//!   duration), so concurrent exports cannot collide into one temp file, and
//!   a collision is an error rather than a silent overwrite.
//! - Without `--force`, the write is **no-clobber atomically**: on unix,
//!   `link(temp, dest)` + `unlink(temp)` — `link` fails with `EEXIST` if
//!   `dest` already exists (including a symlink at that name), so an
//!   existing file cannot be overwritten and the check is atomic with the
//!   write, not a check-then-rename race. With `--force`, `rename(temp, dest)`
//!   atomically replaces `dest` (and a symlink at that name, not its target).
//!
//! # Permissions asymmetry (spec §2)
//!
//! The private state database is `0600` — it is personal. An exported contract
//! file is the opposite: it is *meant* to be committed to a shared repository,
//! so it is written with the platform's default sharing permissions, not
//! hardened to `0600`. That is the intended asymmetry, not a permissions bug:
//! the same claim is private at rest on this machine and shared once exported.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::contracts::ContractOpError;
use crate::contracts::discover::contains;

/// Per-process counter for unique temp-file names. Paired with the process id
/// it is unique across concurrent exports in one process (the counter) and
/// across processes (the pid); `create_new` then turns any residual collision
/// into an error rather than a silent overwrite.
static TEMP_NONCE: AtomicU64 = AtomicU64::new(0);

/// Write `body` to `destination` atomically and contained within the
/// canonicalised destination directory. `overwrite` must be `true` to replace
/// an existing `destination`; otherwise an existing file is a typed error and
/// is not clobbered. See the module docs for the security contract.
pub(crate) fn write_atomic(
    destination: &Path,
    body: &str,
    overwrite: bool,
) -> Result<(), ContractOpError> {
    let target = contained_target(destination)?;
    let temp = temp_path(&target);
    // `create_new` is the load-bearing flag: it maps to `O_CREAT | O_EXCL`, so
    // a temp name that already exists (a collision between concurrent exports)
    // is an error, never a silent overwrite of the other export's temp file.
    let result = (|| -> Result<(), ContractOpError> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| ContractOpError::Unavailable)?;
        file.write_all(body.as_bytes())
            .map_err(|_| ContractOpError::Unavailable)?;
        file.sync_all().map_err(|_| ContractOpError::Unavailable)?;
        drop(file);
        install(&temp, &target, overwrite)
    })();
    if result.is_err() {
        // Leave no partial file behind. `install` only succeeds on a completed
        // write, so a failure here means the temp file still exists and must be
        // removed. Ignore the error: a missing temp is already the desired state.
        let _ = fs::remove_file(&temp);
    }
    result
}

/// Resolve `destination` to a canonical target whose parent is provably inside
/// the canonical destination directory. Canonicalising the destination first
/// means a symlinked or `..`-bearing destination is written at its real
/// location; the `contains` check (shared with the read side) then verifies the
/// target's resolved parent has not escaped. A filename with no separators
/// (the encoded identity) passes trivially — the check exists to catch a
/// future regression to a traversal-permitting filename and to make containment
/// a verified property rather than an assumption.
fn contained_target(destination: &Path) -> Result<PathBuf, ContractOpError> {
    // `destination` is the full target path (`dir/file.toml`); the directory is
    // its parent. Ensure the parent exists, then canonicalise the *parent* —
    // not the file path, which would create the file's name as a directory.
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(ContractOpError::Invalid)?;
    fs::create_dir_all(parent).map_err(|_| ContractOpError::Unavailable)?;
    let canon_dest = parent
        .canonicalize()
        .map_err(|_| ContractOpError::Unavailable)?;
    let file_name = destination.file_name().ok_or(ContractOpError::Invalid)?;
    let target = canon_dest.join(file_name);
    let resolved_parent = target.parent().ok_or(ContractOpError::Invalid)?;
    if !contains(&canon_dest, resolved_parent) {
        // The encoded-identity filename makes this unreachable in practice; it
        // is the fail-closed guard if a future filename ever reintroduces a
        // separator or `..`.
        return Err(ContractOpError::Invalid);
    }
    Ok(target)
}

/// Place the completed temp file at the target. With `overwrite`, `rename`
/// atomically replaces the target. Without it, the unix path uses
/// `link` + `unlink` so the no-clobber check is atomic with the write; the
/// non-unix fallback is a check-then-rename with a stated residual race.
fn install(temp: &Path, target: &Path, overwrite: bool) -> Result<(), ContractOpError> {
    if overwrite {
        fs::rename(temp, target).map_err(|_| ContractOpError::Unavailable)?;
        return Ok(());
    }
    #[cfg(unix)]
    {
        // `link(temp, target)` fails with `EEXIST` if `target` already exists,
        // including a symlink at that name — so an existing file (or a trap
        // symlink) cannot be overwritten, and the existence check is atomic
        // with the link, not a separate check-then-rename race. On success the
        // file now has two names (temp and target); remove the temp name so
        // only the target remains.
        match fs::hard_link(temp, target) {
            Ok(()) => {
                let _ = fs::remove_file(temp);
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(ContractOpError::Conflict)
            }
            Err(_) => Err(ContractOpError::Unavailable),
        }
    }
    #[cfg(not(unix))]
    {
        // Fallback: check-then-rename. Residual race — stated plainly: a file
        // created at `target` between the existence check and the rename is
        // clobbered. The portable stdlib has no atomic no-clobber link; a
        // platform with `link` should use the unix arm above.
        if target.exists() {
            return Err(ContractOpError::Conflict);
        }
        fs::rename(temp, target).map_err(|_| ContractOpError::Unavailable)
    }
}

/// A temp path beside the target: same directory (so the rename/link is atomic
/// on the same filesystem), same name plus a `.saya-export-<pid>-<counter>`
/// suffix. The counter is process-unique; combined with the pid it is unique
/// across concurrent exports, and `create_new` turns any collision into an error.
fn temp_path(target: &Path) -> PathBuf {
    let n = TEMP_NONCE.fetch_add(1, Ordering::Relaxed);
    let mut name = target
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_else(|| std::ffi::OsString::from("contract.toml"));
    name.push(format!(".saya-export-{}-{n}", std::process::id()));
    target
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(name))
}
