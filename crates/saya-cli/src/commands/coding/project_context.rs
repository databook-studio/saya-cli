use super::{CodingApplication, CodingError};
use saya_harness::{coding::context::OriginalProjectContext, workspace::Workspace};
use saya_store::ProjectPreferenceStore;
use saya_types::WorktreeId;

impl CodingApplication {
    pub(super) async fn capture_original_project_context(
        &self,
        workspace: &Workspace,
    ) -> Result<OriginalProjectContext, CodingError> {
        let canonical_path = workspace
            .root()
            .to_str()
            .ok_or(CodingError::InvalidRequest)?;
        let worktree =
            WorktreeId::from_canonical_path(canonical_path).map_err(CodingError::owner)?;
        let preferences = self
            .store
            .list_project_preferences(&worktree)
            .await
            .map_err(CodingError::owner)?;
        OriginalProjectContext::capture(workspace, None, preferences).map_err(CodingError::owner)
    }
}
