//! Use-time composition of one child from verified checkout evidence.

use std::{path::Path, sync::Arc};

use saya_agent::{AgentLimits, ApprovalPolicy, CancellationToken};
use saya_config::{ORCHESTRATOR_ROLE, ResolvedAi};
use saya_harness::coding::{
    child::ChildExecutionSpec,
    context::{ChildBrief, ChildContextError, ChildProjectionOptions, OriginalProjectContext},
    integration::AssignedPaths,
    policy::{AuthorityError, ChildAuthorityRequest, PlatformSupport, RootDelegationScope},
    worktree::{
        ChildEnvironment, OwnershipError, ScratchBase, ScratchScope, verify_in_place_root,
        verify_owned_checkout,
    },
};
use saya_types::{CodingBudgetMode, JobId, ManagedCheckoutId, SnapshotId};
use thiserror::Error;

use super::{CheckoutProof, ChildToolsError, CliChildFactory, TrustedCheckoutPair};
use crate::agent::tools::consent::{Caller, Operator};
use crate::agent::tools::shell_policy::{self, Background, ShellPlacement};
use crate::config::runtime::RuntimeConfig;

#[cfg(test)]
#[path = "tests_compose.rs"]
mod tests;

pub(crate) struct ChildComposition {
    pub(crate) spec: ChildExecutionSpec,
    pub(crate) factory: CliChildFactory,
}

/// The helper's `shell`, as the job layer states it: the mode the shell reads
/// and the operator that answers a host call. `None` on the request means this
/// helper gets no shell at all — the config is off, the job's mode is `never`,
/// or the helper is one the job gives no shell.
///
/// The mode is the job's, except for a read-only (`explore`) helper, whose
/// shell is sandbox-only with nothing writable whatever the job's mode:
/// the job layer states that by passing `ReadOnly`.
#[derive(Clone)]
pub(crate) struct HelperShell {
    pub(crate) policy: ApprovalPolicy,
    pub(crate) operator: Option<Arc<dyn Operator>>,
    /// The job's private state directory, where a result too large for the
    /// model's inline bound saves its full captured output.
    pub(crate) spool: Option<std::path::PathBuf>,
}

/// The helper's web lane, as the job layer states it: the mode the tools read,
/// the operator that answers an `ask`, and the job's own egress. `None` on the
/// request means this helper gets no web tool at all — the job's mode reaches
/// no host, or no wire was composed.
///
/// Unlike the shell, there is no per-helper variant: a fetch is the same read
/// whoever makes it, and a search is the same query, so the job's mode is the
/// whole of it. Whether the lane also holds `web_search` is the egress's own
/// answer — the user's `[web_search]` — and not a per-helper decision.
pub(crate) struct HelperWeb {
    pub(crate) policy: ApprovalPolicy,
    pub(crate) operator: Option<Arc<dyn Operator>>,
    pub(crate) egress: Arc<crate::agent::tools::web::WebEgress>,
}

/// Trusted inputs prepared by the coordinator for one child invocation.
pub(crate) struct ChildComposeRequest<'a> {
    /// The shared root's proof, re-verified at use time.
    pub(crate) proof: CheckoutProof<'a>,
    /// The durable job the helper acts for; its authority view names it.
    pub(crate) job_id: &'a JobId,
    pub(crate) cancellation: &'a CancellationToken,
    /// The in-place job's approved `--write` scope, enforced on every write
    /// door this helper composes. `None` is a managed checkout.
    pub(crate) write_scope: Option<AssignedPaths>,
    pub(crate) root: &'a RootDelegationScope,
    pub(crate) authority: ChildAuthorityRequest,
    pub(crate) project: &'a OriginalProjectContext,
    pub(crate) brief: ChildBrief,
    pub(crate) options: ChildProjectionOptions,
    pub(crate) runtime: &'a RuntimeConfig,
    pub(crate) program_dir: &'a Path,
    /// The helper's `shell` lane, when the job allows one. Built here because
    /// the confinement below is what prepares the helper's scratch tree.
    pub(crate) shell: Option<HelperShell>,
    /// The helper's `web_fetch`, when the job's mode reaches the network.
    pub(crate) web: Option<HelperWeb>,
    /// The helper's own MCP lane, already started by the job layer under the
    /// job's mode and operator. `None` where the mode reaches no server or
    /// none is configured. The lane's lifetime is the helper's: it is dropped,
    /// and its servers closed, when this child ends.
    pub(crate) mcp: Option<Arc<crate::mcp::client::McpTools>>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum ChildComposeError {
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error(transparent)]
    Context(#[from] ChildContextError),
    #[error(transparent)]
    ChildTools(#[from] ChildToolsError),
    #[error("the orchestrator AI endpoint is unavailable")]
    EndpointUnavailable,
    #[error("the child request preflight configuration is invalid")]
    InvalidRequestPreflight,
    #[error("the child authority does not match verified checkout ownership")]
    AuthorityScopeMismatch,
}

/// Builds the verified view, bounded child request and matching CLI factory in
/// one use-time flow.
pub(crate) async fn compose_child(
    input: ChildComposeRequest<'_>,
) -> Result<ChildComposition, ChildComposeError> {
    let verified = match &input.proof {
        CheckoutProof::Managed(ownership) => {
            verify_owned_checkout(ownership, input.cancellation).await?
        }
        CheckoutProof::InPlace(in_place) => verify_in_place_root(in_place)?,
    };
    let pair = TrustedCheckoutPair::from_verified(&verified)?;
    // The helper's workspace tools spawn no command: their writes are admitted
    // in-process against this verified checkout, so composition runs no
    // sandbox probe. A spawned command's confinement is proven where the
    // helper's shell is composed; that shell is absent when the host cannot
    // prove one.
    let mut platform = PlatformSupport::workspace_tools();
    // The database tools independently check each consent-bound identity.
    platform.database_reads = !input.options.profile_names.is_empty();
    let view = input.root.narrow(input.authority, platform)?;
    validate_view_ownership(
        &view,
        pair.checkout_id(),
        input.job_id,
        verified.snapshot_id(),
    )?;

    // The helper's shell and workspace tools share one read-witness store, so
    // a shell view of a file guards an edit exactly as a `read` does. The
    // spool directory is derived from the same job state directory the shell
    // saves into, so `read`/`grep` reach exactly this job's saved output.
    let read_witnesses = Arc::new(crate::agent::tools::workspace::ReadWitnesses::default());
    let spool_dir = input
        .shell
        .as_ref()
        .and_then(|request| request.spool.as_deref())
        .map(shell_policy::spool_dir);
    // The helper's `shell`: the same tool the root advertises, over the same
    // shared working checkout, with this helper's own scratch tree and a
    // subject line that names it. The tree is prepared on this path, the only
    // one that can spawn a command. Built before the limits are projected,
    // because a helper with a shell must reach the shell's own lane decision
    // rather than the loop's workspace-write gate.
    let shell = input.shell.and_then(|request| {
        let prompt = request.operator.map(|operator| {
            shell_policy::prompt(
                operator,
                Caller::Helper {
                    agent_id: input.root.agent_id().clone(),
                    job_id: input.job_id.clone(),
                },
            )
        });
        // The helper's commands get their own scratch tree: helpers share one
        // working checkout, so they must not share a HOME or a TMPDIR. It is
        // prepared here, on the one path that can spawn a command.
        let base = if verified.base().is_in_place() {
            ScratchBase::InPlace
        } else {
            ScratchBase::Checkout
        };
        let environment = ChildEnvironment::prepare(
            Path::new(verified.path().as_str()),
            input.program_dir,
            base,
            ScratchScope::Helper(input.root.agent_id()),
        )
        .ok()?;
        let placement = ShellPlacement::helper(
            Path::new(verified.path().as_str()),
            (environment.home(), environment.tmpdir()),
        );
        let placement = match request.spool.as_deref() {
            Some(spool) => placement.with_spool(spool),
            None => placement,
        };
        shell_policy::compose(
            placement,
            request.policy,
            prompt,
            crate::agent::tools::coding::shell_secrets::host_policy(input.runtime),
            input.cancellation.clone(),
            // A helper's shell lives for the helper, so it owns the background
            // runs it starts: they end when the helper does.
            Background::Owned,
            input.runtime.resolved.permissions.rules.clone(),
        )
        .map(|tool| {
            Arc::new(tool.with_read_witnesses(Arc::clone(&read_witnesses), Arc::clone(&pair.root)))
        })
    });

    // The helper's web lane: the job's own egress, put to the operator under a
    // subject line that names this helper, exactly as its shell is.
    let web = input.web.and_then(|request| {
        crate::agent::tools::web::WebTools::compose(
            &request.egress,
            request.policy,
            request.operator,
            &Caller::Helper {
                agent_id: input.root.agent_id().clone(),
                job_id: input.job_id.clone(),
            },
            input.cancellation.clone(),
        )
        .map(Arc::new)
    });

    let mut options = input.options;
    options.agent_limits = project_agent_limits(&view, shell.is_some(), options.agent_limits);
    let endpoint = input
        .runtime
        .resolved
        .endpoints
        .get(ORCHESTRATOR_ROLE)
        .ok_or(ChildComposeError::EndpointUnavailable)?;
    let mut ai: ResolvedAi = input.runtime.resolved.ai.clone();
    ai.provider = endpoint.provider;
    // The factory uses this exact request model, including trusted caller overrides.
    ai.model = options.model.clone();
    ai.base_url = endpoint.base_url.clone();
    ai.api_key = endpoint.api_key.clone();
    if options.request_preflight.is_none() {
        options.request_preflight = Some(
            crate::agent::preflight_config::request_preflight_config(&ai)
                .map_err(|_| ChildComposeError::InvalidRequestPreflight)?,
        );
    }
    let spec = if crate::commands::coding::limits::mode(&input.runtime.resolved)
        == CodingBudgetMode::Uncapped
    {
        input
            .project
            .project_uncapped_child(view.clone(), input.brief, options)?
    } else {
        input
            .project
            .project_child(view.clone(), input.brief, options)?
    };

    // The job's hooks, over the same working checkout the helper's tools work
    // in: a helper's calls are the job's calls, so a `PreToolUse` block covers
    // them, and the payload names both the job and the helper. The mode is
    // `bypass` for the same reason the root's is: a coding job decides its own
    // calls through its own decider and has no approval mode of its own.
    let hooks = crate::agent::hooks::Hooks::compose(
        input.runtime,
        ApprovalPolicy::Bypass,
        Path::new(verified.path().as_str()),
        crate::agent::hooks::HookCaller::Helper {
            agent_id: input.root.agent_id().as_str().to_owned(),
            job_id: input.job_id.as_str().to_owned(),
        },
        input.cancellation.clone(),
    );

    let factory = CliChildFactory::new(pair, ai, Arc::new(input.runtime.secret_resolver()))
        .with_shell(shell)
        .with_read_witnesses(read_witnesses)
        .with_spool_dir(spool_dir)
        .with_web(web)
        .with_mcp(input.mcp)
        .with_hooks(hooks)
        .with_write_scope(input.write_scope);

    Ok(ChildComposition { spec, factory })
}

/// The loop's broad write permit lets scoped workspace and command definitions
/// reach their independent child decider and exact admission checks — the
/// helper's `shell` included, whose lane decision (host, sandbox, or refusal)
/// is the shell policy's, not the loop's.
pub(super) fn project_agent_limits(
    view: &saya_harness::coding::policy::ChildAuthorityView,
    has_shell: bool,
    mut limits: AgentLimits,
) -> AgentLimits {
    limits.permit_workspace_writes = view.workspace_write() || has_shell;
    limits.permit_candidate_writes = false;
    limits.permit_external_effects = false;
    limits
}

pub(crate) fn validate_view_ownership(
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
