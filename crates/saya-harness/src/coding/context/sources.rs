use std::{fmt, path::Path};

use saya_types::{ProjectPreference, Sha256Digest, WorktreeId};

use crate::instructions::InstructionLimits;
use crate::{instructions, workspace::Workspace};

use super::limits::{
    ChildContextError, MAX_CONTEXT_SOURCE_FACTS, MAX_INSTRUCTION_BYTES,
    MAX_PERSONAL_INSTRUCTION_BYTES, MAX_PROJECT_INSTRUCTION_BYTES,
};
use super::preferences::validate_preferences;

/// Typed identity and admission facts for one captured instruction source.
pub struct InstructionSourceFact {
    pub origin: instructions::InstructionOrigin,
    pub file_name: &'static str,
    pub source_size: u64,
    /// Hash of the complete source file, including when the captured body is truncated.
    pub source_sha256: Sha256Digest,
    pub source_truncated: bool,
    pub(super) body: String,
}

impl fmt::Debug for InstructionSourceFact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InstructionSourceFact")
            .field("origin", &self.origin)
            .field("file_name", &self.file_name)
            .field("source_size", &self.source_size)
            .field("source_sha256", &self.source_sha256)
            .field("source_truncated", &self.source_truncated)
            .field("body", &"[redacted]")
            .finish()
    }
}

/// Immutable original-project facts captured once before child projection.
pub struct OriginalProjectContext {
    pub(super) worktree_id: WorktreeId,
    pub(super) instruction_sources: Vec<InstructionSourceFact>,
    pub(super) skipped_sources: Vec<instructions::SkippedSource>,
    pub(super) preferences: Vec<ProjectPreference>,
}

impl fmt::Debug for OriginalProjectContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OriginalProjectContext")
            .field("worktree_id", &self.worktree_id)
            .field("instruction_source_count", &self.instruction_sources.len())
            .field("skipped_source_count", &self.skipped_sources.len())
            .field("preference_count", &self.preferences.len())
            .finish()
    }
}

impl OriginalProjectContext {
    /// Captures fixed instruction files and already-fetched explicit preferences.
    /// The caller must supply the original workspace; this operation never
    /// infers a root, reads a child checkout, or fetches additional state.
    pub fn capture(
        original_workspace: &Workspace,
        personal_instruction_dir: Option<&Path>,
        preferences: Vec<ProjectPreference>,
    ) -> Result<Self, ChildContextError> {
        let limits = InstructionLimits {
            personal_max_bytes: MAX_PERSONAL_INSTRUCTION_BYTES,
            project_max_bytes: MAX_PROJECT_INSTRUCTION_BYTES,
            total_max_bytes: MAX_INSTRUCTION_BYTES,
        };
        Self::capture_with_policy(
            original_workspace,
            personal_instruction_dir,
            preferences,
            true,
            &limits,
        )
    }

    /// Captures original-project facts using explicit, bounded instruction policy.
    /// Disabled project instructions are omitted at the loader boundary and are
    /// never opened. Supplied limits may lower but cannot exceed context ceilings.
    pub fn capture_with_policy(
        original_workspace: &Workspace,
        personal_instruction_dir: Option<&Path>,
        mut preferences: Vec<ProjectPreference>,
        project_enabled: bool,
        limits: &InstructionLimits,
    ) -> Result<Self, ChildContextError> {
        if limits.personal_max_bytes > MAX_PERSONAL_INSTRUCTION_BYTES
            || limits.project_max_bytes > MAX_PROJECT_INSTRUCTION_BYTES
            || limits.total_max_bytes > MAX_INSTRUCTION_BYTES
        {
            return Err(ChildContextError::InvalidInstructionLimits);
        }
        let canonical_path = original_workspace
            .root()
            .to_str()
            .ok_or(ChildContextError::OriginalProjectPathNotUtf8)?;
        let worktree_id = WorktreeId::from_canonical_path(canonical_path)
            .map_err(|_| ChildContextError::OriginalProjectIdentity)?;
        validate_preferences(&preferences, &worktree_id)?;
        preferences.sort_by(|left, right| left.id.as_str().cmp(right.id.as_str()));

        let loaded = instructions::load_instructions(
            project_enabled.then_some(original_workspace.root()),
            personal_instruction_dir,
            limits,
        );
        let instruction_sources = loaded
            .sources
            .into_iter()
            .map(|source| {
                let source_sha256 = Sha256Digest::parse(&source.sha256)
                    .map_err(|_| ChildContextError::InvalidInstructionDigest)?;
                Ok(InstructionSourceFact {
                    origin: source.origin,
                    file_name: source.file_name,
                    source_size: source.size,
                    source_sha256,
                    source_truncated: source.truncated,
                    body: source.body,
                })
            })
            .collect::<Result<Vec<_>, ChildContextError>>()?;
        let facts = instruction_sources
            .len()
            .checked_add(loaded.skipped.len())
            .and_then(|count| count.checked_add(preferences.len()))
            .ok_or(ChildContextError::TooManySourceFacts)?;
        if facts > MAX_CONTEXT_SOURCE_FACTS {
            return Err(ChildContextError::TooManySourceFacts);
        }

        Ok(Self {
            worktree_id,
            instruction_sources,
            skipped_sources: loaded.skipped,
            preferences,
        })
    }

    pub fn worktree_id(&self) -> &WorktreeId {
        &self.worktree_id
    }

    pub fn instruction_sources(&self) -> &[InstructionSourceFact] {
        &self.instruction_sources
    }

    pub fn skipped_sources(&self) -> &[instructions::SkippedSource] {
        &self.skipped_sources
    }

    pub fn preference_count(&self) -> usize {
        self.preferences.len()
    }
}
