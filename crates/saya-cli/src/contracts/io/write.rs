//! Atomic write of an exported contract file (slice 6b).
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

/// Write `body` to `destination` atomically: a temp file in the destination's
/// parent directory, then rename. A half-written contract file a colleague pulls
/// is worse than no file, so the visible path only appears once the bytes are
/// fully written and renamed. On any failure after the temp file is created, the
/// temp file is removed so no partial file is left behind (spec §2, test 8).
///
/// `overwrite` must be `true` to replace an existing `destination`; otherwise an
/// existing file is a typed error.
pub(crate) fn write_atomic(destination: &Path, body: &str, overwrite: bool) -> Result<(), IoError> {
    if destination.exists() && !overwrite {
        return Err(IoError::DestinationExists(
            destination.display().to_string(),
        ));
    }
    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|_| IoError::WriteFailed)?;
    let temp = temp_path(destination);
    let result = (|| -> Result<(), IoError> {
        let mut file = fs::File::create(&temp).map_err(|_| IoError::WriteFailed)?;
        file.write_all(body.as_bytes())
            .map_err(|_| IoError::WriteFailed)?;
        file.sync_all().map_err(|_| IoError::WriteFailed)?;
        drop(file);
        fs::rename(&temp, destination).map_err(|_| IoError::WriteFailed)
    })();
    if result.is_err() {
        // Leave no partial file behind. The rename only runs on success, so a
        // failure here means the temp file exists and must be removed.
        let _ = fs::remove_file(&temp);
    }
    result
}

/// A temp path beside the destination: same directory, same name plus a
/// `.saya-export-<pid>-<nanos>` suffix. Same-directory so the rename is atomic
/// on the same filesystem; unique so parallel exports do not collide.
fn temp_path(destination: &Path) -> PathBuf {
    let nanos = std::time::Instant::now().elapsed().as_nanos();
    let base = destination
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_else(|| std::ffi::OsString::from("contract.toml"));
    let mut name = base;
    name.push(format!(".saya-export-{}-{nanos}", std::process::id()));
    destination
        .parent()
        .map(|p| p.join(&name))
        .unwrap_or_else(|| PathBuf::from(name))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IoError {
    #[error("destination already exists: {0} (pass --force to overwrite)")]
    DestinationExists(String),
    #[error("could not write the contract file")]
    WriteFailed,
}
