use saya_agent::CancellationToken;
use saya_types::GitObjectId;
use tokio::time::Instant;

use super::{
    command::{GitOperation, GitRepository, require_success},
    inventory_deadline::InventoryDeadline,
    repository::GitInventoryError,
    request::GitRequest,
    service::GitService,
};

impl GitService {
    /// Reads one planned HEAD blob through the shared typed process route,
    /// under the same operation guard, deadline, and cancellation owner as
    /// `read_snapshot_inputs_until`. Residual: the process owner buffers
    /// stdout in memory, so one blob (at most 64 MiB plus slack) is a
    /// bounded transient allocation held for the duration of the call.
    pub(crate) async fn read_snapshot_blob(
        &self,
        repository: &GitRepository,
        object: &GitObjectId,
        cancellation: &CancellationToken,
        deadline_at: Instant,
    ) -> Result<Vec<u8>, GitInventoryError> {
        let _guard = self
            .operation_guard_until(cancellation, deadline_at)
            .await
            .map_err(GitInventoryError::Git)?;
        InventoryDeadline::until(deadline_at).check()?;
        let output = self
            .run_request_until(
                repository.root(),
                GitRequest::SnapshotBlob(object),
                cancellation,
                deadline_at,
            )
            .await
            .map_err(GitInventoryError::Git)?;
        let output = require_success(output, GitOperation::Read).map_err(GitInventoryError::Git)?;
        Ok(output.stdout)
    }
}
