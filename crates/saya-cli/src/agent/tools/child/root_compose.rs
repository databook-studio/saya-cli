//! The direct-mode root's workspace toolset: composed over the agent-less
//! working checkout W from the SAME authority path a child uses, so the root
//! and its children advertise the same workspace tools. Nothing here is wired
//! into the root loop yet.

use std::{path::PathBuf, sync::Arc};

use saya_agent::{CancellationToken, ToolDefinition};
use saya_config::ResolvedAi;
use saya_harness::coding::child::ChildFactoryError;
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::policy::{
    AuthorityError, ChildAuthorityRequest, ChildAuthorityView, PlatformSupport,
    RootDelegationInput, RootDelegationScope,
};
use saya_harness::coding::worktree::{OwnershipError, VerifiedCheckout, verify_in_place_root};
use saya_types::{JobId, SnapshotId};
use thiserror::Error;

use super::compose::validate_view_ownership;
use super::decider::ExactWriteChildApproval;
use super::executor::ChildTools;
use super::{CheckoutProof, ChildToolsError, CliChildFactory, TrustedCheckoutPair};
use crate::commands::coding::checks::ChildCodingOperations;
use crate::commands::coding::verify_root_checkout;
use crate::config::runtime::RuntimeConfig;

#[cfg(test)]
#[path = "tests_root_compose.rs"]
mod tests;

/// The root's composed workspace toolset: the definitions the loop advertises,
/// the executor bound to W, and the decider that admits writes to the ceiling
/// before execution. Built only for a direct-mode job with a non-empty ceiling.
pub(crate) struct RootWorkspace {
    pub(crate) definitions: Vec<ToolDefinition>,
    pub(crate) tools: ChildTools,
    pub(crate) approval: ExactWriteChildApproval,
    /// The verified working checkout's own root path: the filesystem root the
    /// root's sandboxed `shell` is confined to.
    pub(crate) checkout_root: PathBuf,
    /// The use-time proof this composition verified W with. The caller takes
    /// it so the same checkout is not inventoried a second time moments later;
    /// it is still a use-time verification, not a cached claim. Shared with
    /// the readiness seed, which retires it on the first root tool attempt.
    pub(crate) verified: Arc<VerifiedCheckout>,
}

/// Builds the root's coding operations (commands, checks) from the narrowed
/// authority view: type-erased, `Send`, borrowing for the compose request's
/// lifetime `'a`.
pub(crate) type CodingOpsFactory<'a> =
    Box<dyn FnOnce(&ChildAuthorityView) -> Arc<ChildCodingOperations> + Send + 'a>;

/// Trusted inputs the direct-mode coordinator prepares for its own workspace
/// toolset over the agent-less working checkout W.
pub(crate) struct RootComposeRequest<'a> {
    /// W's proof, re-verified at use time (a managed checkout must be Active
    /// and agent-less; an in-place folder must still match its baseline).
    pub(crate) proof: CheckoutProof<'a>,
    /// The proof this same start flow produced moments earlier (the completed
    /// create or the resume verification). When present it is spent instead
    /// of re-verifying `proof`; it was taken within the same flow and is
    /// retired by the first root tool attempt.
    pub(crate) verified: Option<Arc<VerifiedCheckout>>,
    pub(crate) cancellation: &'a CancellationToken,
    /// The in-place job's approved `--write` scope, enforced on every write
    /// door. `None` is a managed checkout.
    pub(crate) write_scope: Option<AssignedPaths>,
    /// The root ceiling: `agent_id` "coordinator", `managed_checkout_id` = W,
    /// `write_paths` = the approved ceiling, capabilities from the request.
    pub(crate) root: RootDelegationInput,
    /// Fresh host consent for the root ceiling (provenance, not a cached proof).
    pub(crate) decider: &'a dyn saya_agent::ApprovalDecider,
    /// The Implementer attenuation over the same ceiling/capabilities.
    pub(crate) authority: ChildAuthorityRequest,
    pub(crate) runtime: &'a RuntimeConfig,
    /// Builds the root's coding operations (commands, checks) from the narrowed
    /// authority view, so they bind the same view the toolset is built over.
    /// `None` advertises no coding command/check tools (the workspace-only
    /// surface). Called once, after the view is narrowed and before the toolset
    /// is built, so the closure captures only what it needs to construct the
    /// `ChildCodingOperations` (the view arrives as its argument).
    pub(crate) coding: Option<CodingOpsFactory<'a>>,
    pub(crate) databases: Option<crate::commands::coding::database::CodingDatabaseTools>,
    /// The job's `[tools] allow` set, whole — the list the root's own
    /// advertisement is narrowed by, and the one a helper's set is derived
    /// from. The derivation is the consumer's (`child/factory.rs` calls
    /// `for_helper`, which removes `helpers`: only the root delegates), so
    /// this field is never a pre-narrowed list.
    pub(crate) tools: saya_config::ResolvedTools,
    /// The job's own shell-output spool directory, the one place outside the
    /// workspace this root's `read` and `grep` may reach.
    pub(crate) spool_dir: Option<PathBuf>,
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub(crate) enum RootComposeError {
    #[error(transparent)]
    Ownership(#[from] OwnershipError),
    #[error(transparent)]
    Authority(#[from] AuthorityError),
    #[error(transparent)]
    ChildTools(#[from] ChildToolsError),
    #[error(transparent)]
    ChildFactory(#[from] ChildFactoryError),
    #[error("the root working checkout is not agent-less")]
    WorkingCheckoutOwned,
    #[error("the root authority does not match verified checkout ownership")]
    AuthorityScopeMismatch,
}

/// Composes the direct-mode root's workspace toolset over W from the same
/// authority path a child uses: re-verify W is Active or Retained (an admitted
/// checkout) and agent-less; pair; approve the root ceiling with `agent_id`
/// "coordinator"; narrow for the Implementer role; build the same
/// coding/database/shell definitions a child gets. Returns the definitions,
/// the executor bound to W, and the `ExactWriteChildApproval` decider. The
/// durable check/merge lineage keeps W's checkout agent id (`None`); only the
/// in-memory authority view carries "coordinator" (the durable merge also
/// passes `None`).
///
/// No sandbox probe runs here: the workspace tools spawn no command, so their
/// writes are admitted in-process. A spawned command's confinement is proven
/// by the shell that spawns it, which is absent where the host cannot prove
/// one.
pub(crate) async fn compose_root_workspace(
    input: RootComposeRequest<'_>,
) -> Result<RootWorkspace, RootComposeError> {
    let verified = match input.verified {
        Some(verified) => verified,
        None => Arc::new(match &input.proof {
            CheckoutProof::Managed(ownership) => {
                verify_root_checkout(ownership, input.cancellation).await?
            }
            CheckoutProof::InPlace(in_place) => verify_in_place_root(in_place)?,
        }),
    };
    // W is the coordinator's own working checkout: it must be agent-less so
    // the durable check/merge lineage (agent id None) stays consistent with
    // the in-memory authority view (agent id "coordinator").
    if verified.agent_id().is_some() {
        return Err(RootComposeError::WorkingCheckoutOwned);
    }
    let pair = TrustedCheckoutPair::from_verified(&verified)?;
    // The workspace tools spawn no command: their writes are admitted
    // in-process against this verified checkout, so the start path runs no
    // sandbox probe. A spawned command's confinement is proven where the shell
    // that spawns it is composed; that shell is absent when the host cannot
    // prove one.
    let mut platform = PlatformSupport::workspace_tools();
    // The database tools independently check each consent-bound identity.
    platform.database_reads = !input.root.database_profiles.is_empty();

    // Capture the identities `validate_view_ownership` expects before the root
    // input is consumed by `approve`.
    let job_id: JobId = input.root.job_id.clone();
    let snapshot_id: SnapshotId = input.root.snapshot_id.clone();
    let root = RootDelegationScope::approve(input.root, input.decider).await?;
    let view = root.narrow(input.authority, platform)?;
    validate_view_ownership(&view, pair.checkout_id(), &job_id, &snapshot_id)
        .map_err(|_| RootComposeError::AuthorityScopeMismatch)?;

    // The factory reuses the child's toolset helpers (`build_toolset`); the
    // resolved AI is a placeholder here because no provider is built (the root
    // loop owns its own provider).
    let ai: ResolvedAi = input.runtime.resolved.ai.clone();
    let mut factory = CliChildFactory::new(pair, ai, Arc::new(input.runtime.secret_resolver()))
        .with_tools(input.tools)
        .with_write_scope(input.write_scope)
        .with_spool_dir(input.spool_dir);
    // The coding operations bind the same narrowed view the toolset is built
    // over; the closure receives the view once it has been narrowed above.
    let coding = input.coding.map(|build| build(&view));
    if let Some(coding) = coding {
        factory = factory.with_coding(coding);
    }
    if let Some(databases) = input.databases {
        factory = factory.with_databases(databases);
    }
    let (definitions, tools, approval) = factory.build_toolset(&view)?;
    Ok(RootWorkspace {
        definitions,
        tools,
        approval,
        checkout_root: PathBuf::from(verified.path().as_str()),
        verified,
    })
}
