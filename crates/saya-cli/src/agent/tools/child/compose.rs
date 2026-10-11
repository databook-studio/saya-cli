//! Use-time composition of one child from verified checkout evidence.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use saya_agent::{AgentLimits, CancellationToken};
use saya_config::{ORCHESTRATOR_ROLE, ResolvedAi};
use saya_harness::{
    coding::{
        child::ChildExecutionSpec,
        context::{ChildBrief, ChildContextError, ChildProjectionOptions, OriginalProjectContext},
        policy::{AuthorityError, ChildAuthorityRequest, RootDelegationScope},
        worktree::{
            ConfinementError, OwnershipError, OwnershipInput, confine_checkout,
            verify_owned_checkout,
        },
    },
    runner::{RunProgram, StaticCredentialSource},
};
use saya_types::{JobId, ManagedCheckoutId, SnapshotId};
use thiserror::Error;

use super::{ChildToolsError, CliChildFactory, TrustedCheckoutPair};
use crate::config::runtime::RuntimeConfig;

#[cfg(test)]
#[path = "tests_compose.rs"]
mod tests;

pub(crate) struct ChildComposition {
    pub(crate) spec: ChildExecutionSpec,
    pub(crate) factory: CliChildFactory,
}

/// Trusted inputs prepared by the coordinator for one child invocation.
pub(crate) struct ChildComposeRequest<'a> {
    pub(crate) ownership: OwnershipInput<'a>,
    pub(crate) cancellation: &'a CancellationToken,
    pub(crate) root: &'a RootDelegationScope,
    pub(crate) authority: ChildAuthorityRequest,
    pub(crate) project: &'a OriginalProjectContext,
    pub(crate) brief: ChildBrief,
    pub(crate) options: ChildProjectionOptions,
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) program_dir: &'a Path,
    /// Existing absolute canonical host directory outside the child checkout.
    /// Callers should canonicalize their trusted path before composing a child.
    pub(crate) trusted_record_dir: &'a Path,
    pub(crate) command_timeout: Duration,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ChildComposeError {
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Confinement(#[from] ConfinementError),
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error(transparent)]
    Context(#[from] ChildContextError),
    #[error(transparent)]
    ChildTools(#[from] ChildToolsError),
    #[error("the orchestrator AI endpoint is unavailable")]
    EndpointUnavailable,
    #[error("the child authority does not match verified checkout ownership")]
    AuthorityScopeMismatch,
    #[error("the trusted child record directory is unavailable")]
    RecordDirectoryUnavailable,
    #[error("the trusted child record directory must be absolute, normalized, and canonical")]
    RecordDirectoryNotCanonical,
    #[error("the trusted child record directory must be outside the checkout")]
    RecordDirectoryInsideCheckout,
}

/// Builds the verified view, bounded child request, confined runner and
/// matching CLI factory in one use-time flow. The record directory must be an
/// existing absolute canonical host directory outside the writable checkout.
pub(crate) async fn compose_child(
    input: ChildComposeRequest<'_>,
) -> Result<ChildComposition, ChildComposeError> {
    let verified = verify_owned_checkout(&input.ownership, input.cancellation).await?;
    let pair = TrustedCheckoutPair::from_verified(&verified)?;
    let confined = confine_checkout(&verified, input.program_dir)?;
    let view = input
        .root
        .narrow(input.authority, confined.platform_support())?;
    validate_view_ownership(
        &view,
        pair.checkout_id(),
        input.ownership.job_id,
        verified.snapshot_id(),
    )?;

    let mut options = input.options;
    options.agent_limits = project_agent_limits(&view, options.agent_limits);
    let spec = input
        .project
        .project_child(view.clone(), input.brief, options)?;

    let has_command_scope =
        view.capabilities().runner.is_some() || view.capabilities().interpreter.is_some();
    let runner = if has_command_scope {
        let record_dir = ensure_record_dir_outside_checkout(
            input.trusted_record_dir,
            Path::new(verified.path().as_str()),
        )?;
        Some(bind_invocation_cancellation(
            confined.runner_tool(
                view.capabilities().runner.clone(),
                view.capabilities().interpreter.clone(),
                input.command_timeout,
                Arc::new(StaticCredentialSource::new([])),
                record_dir,
            ),
            input.cancellation,
        ))
    } else {
        None
    };

    let endpoint = input
        .runtime
        .resolved
        .endpoints
        .get(ORCHESTRATOR_ROLE)
        .ok_or(ChildComposeError::EndpointUnavailable)?;
    let mut ai: ResolvedAi = input.runtime.resolved.ai.clone();
    ai.provider = endpoint.provider;
    ai.model = endpoint.model.clone();
    ai.base_url = endpoint.base_url.clone();
    ai.api_key = endpoint.api_key.clone();
    let factory = CliChildFactory::new(pair, runner, ai, Arc::new(input.runtime.secret_resolver()));

    Ok(ChildComposition { spec, factory })
}

/// The loop's broad write permit lets scoped workspace and command definitions
/// reach their independent child decider and exact admission checks.
pub(super) fn project_agent_limits(
    view: &saya_harness::coding::policy::ChildAuthorityView,
    mut limits: AgentLimits,
) -> AgentLimits {
    limits.permit_workspace_writes = (view.managed_checkout_id().is_some()
        && !view.write_paths().is_empty())
        || view.capabilities().runner.is_some()
        || view.capabilities().interpreter.is_some();
    limits.permit_candidate_writes = false;
    limits.permit_external_effects = false;
    limits
}

fn validate_view_ownership(
    view: &saya_harness::coding::policy::ChildAuthorityView,
    checkout_id: &ManagedCheckoutId,
    job_id: &JobId,
    snapshot_id: &SnapshotId,
) -> Result<(), ChildComposeError> {
    if view.managed_checkout_id() != Some(checkout_id)
        || view.job_id() != job_id
        || view.snapshot_id() != snapshot_id
    {
        return Err(ChildComposeError::AuthorityScopeMismatch);
    }
    Ok(())
}

fn bind_invocation_cancellation(
    runner: RunProgram,
    cancellation: &CancellationToken,
) -> Arc<RunProgram> {
    Arc::new(runner.with_cancellation(cancellation.clone()))
}

fn ensure_record_dir_outside_checkout(
    record_dir: &Path,
    checkout: &Path,
) -> Result<PathBuf, ChildComposeError> {
    if !record_dir.is_absolute() {
        return Err(ChildComposeError::RecordDirectoryNotCanonical);
    }
    let canonical_record_dir = record_dir
        .canonicalize()
        .map_err(|_| ChildComposeError::RecordDirectoryUnavailable)?;
    if canonical_record_dir.as_os_str() != record_dir.as_os_str() {
        return Err(ChildComposeError::RecordDirectoryNotCanonical);
    }
    if !canonical_record_dir.is_dir() {
        return Err(ChildComposeError::RecordDirectoryUnavailable);
    }
    let checkout = checkout
        .canonicalize()
        .map_err(|_| ChildComposeError::RecordDirectoryUnavailable)?;
    if canonical_record_dir.starts_with(checkout) {
        return Err(ChildComposeError::RecordDirectoryInsideCheckout);
    }
    Ok(canonical_record_dir)
}
