use std::{collections::BTreeSet, fmt};

use saya_agent::{AgentLimits, AnsweringLimits, RequestPreflightConfig};
use saya_types::Sha256Digest;

use super::limits::{
    ChildContextError, MAX_CHILD_ARTIFACT_REFERENCES, MAX_CHILD_DEPENDENCIES,
    MAX_CHILD_DEPENDENCY_SUMMARY_BYTES, MAX_CHILD_TASK_BYTES, safe_reference,
};

/// A dependency identity plus its caller-selected, untrusted summary.
#[derive(Clone)]
pub struct ChildDependency {
    pub id: String,
    pub summary: String,
}

impl fmt::Debug for ChildDependency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildDependency")
            .field("id", &"[opaque]")
            .field("summary", &"[redacted]")
            .finish()
    }
}

/// A caller-selected opaque artifact reference; no artifact file is loaded.
#[derive(Clone)]
pub struct ArtifactReference {
    pub id: String,
    pub sha256: Sha256Digest,
}

impl fmt::Debug for ArtifactReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactReference")
            .field("id", &"[opaque]")
            .field("sha256", &self.sha256)
            .finish()
    }
}

/// One selected child task with only its declared dependencies and artifacts.
pub struct ChildBrief {
    pub task: String,
    pub dependencies: Vec<ChildDependency>,
    pub artifacts: Vec<ArtifactReference>,
}

impl fmt::Debug for ChildBrief {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildBrief")
            .field("task", &"[redacted]")
            .field("dependency_count", &self.dependencies.len())
            .field("artifact_count", &self.artifacts.len())
            .finish()
    }
}

/// Caller-resolved model/profile identities and existing finite child controls.
#[derive(Debug, Clone)]
pub struct ChildProjectionOptions {
    pub model: String,
    pub profile_names: Vec<String>,
    pub agent_limits: AgentLimits,
    pub answering_limits: AnsweringLimits,
    pub request_preflight: Option<RequestPreflightConfig>,
}

pub(super) fn validate_brief(brief: &ChildBrief) -> Result<(), ChildContextError> {
    if brief.task.trim().is_empty()
        || brief.task.len() > MAX_CHILD_TASK_BYTES
        || has_disallowed_control(&brief.task)
    {
        return Err(ChildContextError::TaskTooLarge);
    }
    if brief.dependencies.len() > MAX_CHILD_DEPENDENCIES {
        return Err(ChildContextError::TooManyDependencies);
    }
    let mut dependency_ids = BTreeSet::new();
    for dependency in &brief.dependencies {
        if !safe_reference(&dependency.id) || !dependency_ids.insert(&dependency.id) {
            return Err(ChildContextError::InvalidDependency);
        }
        if dependency.summary.len() > MAX_CHILD_DEPENDENCY_SUMMARY_BYTES
            || has_disallowed_control(&dependency.summary)
        {
            return Err(ChildContextError::DependencySummaryTooLarge);
        }
    }
    if brief.artifacts.len() > MAX_CHILD_ARTIFACT_REFERENCES {
        return Err(ChildContextError::TooManyArtifacts);
    }
    let mut artifact_ids = BTreeSet::new();
    for artifact in &brief.artifacts {
        if !safe_reference(&artifact.id) || !artifact_ids.insert(&artifact.id) {
            return Err(ChildContextError::InvalidArtifactReference);
        }
    }
    Ok(())
}

fn has_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
}
