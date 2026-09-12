//! Resumable partial downloads: the sidecar-plus-part-file pair that lets an
//! interrupted download continue instead of restart.
//!
//! The destination path never holds partial content. A download streams into
//! `<destination>.saya-part` and records a sidecar at
//! `<destination>.saya-part.json` — atomically, via [`Workspace::write`] —
//! carrying the requested URL, the byte length accounted for, and the SHA-256
//! of exactly those bytes. The invariant that makes every state safe:
//! `len` is always ≤ the part file's true length, and the digest always
//! matches the part file's first `len` bytes. A crash that lands bytes before
//! the sidecar caught up leaves an *older* sidecar; resuming truncates the
//! unaccounted tail rather than ever appending over or duplicating it.
//!
//! Resume is decided by digest: the first `len` bytes are re-read from disk
//! and re-digested, so a tampered or clobbered partial is a typed refusal —
//! never a shrug, and never a corrupted completion.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::HarnessError;
use crate::workspace::Workspace;

use super::download_error::DownloadError;

/// Sidecar bound: a fixed-shape JSON record, small by construction; anything
/// larger is corruption, not metadata.
const MAX_SIDECAR_BYTES: u64 = 4096;

/// Read chunk for the prefix re-digest.
pub(super) const READ_CHUNK: usize = 64 * 1024;

pub(super) fn sidecar_rel(destination: &str) -> String {
    format!("{destination}.saya-part.json")
}

pub(super) fn part_rel(destination: &str) -> String {
    format!("{destination}.saya-part")
}

/// What one partial download has recorded so far. `sha256` digests exactly
/// the first `len` bytes of the part file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PartialMeta {
    pub url: String,
    pub len: u64,
    pub sha256: String,
}

/// An open partial: the contained part file plus the digest state. The
/// hasher is continuous — for a resume it was fed the verified prefix from
/// disk, so finishing the download yields the whole file's digest directly.
pub(super) struct OpenPart {
    pub path: PathBuf,
    pub file: std::fs::File,
    pub hasher: Sha256,
}

/// Lowercase hex of a digest.
pub(super) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The digest of zero bytes — a fresh partial's recorded value.
fn empty_digest() -> String {
    hex(Sha256::digest([]).as_slice())
}

/// Loads the sidecar for `destination`, `None` when there is none. A
/// sidecar that does not parse is corruption, not absence: typed refusal.
pub(super) fn load_meta(
    workspace: &Workspace,
    destination: &str,
) -> Result<Option<PartialMeta>, DownloadError> {
    let mismatch = |detail: String| DownloadError::ResumeMismatch {
        path: destination.to_owned(),
        detail,
    };
    match workspace.read(&sidecar_rel(destination), MAX_SIDECAR_BYTES) {
        Ok(read) => {
            let meta: PartialMeta = serde_json::from_slice(&read.bytes)
                .map_err(|_| mismatch("download sidecar does not parse".to_owned()))?;
            Ok(Some(meta))
        }
        Err(HarnessError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

/// Writes the sidecar atomically (contained, 0600) — the resume contract.
pub(super) fn write_meta(
    workspace: &Workspace,
    destination: &str,
    meta: &PartialMeta,
) -> Result<(), DownloadError> {
    let bytes = serde_json::to_vec(meta).map_err(|error| DownloadError::ResumeMismatch {
        path: destination.to_owned(),
        detail: format!("download sidecar could not be serialized: {error}"),
    })?;
    workspace
        .write(&sidecar_rel(destination), &bytes)
        .map_err(DownloadError::from)
}

/// Removes the sidecar after a completed download: the destination is whole,
/// so there is no partial to resume. Absence is already-done, not an error.
pub(super) fn remove_meta(workspace: &Workspace, destination: &str) -> Result<(), DownloadError> {
    workspace
        .unlink(&sidecar_rel(destination))
        .map_err(DownloadError::from)
}

/// Starts a fresh download: the zero sidecar is written *before* the part
/// file is truncated, so a crash in between leaves a sidecar that describes
/// less than the disk holds — the safe direction (resume truncates the tail)
/// — rather than more than it holds.
pub(super) fn start_fresh(
    workspace: &Workspace,
    destination: &str,
    url: &str,
) -> Result<OpenPart, DownloadError> {
    write_meta(
        workspace,
        destination,
        &PartialMeta {
            url: url.to_owned(),
            len: 0,
            sha256: empty_digest(),
        },
    )?;
    let (path, file) = workspace
        .open_download_part(&part_rel(destination), true)
        .map_err(DownloadError::from)?;
    Ok(OpenPart {
        path,
        file,
        hasher: Sha256::new(),
    })
}

/// Promotes the completed part over the destination and clears the sidecar.
/// The part and the destination are both re-anchored through the contained
/// walk, so the promotion acts on the directories the walk validates.
pub(super) fn promote(workspace: &Workspace, destination: &str) -> Result<(), DownloadError> {
    workspace
        .promote_download(&part_rel(destination), destination)
        .map_err(DownloadError::from)?;
    remove_meta(workspace, destination)
}
