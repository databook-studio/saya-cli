//! The Windows recoverability proof: every retained object is re-opened
//! through the contained workspace (reparse refusal plus post-open identity
//! check) and re-hashed to its manifest digest.

use std::{
    fs,
    io::{ErrorKind, Read},
    time::{Duration, Instant},
};

use saya_agent::CancellationToken;
use sha2::{Digest, Sha256};

use super::CleanupError;
use crate::coding::snapshot::SnapshotArtifacts;
use crate::workspace::{Workspace, windows_identity};

const RECOVERY_DEADLINE: Duration = Duration::from_secs(10);

pub(super) fn verify_recovery_objects(
    artifacts: &SnapshotArtifacts,
    snapshot: &crate::coding::snapshot::CapturedSnapshot,
    cancellation: &CancellationToken,
) -> Result<(), CleanupError> {
    let workspace =
        Workspace::open(artifacts.root()).map_err(|_| CleanupError::ArtifactUnavailable)?;
    let deadline = Instant::now() + RECOVERY_DEADLINE;
    for entry in snapshot.entries() {
        if cancellation.is_cancelled() || Instant::now() >= deadline {
            return Err(CleanupError::ArtifactUnavailable);
        }
        let path = format!(
            "{}/objects/{}",
            snapshot.snapshot_id().as_str(),
            entry.content_sha256().as_str()
        );
        let target = workspace
            .target(&path, false)
            .map_err(|_| CleanupError::ArtifactUnavailable)?;
        let pre = fs::symlink_metadata(&target).map_err(|_| CleanupError::ArtifactUnavailable)?;
        if pre.file_type().is_symlink()
            || !pre.is_file()
            || pre.len() != entry.captured_bytes()
            || windows_identity::path_is_reparse_point(&target)
                .map_err(|_| CleanupError::ArtifactUnavailable)?
        {
            return Err(CleanupError::ArtifactUnavailable);
        }
        let mut file = workspace
            .open_verified(&target, &pre, &path)
            .map_err(|_| CleanupError::ArtifactUnavailable)?;
        let mut hash = Sha256::new();
        let mut buffer = [0; 65536];
        let mut bytes = 0_u64;
        loop {
            if cancellation.is_cancelled() || Instant::now() >= deadline {
                return Err(CleanupError::ArtifactUnavailable);
            }
            let count = match file.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(_) => return Err(CleanupError::ArtifactUnavailable),
            };
            if count == 0 {
                break;
            }
            bytes += count as u64;
            if bytes > entry.captured_bytes() {
                return Err(CleanupError::ArtifactUnavailable);
            }
            hash.update(&buffer[..count]);
        }
        if bytes != entry.captured_bytes()
            || format!("{:x}", hash.finalize()) != entry.content_sha256().as_str()
        {
            return Err(CleanupError::ArtifactUnavailable);
        }
    }
    Ok(())
}
