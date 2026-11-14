use std::{collections::BTreeSet, fmt};

use saya_agent::{AgentLimits, AnsweringLimits, RequestPreflightConfig};
use saya_types::Sha256Digest;

use super::limits::{
    ChildContextError, MAX_CHILD_ARTIFACT_REFERENCES, MAX_CHILD_DEPENDENCIES,
    MAX_CHILD_DEPENDENCY_SUMMARY_BYTES, MAX_CHILD_OPERATOR_REQUEST_BYTES, MAX_CHILD_TASK_BYTES,
    safe_reference,
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
    /// The operator's original request, delivered verbatim as required context
    /// when present. `None` keeps the assignment-only projection.
    pub operator_request: Option<String>,
    pub task: String,
    pub dependencies: Vec<ChildDependency>,
    pub artifacts: Vec<ArtifactReference>,
}

impl fmt::Debug for ChildBrief {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChildBrief")
            .field(
                "operator_request",
                &self.operator_request.as_ref().map(|_| "[redacted]"),
            )
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
    if let Some(request) = &brief.operator_request
        && (request.trim().is_empty()
            || request.len() > MAX_CHILD_OPERATOR_REQUEST_BYTES
            || has_disallowed_control(request))
    {
        return Err(ChildContextError::InvalidOperatorRequest);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn brief(task: String) -> ChildBrief {
        ChildBrief {
            operator_request: None,
            task,
            dependencies: Vec::new(),
            artifacts: Vec::new(),
        }
    }

    /// The largest assignment the `helpers` tool admits: a prompt up to
    /// 32 KiB (`MAX_HELPER_ASSIGNMENT_BYTES` in `saya-cli`), a description up
    /// to 128 bytes, and the blank separator line. Every admitted assignment
    /// must compose, so the child task bound must cover it.
    #[test]
    fn the_child_task_bound_covers_the_largest_admitted_helper_assignment() {
        const HELPERS_PROMPT_BYTES: usize = 32 * 1024;
        const HELPERS_DESCRIPTION_BYTES: usize = 128;
        let maximal = HELPERS_PROMPT_BYTES + HELPERS_DESCRIPTION_BYTES + 2;
        assert!(
            validate_brief(&brief("t".repeat(maximal))).is_ok(),
            "an assignment the helpers tool admits must compose"
        );
        assert!(matches!(
            validate_brief(&brief("t".repeat(maximal + 1))),
            Err(ChildContextError::TaskTooLarge)
        ));
    }
}
