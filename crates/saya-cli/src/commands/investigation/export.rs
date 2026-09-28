//! `saya investigation export` (D5): writes one saved definition's portable
//! document to a file for sharing. Only the definition travels — the
//! per-machine local review binding stays beside the stored document and is
//! never included. The write mirrors the store's private-file sequence (a
//! private temp in the destination's directory, `create_new`, 0600 on unix,
//! fsync, rename), so a refused or failed export leaves any existing
//! destination byte-for-byte unchanged and no temp file behind.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{EXIT_INVESTIGATION_ERROR, parse_investigation_id, store_failure};
use crate::commands::output::{failure_message, result};
use crate::render::RenderFormat;
use saya_store::InvestigationRepository;

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The export flow: read the definition, serialize it, and publish it to the
/// destination atomically behind the explicit overwrite decision.
pub(super) fn export(
    repo: &InvestigationRepository,
    format: RenderFormat,
    id: &str,
    path: &Path,
    overwrite: bool,
) -> Result<i32, Box<dyn std::error::Error>> {
    let id = match parse_investigation_id(id) {
        Ok(id) => id,
        Err((code, message)) => return failure_message(code, message, format),
    };
    let definition = match repo.get(&id) {
        Ok(definition) => definition,
        Err(error) => return store_failure(error, id.as_str(), format),
    };
    // The definition alone, never the binding: `get` returns only the
    // portable document, and this serializes exactly that.
    let json = definition.to_json_pretty()?;
    if let Err(message) = publish(path, json.as_bytes(), overwrite) {
        return failure_message(EXIT_INVESTIGATION_ERROR, message, format);
    }
    result(
        format!(
            "Exported {} to {}.\nThe file holds the portable definition only — review the SQL before sharing.",
            id.as_str(),
            path.display()
        ),
        format,
    )
}

/// Refuses a destination the export must never write through: a symlink
/// (always — the link could point anywhere), a directory, or an existing
/// file when `overwrite` was not given. A missing destination is fine.
fn guard(path: &Path, overwrite: bool) -> Result<(), String> {
    let display = path.display();
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("could not inspect {display}: {error}")),
        Ok(meta) if meta.file_type().is_symlink() => Err(format!(
            "{display} is a symlink; refusing to export over it"
        )),
        Ok(meta) if meta.is_dir() => Err(format!("{display} is a directory; choose a file path")),
        Ok(_) if !overwrite => Err(format!("{display} exists; pass --overwrite")),
        Ok(_) => Ok(()),
    }
}

/// Publishes complete `bytes` at `path` through a private temp file in the
/// destination's directory, re-checking the guard just before the rename so
/// a file created after the first check cannot be clobbered unless
/// `--overwrite` was given. Any failure removes the temp and leaves the
/// destination untouched.
fn publish(path: &Path, bytes: &[u8], overwrite: bool) -> Result<(), String> {
    guard(path, overwrite)?;
    let temp = temporary_path(path);
    let result = stage_and_rename(&temp, path, bytes, overwrite);
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn stage_and_rename(
    temp: &Path,
    target: &Path,
    bytes: &[u8],
    overwrite: bool,
) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(temp)
        .map_err(|e| format!("could not create the export temporary file: {e}"))?;
    file.write_all(bytes)
        .map_err(|e| format!("could not write the export temporary file: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("could not fsync the export temporary file: {e}"))?;
    guard(target, overwrite)?;
    fs::rename(temp, target)
        .map_err(|e| format!("could not publish the export to {}: {e}", target.display()))
}

/// A unique staging path beside `target`: dot-prefixed so directory scans
/// ignore it, pid- and counter-tagged for uniqueness, `.tmp`-suffixed.
fn temporary_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("export");
    let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), sequence))
}
