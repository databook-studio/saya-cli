use crate::{AgentsFile, ConfigError};

use super::limits::{MAX_CHILDREN, MAX_CONCURRENT_MODEL_REQUESTS};
use super::registry::HelperAgents;
use crate::bounds::require;

/// Effective trusted `[agents]` settings: the two bounds a job's helper
/// scheduler runs under, and the helper type registry its `helpers` tool
/// advertises. These settings shape admission only; they never grant model,
/// filesystem, process, network, or database authority.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAgents {
    /// The most helper children one job runs at once.
    pub max_children: usize,
    /// The most model requests in flight across the job and its helpers.
    pub max_concurrent_model_requests: usize,
    /// The helper types the job advertises: the two built-ins, plus whatever
    /// trusted `[agents]` added and narrowed. A type is a shape, never a grant —
    /// the harness narrows every child against the job's own scope whatever the
    /// registry says.
    pub types: HelperAgents,
}

impl Default for ResolvedAgents {
    fn default() -> Self {
        Self {
            max_children: MAX_CHILDREN,
            max_concurrent_model_requests: MAX_CONCURRENT_MODEL_REQUESTS,
            types: HelperAgents::builtin(),
        }
    }
}

pub(crate) fn resolve(file: &AgentsFile) -> Result<ResolvedAgents, ConfigError> {
    let defaults = ResolvedAgents::default();
    let max_children = file.max_children.unwrap_or(defaults.max_children);
    let max_concurrent_model_requests = file
        .max_concurrent_model_requests
        .unwrap_or(defaults.max_concurrent_model_requests);
    require(max_children, "agents.max_children", MAX_CHILDREN)?;
    if !(2..=MAX_CONCURRENT_MODEL_REQUESTS).contains(&max_concurrent_model_requests) {
        return Err(ConfigError::SettingOutOfRange {
            field: "agents.max_concurrent_model_requests",
            value: max_concurrent_model_requests,
            min: 2,
            max: MAX_CONCURRENT_MODEL_REQUESTS,
        });
    }
    Ok(ResolvedAgents {
        max_children,
        max_concurrent_model_requests,
        types: super::registry::resolve(&file.types)?,
    })
}

#[cfg(test)]
#[path = "resolve_tests.rs"]
mod tests;
