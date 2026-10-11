//! Retention is the default: no age, count, or disk pressure deletes user work.
use super::{CleanupError, OwnershipInput};
use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, ManagedCheckoutPath, SnapshotId};

/// Recoverability evidence from current ownership verification. This is not a
/// deletion approval and does not change the checkout's durable lifecycle.
pub struct RetainedCheckout {
    pub checkout_id: ManagedCheckoutId,
    pub path: ManagedCheckoutPath,
    pub snapshot_id: SnapshotId,
}
pub async fn retain_managed_checkout(
    input: &OwnershipInput<'_>,
    cancellation: &CancellationToken,
) -> Result<RetainedCheckout, CleanupError> {
    let record = input
        .store
        .get_managed_checkout(input.job_id, input.checkout_id)
        .await?
        .ok_or(super::OwnershipError::NotFound)?;
    if !matches!(
        record.current_lifecycle,
        saya_types::ManagedCheckoutLifecycle::Active
            | saya_types::ManagedCheckoutLifecycle::Retained
    ) {
        return Err(super::OwnershipError::NotActive.into());
    }
    let verified =
        super::ownership::verify_checkout_state(input, cancellation, record.current_lifecycle)
            .await?;
    let snapshot = input.artifacts.read_snapshot(verified.snapshot_id())?;
    verify_recovery_objects(input.artifacts, &snapshot, cancellation)?;
    Ok(RetainedCheckout {
        checkout_id: verified.checkout_id().clone(),
        path: verified.path().clone(),
        snapshot_id: verified.snapshot_id().clone(),
    })
}

/// Rehash every retained object before deletion; manifest verification alone
/// cannot establish recoverability when an object was lost or corrupted.
pub(super) fn verify_recovery_objects(
    artifacts: &crate::coding::snapshot::SnapshotArtifacts,
    snapshot: &crate::coding::snapshot::CapturedSnapshot,
    cancellation: &CancellationToken,
) -> Result<(), CleanupError> {
    #[cfg(not(unix))]
    {
        let _ = (artifacts, snapshot, cancellation);
        Err(CleanupError::ArtifactUnavailable)
    }
    #[cfg(unix)]
    {
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let workspace = crate::workspace::Workspace::open(artifacts.root())
            .map_err(|_| CleanupError::ArtifactUnavailable)?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        for entry in snapshot.entries() {
            let path = format!(
                "{}/objects/{}",
                snapshot.snapshot_id().as_str(),
                entry.content_sha256().as_str()
            );
            let anchor = workspace
                .anchor(&path, false)
                .map_err(|_| CleanupError::ArtifactUnavailable)?;
            let mut file = anchor
                .open_file(libc::O_RDONLY | libc::O_NONBLOCK, 0)
                .map_err(|_| CleanupError::ArtifactUnavailable)?;
            let metadata = file
                .metadata()
                .map_err(|_| CleanupError::ArtifactUnavailable)?;
            if !metadata.is_file() || metadata.len() != entry.captured_bytes() {
                return Err(CleanupError::ArtifactUnavailable);
            }
            let mut hash = Sha256::new();
            let mut buffer = [0; 65536];
            let mut bytes = 0_u64;
            loop {
                if cancellation.is_cancelled() || std::time::Instant::now() >= deadline {
                    return Err(CleanupError::ArtifactUnavailable);
                }
                let count = file
                    .read(&mut buffer)
                    .map_err(|_| CleanupError::ArtifactUnavailable)?;
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
}
