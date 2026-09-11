//! The resume decision, made read-only before a request is built.
//!
//! [`verify_resume`] re-digests the recorded prefix straight from the disk —
//! the tamper check — and hands back the plan whose hasher already carries
//! the prefix, so finishing the download continues the digest into the whole
//! file's. [`reopen`](self::reopen) then opens the part file for appending
//! and truncates the unaccounted tail. Nothing here writes until the body
//! actually streams; a mismatch leaves the evidence in place.

use std::io::Read;

use sha2::{Digest, Sha256};

use super::download_error::DownloadError;
use std::path::PathBuf;

use super::partial::{PartialMeta, READ_CHUNK, hex, part_rel};
use crate::{HarnessError, io_error, workspace::Workspace};

/// A verified resume plan: the recorded partial is digest-checked against
/// the disk, and the hasher is already fed the verified prefix — so
/// finishing the download continues the digest into the whole file's.
pub(super) struct ResumePlan {
    pub meta: PartialMeta,
    pub hasher: Sha256,
}

/// Verifies a recorded partial, read-only: the part file must exist, hold at
/// least the recorded length, and digest — re-read from disk — to exactly
/// the recorded digest. Any mismatch is a typed refusal that leaves the
/// evidence in place; nothing is written here, so a resumed download that is
/// then refused (by the policy or a bad status) has touched nothing.
/// Returns `None` when there is nothing recorded to resume (a zero-length
/// sidecar whose part file is gone).
pub(super) fn verify_resume(
    workspace: &Workspace,
    destination: &str,
    meta: &PartialMeta,
) -> Result<Option<ResumePlan>, DownloadError> {
    let part_rel = part_rel(destination);
    let mismatch = |detail: String| DownloadError::ResumeMismatch {
        path: destination.to_owned(),
        detail,
    };
    let path = match workspace.target(&part_rel, false) {
        Ok(path) => path,
        Err(HarnessError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            // A zero-length sidecar with no part file is an empty start that
            // crashed before its first byte — nothing to resume, and the
            // caller starts fresh. A recorded length with no file is lost
            // progress: suspicious, so refused rather than restarted over.
            if meta.len == 0 {
                return Ok(None);
            }
            return Err(mismatch("the recorded partial file is missing".to_owned()));
        }
        Err(error) => return Err(error.into()),
    };
    let pre = std::fs::symlink_metadata(&path)
        .map_err(|error| io_error("scan download partial", &path, error))?;
    if !pre.is_file() {
        return Err(mismatch(
            "the recorded partial is not a regular file".to_owned(),
        ));
    }
    if pre.len() < meta.len {
        return Err(mismatch(format!(
            "the partial is {} bytes, shorter than the recorded {}",
            pre.len(),
            meta.len
        )));
    }
    // Re-digest the recorded prefix from the disk, through the no-follow,
    // identity-checked read path — the tamper check.
    let mut file = workspace.open_verified(&path, &pre, &part_rel)?;
    let mut hasher = Sha256::new();
    let mut remaining = meta.len;
    let mut buffer = vec![0u8; READ_CHUNK];
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let read = file
            .read(&mut buffer[..want])
            .map_err(|error| mismatch(format!("reading the partial back failed: {error}")))?;
        if read == 0 {
            return Err(mismatch(
                "the partial ended before its recorded length".to_owned(),
            ));
        }
        remaining -= read as u64;
        hasher.update(&buffer[..read]);
    }
    drop(file);
    if hex(hasher.clone().finalize().as_slice()) != meta.sha256 {
        return Err(mismatch(
            "the partial's bytes do not match its recorded digest".to_owned(),
        ));
    }
    Ok(Some(ResumePlan {
        meta: meta.clone(),
        hasher,
    }))
}

/// Reopens a verified partial for appending: a no-follow append at 0600,
/// truncated back to the recorded length so the unaccounted tail beyond it
/// (a crash between a chunk write and the sidecar update) is discarded —
/// never re-fetched on top of.
pub(super) fn reopen(
    workspace: &Workspace,
    destination: &str,
    meta: &PartialMeta,
) -> Result<(PathBuf, std::fs::File), DownloadError> {
    let (path, file) = workspace
        .open_download_part(&part_rel(destination), false)
        .map_err(DownloadError::from)?;
    file.set_len(meta.len)
        .map_err(|error| DownloadError::ResumeMismatch {
            path: destination.to_owned(),
            detail: format!("truncating the partial's tail failed: {error}"),
        })?;
    Ok((path, file))
}
