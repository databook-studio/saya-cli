//! The Windows working-tree capture reader: the same contract as the unix
//! reader — revalidate the plan's source identity, open no-follow, stream and
//! hash, verify again after the last byte — expressed with the path walk and
//! `GetFileInformationByHandle` identity the workspace module already proves.

use std::io::{Read as _, Write};

use saya_agent::CancellationToken;
use saya_types::Sha256Digest;
use sha2::{Digest as _, Sha256};

use crate::workspace::Workspace;
use crate::workspace::contain::path_identity;

use super::super::{
    manifest::{RedactedIoError, SnapshotError},
    symlink::validate_symlink_target,
    types::{
        MAX_SNAPSHOT_FILE_BYTES, MAX_SYMLINK_TARGET_BYTES, SnapshotContentSource,
        SnapshotFileEntry, SnapshotFileMode, SnapshotSourceIdentity,
    },
};
use super::{CHUNK_BYTES, CapturedFile};

/// Reads one planned working-tree file in a single contained open, streaming
/// it into `sink` while hashing. The per-file ceiling and the aggregate budget
/// are enforced on the cumulative bytes during the stream; a budget below the
/// planned size refuses before the open.
pub fn read_working_file(
    workspace: &Workspace,
    entry: &SnapshotFileEntry,
    remaining_aggregate_bytes: u64,
    sink: &mut dyn Write,
    cancellation: &CancellationToken,
) -> Result<CapturedFile, SnapshotError> {
    if !matches!(entry.source(), SnapshotContentSource::WorkingTree) {
        return Err(SnapshotError::SourceNotWorkingTree);
    }
    let Some(identity) = entry.source_identity() else {
        return Err(SnapshotError::SourceIdentityChanged);
    };
    if cancellation.is_cancelled() {
        return Err(SnapshotError::Cancelled);
    }
    let declared = identity.size();
    if declared > MAX_SNAPSHOT_FILE_BYTES || declared > remaining_aggregate_bytes {
        return Err(SnapshotError::TooLarge);
    }
    if entry.mode() == SnapshotFileMode::Symlink {
        return read_working_symlink(
            workspace,
            entry,
            identity,
            remaining_aggregate_bytes,
            sink,
            cancellation,
        );
    }
    if entry.mode() == SnapshotFileMode::Executable {
        // Windows has no executable bit; a plan that claims one did not come
        // from this host's selection.
        return Err(SnapshotError::SourceIdentityChanged);
    }
    let path = entry.path().as_str();
    let target = workspace
        .target(path, false)
        .map_err(|error| refuse("scan the snapshot source", error))?;
    let pre = std::fs::symlink_metadata(&target)
        .map_err(|error| scan_error("scan the snapshot source", error))?;
    if pre.file_type().is_symlink() {
        return Err(SnapshotError::SourceSymlink);
    }
    if !pre.is_file() {
        return Err(SnapshotError::SourceNotRegularFile);
    }
    if pre.len() != declared {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    let scanned_identity = path_identity(&target)
        .map_err(|error| scan_error("identify the snapshot source", error))?;
    if scanned_identity != (identity.device, identity.inode) {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    let mut file = workspace
        .open_verified(&target, &pre, path)
        .map_err(|error| refuse("open the snapshot source", error))?;
    let (hasher, scanned) = stream_captured(
        &mut file,
        identity,
        remaining_aggregate_bytes,
        sink,
        cancellation,
    )?;
    let final_meta = std::fs::symlink_metadata(&target)
        .map_err(|error| scan_error("verify the captured source", error))?;
    let final_identity =
        path_identity(&target).map_err(|error| scan_error("verify the captured source", error))?;
    if final_identity != (identity.device, identity.inode)
        || !final_meta.is_file()
        || final_meta.len() != scanned
    {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    Ok(CapturedFile {
        sha256: digest_of(hasher)?,
        bytes: scanned,
    })
}

/// Reads one planned symlink: the target string is the content. Windows
/// spells link targets with backslashes, so the stored and hashed form is the
/// forward-slash spelling the snapshot validator accepts.
fn read_working_symlink(
    workspace: &Workspace,
    entry: &SnapshotFileEntry,
    identity: &SnapshotSourceIdentity,
    remaining_aggregate_bytes: u64,
    sink: &mut dyn Write,
    cancellation: &CancellationToken,
) -> Result<CapturedFile, SnapshotError> {
    let declared = identity.size();
    if declared > MAX_SYMLINK_TARGET_BYTES as u64 || declared > remaining_aggregate_bytes {
        return Err(SnapshotError::TooLarge);
    }
    if cancellation.is_cancelled() {
        return Err(SnapshotError::Cancelled);
    }
    let path = entry.path().as_str();
    let target = workspace_entry_target(workspace, path)?;
    let before = std::fs::symlink_metadata(&target)
        .map_err(|error| scan_error("scan the snapshot symlink", error))?;
    if !before.file_type().is_symlink() {
        return Err(SnapshotError::SourceNotSymlink);
    }
    let link = std::fs::read_link(&target)
        .map_err(|error| scan_error("read the snapshot symlink", error))?;
    let link = link.to_string_lossy().replace('\\', "/");
    if link.len() as u64 != declared {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    validate_symlink_target(path, link.as_bytes())?;
    if cancellation.is_cancelled() {
        return Err(SnapshotError::Cancelled);
    }
    sink.write_all(link.as_bytes())
        .map_err(|error| capture_io("write the captured symlink", error))?;
    let after = std::fs::symlink_metadata(&target)
        .map_err(|error| scan_error("verify the captured symlink", error))?;
    if !after.file_type().is_symlink() || after.len() != declared {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    let mut hasher = Sha256::new();
    hasher.update(link.as_bytes());
    Ok(CapturedFile {
        sha256: digest_of(hasher)?,
        bytes: declared,
    })
}

/// The absolute path a workspace-relative entry names. The final component is
/// deliberately not passed through `Workspace::target`, which refuses a link;
/// the parent is walked and the link itself is scanned without following it.
fn workspace_entry_target(
    workspace: &Workspace,
    path: &str,
) -> Result<std::path::PathBuf, SnapshotError> {
    let (parent_rel, name) = match path.rsplit_once('/') {
        Some((parent, name)) => (parent, name),
        None => ("", path),
    };
    let parent = if parent_rel.is_empty() {
        workspace.root().to_path_buf()
    } else {
        workspace
            .target(parent_rel, false)
            .map_err(|error| refuse("scan the snapshot symlink", error))?
    };
    Ok(parent.join(name))
}

fn stream_captured(
    file: &mut std::fs::File,
    identity: &SnapshotSourceIdentity,
    remaining_aggregate_bytes: u64,
    sink: &mut dyn Write,
    cancellation: &CancellationToken,
) -> Result<(Sha256, u64), SnapshotError> {
    let declared = identity.size();
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; CHUNK_BYTES];
    let mut scanned = 0u64;
    loop {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let read = file
            .read(&mut chunk)
            .map_err(|error| capture_io("read the snapshot source", error))?;
        if read == 0 {
            break;
        }
        scanned = scanned
            .checked_add(read as u64)
            .ok_or(SnapshotError::TooLarge)?;
        if scanned > declared {
            return Err(SnapshotError::SourceIdentityChanged);
        }
        if scanned > MAX_SNAPSHOT_FILE_BYTES || scanned > remaining_aggregate_bytes {
            return Err(SnapshotError::TooLarge);
        }
        hasher.update(&chunk[..read]);
        sink.write_all(&chunk[..read])
            .map_err(|error| capture_io("write the captured chunk", error))?;
    }
    if scanned != declared {
        return Err(SnapshotError::SourceIdentityChanged);
    }
    Ok((hasher, scanned))
}

fn digest_of(hasher: Sha256) -> Result<Sha256Digest, SnapshotError> {
    let hex: String = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    Sha256Digest::parse(&hex).map_err(|_| SnapshotError::Serialization)
}

fn capture_io(description: &str, source: std::io::Error) -> SnapshotError {
    SnapshotError::CaptureIo {
        description: description.to_owned(),
        source: Some(RedactedIoError::new(source)),
    }
}

fn scan_error(description: &str, source: std::io::Error) -> SnapshotError {
    if source.kind() == std::io::ErrorKind::NotFound {
        return SnapshotError::SourceDisappeared;
    }
    capture_io(description, source)
}

fn refuse(description: &str, error: crate::HarnessError) -> SnapshotError {
    match error {
        crate::HarnessError::SymlinkRefused { .. } => SnapshotError::SourceSymlink,
        crate::HarnessError::NotFound { .. } => SnapshotError::SourceDisappeared,
        crate::HarnessError::NotRegularFile { .. } => SnapshotError::SourceNotRegularFile,
        crate::HarnessError::IdentityChanged { .. } => SnapshotError::SourceIdentityChanged,
        other => SnapshotError::CaptureIo {
            description: format!("{description}: {other}"),
            source: None,
        },
    }
}
