//! The materializer's verified copy: opening one stored object no-follow,
//! validating the whole symlink set before the first write, and streaming
//! each object into the target while re-hashing the same bytes. A corrupted
//! store can only produce a typed refusal, never materialized bytes.

use std::{
    fs,
    io::{Read, Write as _},
};

use saya_agent::CancellationToken;
use saya_types::{Sha256Digest, SnapshotId};
use sha2::{Digest as _, Sha256};

use super::{
    CapturedEntry, CapturedSnapshot, SnapshotArtifacts, SnapshotError,
    capture_dest::capture_io,
    manifest,
    symlink::{SymlinkSet, validate_symlink_target},
    types::{MAX_SNAPSHOT_TOTAL_BYTES, MAX_SYMLINK_TARGET_BYTES, SnapshotFileMode},
};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::{ffi::OsStr, os::fd::AsRawFd};

#[cfg(any(target_os = "macos", target_os = "linux"))]
use super::materialize_target::TargetWriter;
#[cfg(windows)]
use super::materialize_target_windows::TargetWriter;

#[cfg(any(target_os = "macos", target_os = "linux"))]
use crate::workspace::fd::{open_dir_at, open_file_at};

#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;

#[cfg(windows)]
use crate::workspace::{contain::FILE_FLAG_OPEN_REPARSE_POINT, windows_identity};

/// The copy block size: the same 64 KiB streaming granularity the
/// anchored capture reader uses.
const CHUNK_BYTES: usize = 64 * 1024;

/// Reads every stored symlink target once, before anything is written, and
/// validates the whole set chain-aware. A chained target that leaves the
/// target root is refused before the first link is created.
pub(super) fn validate_symlink_set(
    captured: &CapturedSnapshot,
    artifacts: &SnapshotArtifacts,
    cancellation: &CancellationToken,
) -> Result<(), SnapshotError> {
    let mut links = SymlinkSet::default();
    let mut total = 0u64;
    for entry in captured.entries() {
        if entry.mode() != SnapshotFileMode::Symlink {
            continue;
        }
        let mut object = open_object(artifacts, captured.snapshot_id(), entry.content_sha256())?;
        let target = read_verified_symlink(&mut object, entry, &mut total, cancellation)?;
        links.insert(entry.path().as_str(), target);
    }
    links
        .validate()
        .map_err(|refusal| SnapshotError::unsafe_symlink(&refusal.path, refusal.reason))
}

/// Opens the stored object for one entry no-follow — the id directory,
/// then `objects/`, then the content-addressed name — and refuses
/// non-regular entries on the opened descriptor.
fn open_object(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    content_sha256: &Sha256Digest,
) -> Result<fs::File, SnapshotError> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        open_object_unix(artifacts.root_dir(), snapshot_id, content_sha256)
    }
    #[cfg(windows)]
    {
        open_object_windows(artifacts, snapshot_id, content_sha256)
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn open_object_unix(
    root_dir: &fs::File,
    snapshot_id: &SnapshotId,
    content_sha256: &Sha256Digest,
) -> Result<fs::File, SnapshotError> {
    let id_dir = open_dir_at(root_dir.as_raw_fd(), OsStr::new(snapshot_id.as_str()))
        .map_err(|error| capture_io("open stored snapshot directory", &error))
        .map(fs::File::from)?;
    let objects_dir = open_dir_at(id_dir.as_raw_fd(), OsStr::new("objects"))
        .map_err(|error| capture_io("open stored object directory", &error))
        .map(fs::File::from)?;
    let object = open_file_at(
        objects_dir.as_raw_fd(),
        OsStr::new(content_sha256.as_str()),
        libc::O_RDONLY | libc::O_NONBLOCK,
        0,
    )
    .map_err(|error| capture_io("open stored snapshot object", &error))?;
    if !object
        .metadata()
        .map_err(|error| capture_io("scan stored snapshot object", &error))?
        .is_file()
    {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(object)
}

/// The Windows path form: the id directory, `objects/`, and the object name
/// are re-checked as real directories and a real file, with the object
/// opened as the link itself.
#[cfg(windows)]
fn open_object_windows(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    content_sha256: &Sha256Digest,
) -> Result<fs::File, SnapshotError> {
    let id_dir = artifacts.root().join(snapshot_id.as_str());
    require_plain_dir(&id_dir, "open stored snapshot directory")?;
    let objects_dir = id_dir.join("objects");
    require_plain_dir(&objects_dir, "open stored object directory")?;
    let path = objects_dir.join(content_sha256.as_str());
    let object = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(&path)
        .map_err(|error| capture_io("open stored snapshot object", &error))?;
    if windows_identity::is_reparse_point(&object)
        .map_err(|error| capture_io("scan stored snapshot object", &error))?
    {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    if !object
        .metadata()
        .map_err(|error| capture_io("scan stored snapshot object", &error))?
        .is_file()
    {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(object)
}

#[cfg(windows)]
fn require_plain_dir(path: &std::path::Path, context: &'static str) -> Result<(), SnapshotError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| capture_io(context, &error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    if windows_identity::path_is_reparse_point(path).map_err(|error| capture_io(context, &error))? {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(())
}

/// Materializes one stored object into its target file, refusing oversize
/// counts, oversize totals, short objects, and digest mismatches. On APFS the
/// object is first cloned (copy-on-write), and the clone is re-hashed before
/// it is accepted; where the filesystem refuses a clone, the bytes are
/// streamed as before. Either way the bytes written to the target are the
/// bytes the digest verified.
pub(super) fn copy_entry(
    artifacts: &SnapshotArtifacts,
    snapshot_id: &SnapshotId,
    entry: &CapturedEntry,
    writer: &mut TargetWriter<'_>,
    total: &mut u64,
    cancellation: &CancellationToken,
) -> Result<u64, SnapshotError> {
    let mut object = open_object(artifacts, snapshot_id, entry.content_sha256())?;
    if entry.mode() == SnapshotFileMode::Symlink {
        let target = read_verified_symlink(&mut object, entry, total, cancellation)?;
        validate_symlink_target(entry.path().as_str(), &target)?;
        writer.create_symlink(entry.path(), &target)?;
        return Ok(target.len() as u64);
    }
    #[cfg(target_os = "macos")]
    if let Some(mut clone) = writer.clone_file(entry.path(), &object)? {
        let count = verify_bytes(
            &mut clone,
            None,
            entry,
            total,
            cancellation,
            "read materialized clone",
        )?;
        writer.finish_file(clone, entry.mode())?;
        return Ok(count);
    }
    let mut out = writer.create_file(entry.path(), entry.mode())?;
    let count = verify_bytes(
        &mut object,
        Some(&mut out),
        entry,
        total,
        cancellation,
        "read stored snapshot object",
    )?;
    writer.finish_file(out, entry.mode())?;
    Ok(count)
}

/// Reads one stored symlink object whole: the target is at most 4 KiB, so
/// the verified bytes can be held and passed to `symlinkat`. The same count,
/// total, and digest rules as the streaming path apply.
fn read_verified_symlink(
    reader: &mut impl Read,
    entry: &CapturedEntry,
    total: &mut u64,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>, SnapshotError> {
    let expected = entry.captured_bytes();
    if expected > MAX_SYMLINK_TARGET_BYTES as u64 {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    let mut bytes = Vec::with_capacity(expected as usize);
    let mut hasher = Sha256::new();
    let mut chunk = [0u8; 4096];
    loop {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let read = reader
            .read(&mut chunk)
            .map_err(|error| capture_io("read stored snapshot object", &error))?;
        if read == 0 {
            break;
        }
        if bytes.len().saturating_add(read) > expected as usize {
            return Err(SnapshotError::ObjectDigestMismatch);
        }
        *total = total
            .checked_add(read as u64)
            .ok_or(SnapshotError::TooLarge)?;
        if *total > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        bytes.extend_from_slice(&chunk[..read]);
        hasher.update(&chunk[..read]);
    }
    if bytes.len() as u64 != expected {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    if manifest::digest_from_hasher(hasher)? != *entry.content_sha256() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(bytes)
}

/// Streams one source into the target (when a sink is given) while hashing
/// the same bytes, refusing oversize counts, oversize totals, short objects,
/// and digest mismatches.
fn verify_bytes(
    reader: &mut impl std::io::Read,
    mut sink: Option<&mut fs::File>,
    entry: &CapturedEntry,
    total: &mut u64,
    cancellation: &CancellationToken,
    read_error: &'static str,
) -> Result<u64, SnapshotError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; CHUNK_BYTES];
    let mut count: u64 = 0;
    loop {
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let read = reader
            .read(&mut buffer)
            .map_err(|error| capture_io(read_error, &error))?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or(SnapshotError::ObjectDigestMismatch)?;
        if count > entry.captured_bytes() {
            return Err(SnapshotError::ObjectDigestMismatch);
        }
        *total = (*total)
            .checked_add(read as u64)
            .ok_or(SnapshotError::TooLarge)?;
        if *total > MAX_SNAPSHOT_TOTAL_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        if let Some(out) = sink.as_deref_mut() {
            out.write_all(&buffer[..read])
                .map_err(|error| capture_io("write materialized file", &error))?;
        }
        hasher.update(&buffer[..read]);
    }
    if count != entry.captured_bytes() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    let actual = manifest::digest_from_hasher(hasher)?;
    if actual != *entry.content_sha256() {
        return Err(SnapshotError::ObjectDigestMismatch);
    }
    Ok(count)
}
