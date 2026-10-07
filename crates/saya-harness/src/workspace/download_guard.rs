//! The destination guard for download publication: the create-only and
//! digest-replace preconditions every promotion checks at the rename, plus
//! the non-unix promotion body. No digest is create-if-absent — an existing
//! destination, file or directory, refuses `AlreadyExists` untouched; a
//! digest replaces only when the destination's current bytes hash to it and
//! refuses `NotFound` when absent — a digest never authorizes creation —
//! and `ContentChanged` on a mismatch (wrong-shape text included). The read
//! is streamed, bounded by the download file limit, inside the commit gate
//! at the rename — the guarded write's optimistic stale-version window.

use std::{fs, io::Read as _, path::Path};

use sha2::{Digest, Sha256};

use crate::{HarnessError, io_error};

#[cfg(not(unix))]
use super::commit_gate::hold;
#[cfg(not(unix))]
use super::contain::{Workspace, path_identity, replace_workspace_file};

#[cfg(test)]
#[path = "download_guard_tests.rs"]
mod download_guard_tests;

/// The bound for hashing an existing destination before replacing it: the
/// download file limit — the largest file a download could have placed
/// there. The ordinary 8 MiB whole-file bound does not apply.
pub(super) const DESTINATION_DIGEST_MAX_BYTES: u64 = crate::fetch::DEFAULT_MAX_FILE_BYTES;

/// Streams `file` through SHA-256 in 64 KiB chunks — refused past
/// [`DESTINATION_DIGEST_MAX_BYTES`] — refusing `ContentChanged` on mismatch.
pub(super) fn digest_matches(
    rel: &str,
    path: &Path,
    mut file: &fs::File,
    expected: &str,
) -> Result<(), HarnessError> {
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut scanned = 0u64;
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|error| io_error("hash download destination", path, error))?;
        if read == 0 {
            break;
        }
        scanned += read as u64;
        if scanned > DESTINATION_DIGEST_MAX_BYTES {
            return Err(HarnessError::BoundsExceeded {
                path: rel.to_string(),
                found: scanned,
                max: DESTINATION_DIGEST_MAX_BYTES,
            });
        }
        hasher.update(&chunk[..read]);
    }
    let current: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if current != expected {
        return Err(HarnessError::ContentChanged {
            path: rel.to_string(),
            expected: expected.to_string(),
            current,
        });
    }
    Ok(())
}

/// The non-unix promotion, by path: the unix body's guard semantics, with
/// the committed file re-verified afterwards.
#[cfg(not(unix))]
pub(super) fn promote_by_path(
    ws: &Workspace,
    part_rel: &str,
    dest_rel: &str,
    expected_destination_sha256: Option<&str>,
) -> Result<(), HarnessError> {
    let _gate = hold();
    let dest = ws.target(dest_rel, true)?;
    let part = ws.target(part_rel, false)?;
    let part_identity = {
        let meta = fs::symlink_metadata(&part)
            .map_err(|error| io_error("stat download part", &part, error))?;
        if !meta.is_file() {
            return Err(HarnessError::NotRegularFile {
                path: part_rel.to_string(),
            });
        }
        path_identity(&part).map_err(|error| io_error("stat download part", &part, error))?
    };
    match fs::symlink_metadata(&dest) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if expected_destination_sha256.is_some() {
                return Err(HarnessError::NotFound {
                    path: dest_rel.to_string(),
                });
            }
        }
        Err(error) => return Err(io_error("scan download destination", &dest, error)),
        Ok(scan) => {
            if scan.file_type().is_symlink() {
                return Err(HarnessError::SymlinkRefused {
                    path: dest_rel.to_string(),
                });
            }
            match expected_destination_sha256 {
                // Create-only: an existing destination refuses untouched.
                None => {
                    return Err(HarnessError::AlreadyExists {
                        path: dest_rel.to_string(),
                    });
                }
                // Replace only on a digest-matched regular file within the limit.
                Some(expected) => {
                    if !scan.is_file() {
                        return Err(HarnessError::NotRegularFile {
                            path: dest_rel.to_string(),
                        });
                    }
                    if scan.len() > DESTINATION_DIGEST_MAX_BYTES {
                        return Err(HarnessError::BoundsExceeded {
                            path: dest_rel.to_string(),
                            found: scan.len(),
                            max: DESTINATION_DIGEST_MAX_BYTES,
                        });
                    }
                    let file = ws.open_verified(&dest, &scan, dest_rel)?;
                    digest_matches(dest_rel, &dest, &file, expected)?;
                }
            }
        }
    }
    replace_workspace_file(&part, &dest)?;
    let final_meta = fs::symlink_metadata(&dest)
        .map_err(|error| io_error("verify written workspace file", &dest, error))?;
    let exec_bits = false;
    let committed = path_identity(&dest)
        .map_err(|error| io_error("verify written workspace file", &dest, error))?;
    if committed != part_identity || !final_meta.is_file() || exec_bits {
        return Err(HarnessError::IdentityChanged {
            path: dest_rel.to_string(),
        });
    }
    Ok(())
}
