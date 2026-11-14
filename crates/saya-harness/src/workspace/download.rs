//! The download surface's containment (unix): the partial-file open, the
//! completed-part promotion, and the sidecar removal. Each one validates its
//! argument through the anchored walk and then acts on the descriptors the
//! walk held — the open, the rename, and the removal all anchor to the
//! directories the walk validated, so a swap of an intermediate directory
//! between the walk and the act cannot divert either side of it.

use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};

use super::anchored::final_stat;
use super::commit_gate::hold;
use super::contain::Workspace;
use super::download_guard::{DESTINATION_DIGEST_MAX_BYTES, digest_matches};
use crate::HarnessError;
use crate::io_error;
use std::io;

/// Opens the download machinery's partial file under containment: the anchored
/// walk with directory creation, then a no-follow open at 0600 — `truncate`
/// for a fresh download, append for a resume — with a post-open identity
/// check so the file streamed into is the one that was scanned. Never
/// executable.
pub(crate) fn open_download_part(
    ws: &Workspace,
    rel: &str,
    truncate: bool,
) -> Result<(PathBuf, fs::File), HarnessError> {
    let anchor = ws.anchor(rel, true)?;
    let mut flags = libc::O_WRONLY | libc::O_CREAT;
    if truncate {
        flags |= libc::O_TRUNC;
    } else {
        flags |= libc::O_APPEND;
    }
    let file = anchor.open_file(flags, 0o600).map_err(|error| {
        if error.raw_os_error() == Some(libc::ELOOP) {
            return HarnessError::SymlinkRefused {
                path: rel.to_string(),
            };
        }
        io_error("open workspace download part", anchor.path(), error)
    })?;
    let opened = file
        .metadata()
        .map_err(|error| io_error("stat opened download part", anchor.path(), error))?;
    let current = anchor
        .stat_final()
        .map_err(|error| io_error("verify opened download part", anchor.path(), error))?;
    if (opened.dev(), opened.ino()) != current.identity() {
        return Err(HarnessError::IdentityChanged {
            path: rel.to_string(),
        });
    }
    Ok((anchor.path().to_path_buf(), file))
}

/// Promotes a completed download part over `dest_rel` atomically, under the
/// destination guard (`download_guard`): both part and destination are
/// anchored, and the rename acts on those anchors, so it names the
/// destination *there* — never wherever the paths resolve now. No expected
/// digest is create-only: an existing destination — file or directory —
/// refuses [`HarnessError::AlreadyExists`], destination and part untouched.
/// A digest replaces only a regular file whose current bytes hash to it —
/// streamed, bounded by the download file limit — refusing
/// [`HarnessError::NotFound`] on a missing destination (a digest grants no
/// authority to create), [`HarnessError::NotRegularFile`],
/// [`HarnessError::BoundsExceeded`], or [`HarnessError::ContentChanged`];
/// every refusal leaves the part in place. The destination is re-verified
/// afterwards (regular file, never executable, still the inode that was
/// renamed) so a completed download is never pretended onto a swapped path.
pub(crate) fn promote(
    ws: &Workspace,
    part_rel: &str,
    dest_rel: &str,
    expected_destination_sha256: Option<&str>,
) -> Result<(), HarnessError> {
    // The gate is held from before the part and destination walks to after
    // the rename, so two Saya writers serialize their guard-then-publish
    // window — the same optimistic stale-version guard the guarded write
    // and patch hold, not an OS-level compare-and-swap against external
    // programs.
    let _gate = hold();
    let part = ws.anchor(part_rel, false)?;
    let part_stat = final_stat(&part, "stat download part", part_rel)?;
    if !part_stat.is_file() {
        return Err(HarnessError::NotRegularFile {
            path: part_rel.to_string(),
        });
    }
    let dest = ws.anchor(dest_rel, true)?;
    match (expected_destination_sha256, dest.stat()) {
        // Create-only: an existing destination — file or directory —
        // refuses, destination and part untouched.
        (None, Some(_)) => {
            return Err(HarnessError::AlreadyExists {
                path: dest_rel.to_string(),
            });
        }
        // A digest grants no authority to create: a missing destination is
        // a refusal, and it discloses no digest to invent one from.
        (Some(_), None) => {
            return Err(HarnessError::NotFound {
                path: dest_rel.to_string(),
            });
        }
        // Replace only when the destination's current bytes hash to the
        // caller's observed digest: a regular file within the download file
        // limit, streamed — never loaded whole.
        (Some(expected), Some(stat)) => {
            if !stat.is_file() {
                return Err(HarnessError::NotRegularFile {
                    path: dest_rel.to_string(),
                });
            }
            if stat.len() > DESTINATION_DIGEST_MAX_BYTES {
                return Err(HarnessError::BoundsExceeded {
                    path: dest_rel.to_string(),
                    found: stat.len(),
                    max: DESTINATION_DIGEST_MAX_BYTES,
                });
            }
            let file = dest.open_verified(stat.identity(), dest_rel)?;
            digest_matches(dest_rel, dest.path(), &file, expected)?;
        }
        // Nothing names the destination: the rename is a create.
        (None, None) => {}
    }
    dest.rename_from(&part)?;
    let final_stat = dest
        .stat_final()
        .map_err(|error| io_error("verify written workspace file", dest.path(), error))?;
    if final_stat.identity() != part_stat.identity()
        || !final_stat.is_file()
        || final_stat.has_exec_bits()
    {
        return Err(HarnessError::IdentityChanged {
            path: dest_rel.to_string(),
        });
    }
    Ok(())
}

/// Removes a workspace file relative to its anchored parent; absence is
/// already-done, not an error. The sidecar removal's anchored form.
pub(crate) fn unlink(ws: &Workspace, rel: &str) -> Result<(), HarnessError> {
    let anchor = ws.anchor(rel, false)?;
    if anchor.stat().is_none() {
        return Ok(());
    }
    match anchor.unlink_final() {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("remove download sidecar", anchor.path(), error)),
    }
}
