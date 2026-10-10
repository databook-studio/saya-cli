//! The capture orchestrator (W04d): the single public producer composing
//! the accepted W04a staging destination, W04b working-tree reader, and
//! W04c HEAD-blob source into `GitService::capture_snapshot`. Plan entries
//! are captured in plan order into the staging directory; only after every
//! entry verifies does `commit` publish the final directory atomically. On
//! error or cancellation the staging is dropped — its `Drop` removes it;
//! the error contract, including the post-publish `PublishedButNotDurable`
//! outcome, is documented on `capture_snapshot`.
//!
//! Ownership residual: the captured-manifest digest binds captured
//! content, never path or authorship. It is NOT an ownership proof: W05
//! must bind `(snapshot_id, captured_manifest_sha256)` and never treat the
//! plan digest, a path hash, or this digest alone as ownership
//! (W04-PREFLIGHT.md §6.1).
//!
//! Send: the returned future is NOT guaranteed `Send`. The only await is
//! the GitBlob branch's `capture_head_blob` call, and the borrowed
//! `&mut dyn Write` sink passed to it is held across its awaits; a `&mut
//! dyn Write` trait object is not `Send`, so the composed future is not
//! `Send` and callers must await the capture on one task (no spawn). No
//! unsafe and no seam is added for this.

use saya_agent::CancellationToken;
use saya_types::{RepositoryId, Sha256Digest, SnapshotId};
use serde::Serialize;

use crate::coding::git::{GitRepository, GitService};
use crate::workspace::Workspace;

use super::{
    SnapshotArtifacts, SnapshotManifest,
    capture_blob::capture_head_blob,
    capture_reader::read_working_file,
    capture_types::{CapturedEntry, CapturedSnapshot},
    manifest::{RedactedIoError, SnapshotError, digest},
    types::{MAX_SNAPSHOT_MANIFEST_BYTES, MAX_SNAPSHOT_TOTAL_BYTES, SnapshotContentSource},
};

/// The captured manifest body, in this fixed field order: serializing a
/// struct (never a map or `serde_json::Value`) keeps the stored bytes
/// canonical for a given capture, so the stored digest is reproducible.
/// Entries reuse [`CapturedEntry`], whose field order is the same
/// contract.
#[derive(Serialize)]
struct CapturedManifestBody<'a> {
    schema_version: u32,
    snapshot_id: &'a SnapshotId,
    repository_id: &'a RepositoryId,
    plan_manifest_sha256: &'a Sha256Digest,
    entries: &'a [CapturedEntry],
}

impl GitService {
    /// Captures one planned snapshot into the private artifacts root:
    /// `begin` opens a staging directory, then each plan entry in order is
    /// read through the working-tree reader (same-open, identity
    /// revalidated) or the HEAD-blob source into one staged object under
    /// the shrinking aggregate budget, then the captured manifest is
    /// serialized in canonical field order, size-checked against
    /// `MAX_SNAPSHOT_MANIFEST_BYTES`, and `commit` publishes the final
    /// directory atomically.
    ///
    /// Error contract: any error other than `PublishedButNotDurable` leaves
    /// no complete snapshot — staging is removed by drop on every error or
    /// cancellation. `PublishedButNotDurable` means the id is occupied: the
    /// snapshot directory was published under it and a durability sync
    /// failed afterwards. `SnapshotExists` means another snapshot owns the
    /// id.
    ///
    /// The aggregate-budget check is defence in depth: the plan already
    /// bounds the total.
    ///
    /// The returned `captured_manifest_sha256` digests the manifest bytes
    /// stored beside the objects; it is distinct from the plan digest, and
    /// per the module docs it is not an ownership proof on its own.
    ///
    /// Non-unix hosts refuse with the same typed unix-host error as the
    /// sibling reader and blob source; nothing is staged there.
    pub async fn capture_snapshot(
        &self,
        repository: &GitRepository,
        workspace: &Workspace,
        manifest: &SnapshotManifest,
        artifacts: &SnapshotArtifacts,
        cancellation: &CancellationToken,
    ) -> Result<CapturedSnapshot, SnapshotError> {
        if !cfg!(unix) {
            let _ = (
                self,
                repository,
                workspace,
                manifest,
                artifacts,
                cancellation,
            );
            return Err(SnapshotError::CaptureIo {
                description: "snapshot capture requires a unix host".to_owned(),
                source: Some(RedactedIoError::new(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "non-unix host",
                ))),
            });
        }
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        let mut staged = artifacts.begin(manifest.snapshot_id())?;
        let mut captured_entries = Vec::with_capacity(manifest.entries().len());
        let mut remaining_aggregate = MAX_SNAPSHOT_TOTAL_BYTES;
        for entry in manifest.entries() {
            if cancellation.is_cancelled() {
                return Err(SnapshotError::Cancelled);
            }
            let mut sink = staged.new_object()?;
            let captured = if matches!(entry.source(), SnapshotContentSource::WorkingTree) {
                read_working_file(
                    workspace,
                    entry,
                    remaining_aggregate,
                    &mut sink,
                    cancellation,
                )?
            } else {
                capture_head_blob(
                    self,
                    repository,
                    entry,
                    remaining_aggregate,
                    &mut sink,
                    cancellation,
                )
                .await?
            };
            sink.finish(captured.sha256())?;
            remaining_aggregate = remaining_aggregate
                .checked_sub(captured.bytes())
                .ok_or(SnapshotError::TooLarge)?;
            captured_entries.push(CapturedEntry {
                path: entry.path().clone(),
                mode: entry.mode(),
                content_sha256: captured.sha256().clone(),
                captured_bytes: captured.bytes(),
                source: entry.source().clone(),
            });
        }
        let bytes = serde_json::to_vec(&CapturedManifestBody {
            schema_version: 1,
            snapshot_id: manifest.snapshot_id(),
            repository_id: manifest.repository_id(),
            plan_manifest_sha256: manifest.manifest_sha256(),
            entries: &captured_entries,
        })
        .map_err(|_| SnapshotError::Serialization)?;
        if bytes.len() > MAX_SNAPSHOT_MANIFEST_BYTES {
            return Err(SnapshotError::TooLarge);
        }
        let captured_manifest_sha256 = digest(&bytes)?;
        if cancellation.is_cancelled() {
            return Err(SnapshotError::Cancelled);
        }
        staged.commit(&bytes, &captured_manifest_sha256)?;
        Ok(CapturedSnapshot {
            snapshot_id: manifest.snapshot_id().clone(),
            repository_id: manifest.repository_id().clone(),
            plan_manifest_sha256: manifest.manifest_sha256().clone(),
            captured_manifest_sha256,
            entries: captured_entries,
        })
    }
}
