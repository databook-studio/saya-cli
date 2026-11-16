//! Local capture: an edit-time baseline publishes an empty snapshot through
//! the same orchestrator a Git snapshot uses, so the stored manifest, object
//! layout, and bounds are identical. Nothing is read from the root: the
//! marker holds no inventory.

use std::path::Path;

use saya_agent::CancellationToken;

use crate::coding::snapshot::{
    CapturePlan, CaptureSource, CapturedSnapshot, SnapshotArtifacts, SnapshotError, capture_entries,
};
use crate::workspace::Workspace;

use super::plan::LocalBaseline;

impl LocalBaseline {
    /// Publishes this baseline as a complete, empty snapshot in the private
    /// artifacts root. The baseline's own manifest digest is the capture's
    /// plan digest, so the root verification still binds the two.
    pub async fn capture(
        &self,
        artifacts: &SnapshotArtifacts,
        cancellation: &CancellationToken,
    ) -> Result<CapturedSnapshot, SnapshotError> {
        let workspace = Workspace::open(Path::new(self.root_path()))
            .map_err(SnapshotError::from_workspace_error)?;
        capture_entries(
            &workspace,
            CapturePlan {
                snapshot_id: self.snapshot_id(),
                repository_id: self.repository_id(),
                plan_manifest_sha256: self.manifest_sha256(),
                entries: &[],
            },
            CaptureSource::WorkingTree,
            artifacts,
            cancellation,
        )
        .await
    }
}
