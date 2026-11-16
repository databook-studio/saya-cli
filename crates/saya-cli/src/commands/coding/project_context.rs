use super::{CodingApplication, CodingError};
use saya_harness::{
    coding::context::{
        MAX_INSTRUCTION_BYTES, MAX_PERSONAL_INSTRUCTION_BYTES, MAX_PROJECT_INSTRUCTION_BYTES,
        OriginalProjectContext,
    },
    instructions::InstructionLimits,
    workspace::Workspace,
};
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
        let config = self.runtime.resolved.instructions;
        let limits = InstructionLimits {
            personal_max_bytes: config
                .personal_max_bytes
                .min(MAX_PERSONAL_INSTRUCTION_BYTES),
            project_max_bytes: config.project_max_bytes.min(MAX_PROJECT_INSTRUCTION_BYTES),
            total_max_bytes: config.total_max_bytes.min(MAX_INSTRUCTION_BYTES),
        };
        let personal_instruction_dir = if config.personal_enabled {
            self.personal_instruction_dir.as_deref()
        } else {
            None
        };
        OriginalProjectContext::capture_with_policy(
            workspace,
            personal_instruction_dir,
            preferences,
            config.project_enabled,
            &limits,
        )
        .map_err(CodingError::owner)
    }
}
