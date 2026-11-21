//! The root's tool surface, keyed on whether the job has a working checkout.
//! `ReadOnly` is the read-only workspace toolset over the job's own folder (a
//! job with no approved write path, or an in-place root); `WorkingCheckout`
//! composes the child workspace toolset over the agent-less working checkout W
//! with the coordinator's delegation tools, behind the exact-write decider.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ImmutableAuthorityRefusal, ToolDefinition, ToolError, ToolExecutor,
};
use saya_harness::coding::worktree::VerifiedCheckout;
use serde_json::Value;

use crate::commands::coding::{CodingError, NativeCodingJob};

use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use super::readonly::ReadOnlyTools;
use super::root_shell::RootShell;
use super::routes::routes_to_root_owned;
use crate::agent::tools::write_ask::{self, hidden_write};

/// The root loop's tool surface, keyed on the job's working checkout:
/// `ReadOnly` reads the job's own folder; `WorkingCheckout` is the composed
/// child workspace + optional delegation tools. Both variants are boxed: each
/// carries a large toolset (`ReadOnlyTools` holds the database and workspace
/// tools, `DirectCodingTools` a whole child workspace), so indirection keeps
/// the enum itself pointer-sized.
pub(crate) enum RootTools {
    ReadOnly(Box<ReadOnlyTools>),
    WorkingCheckout(Box<DirectCodingTools>),
}

impl RootTools {
    /// The contained checkout root this surface is bound to, when it has one.
    /// The read-only surface reads the job's own folder, but it offers no
    /// shell, so the data-file shell route stays off: `None` here keeps that
    /// guidance out of the prompt.
    pub(crate) fn workspace(&self) -> Option<&saya_harness::workspace::Workspace> {
        match self {
            RootTools::ReadOnly(_) => None,
            RootTools::WorkingCheckout(direct) => direct.workspace.workspace(),
        }
    }

    /// Whether this surface advertises the root shell. The data-file route
    /// names `shell`, so it may be appended only where the model is offered
    /// that tool: a read-only surface, a composition with the shell config off,
    /// a closed plan posture and a `[tools] allow` that drops it all answer no.
    pub(crate) fn offers_shell(&self) -> bool {
        self.definitions()
            .iter()
            .any(|definition| definition.name == "shell")
    }

    /// Starts the root shell's held composition, if this surface has one, once
    /// the turn's context is built: the probe it starts mutates W.
    pub(crate) fn start_shell_composition(&self) {
        if let RootTools::WorkingCheckout(direct) = self {
            direct.start_shell_composition();
        }
    }

    /// Builds the root tool surface from the job. A job with no working
    /// checkout (no approved write path, or in place) gets the read-only
    /// workspace toolset over its own folder; a job with one gets the composed
    /// child workspace toolset over W plus the coordinator's delegation tools,
    /// behind the exact-write decider. `ensure_root_checkout` is called once
    /// here (the caller no longer calls it). The working checkout's use-time
    /// verification is returned so the caller does not inventory the same
    /// checkout again.
    pub(crate) async fn build(
        job: &Arc<NativeCodingJob>,
    ) -> Result<(Self, Option<Arc<VerifiedCheckout>>), CodingError> {
        job.lifecycle().enter("checkout");
        let checkout = job.ensure_root_checkout().await?;
        let Some(checkout) = checkout else {
            return Ok((
                RootTools::ReadOnly(Box::new(ReadOnlyTools::build(job)?)),
                None,
            ));
        };
        // The proof `ensure_root_checkout` just produced (a completed create
        // or a resume verification) composes the workspace without a second
        // walk of W. It is keyed to W's epoch, so a stale proof is never
        // spent; the composition re-verifies when it is absent.
        let proof = job.root_checkout_proof();
        // An in-place write job composes the same full workspace toolset, but
        // over the user's own folder; there is no managed checkout to verify.
        job.lifecycle().enter("workspace");
        let workspace = if job.is_in_place() {
            job.compose_in_place_workspace().await?
        } else {
            job.compose_direct_workspace(&checkout, proof).await?
        };
        job.lifecycle().enter("shell");
        let coordinator = CodingTools {
            job: Arc::clone(job),
            databases: None,
            denials: super::owner_denial::DenialTally::default(),
        };
        // The root writes the shared working checkout, so the exact
        // write admission over W is the whole write decision.
        let decider = workspace.approval;
        // The root's shell: built over the working checkout's own root, and
        // only when the config is on, the mode is not `never`, and the sandbox
        // (or, without one, the host lane) allows it.
        let shell = super::root_shell::build(
            job,
            &workspace.checkout_root,
            workspace.tools.read_witnesses(),
            workspace.tools.workspace_handle(),
        );
        // The root's `web_fetch`: the job's own egress over the mode's
        // consent rule, and `None` in a mode that reaches no host.
        let web = super::root_web::build(job);
        // The root's MCP lane: the configured servers' tools, started here
        // once for the whole run, or `None` where the mode reaches no server
        // and the configuration names none.
        job.lifecycle().enter("mcp");
        let mcp = super::root_mcp::build(job).await;
        job.lifecycle().enter("toolset");
        // The root's plan gate: present only for a job started plan-first, and
        // then closed until the operator answers a submitted plan. A job
        // without `--plan` composes `None` and advertises the whole surface.
        let plan = job.plan_gate();
        let mut direct = DirectCodingTools::new(
            coordinator,
            workspace.tools,
            workspace.definitions,
            decider,
            shell,
            job.helper_agents(),
            web,
            mcp,
            plan,
            job.task().to_owned(),
            job.tools().clone(),
        );
        // The root's model-facing rewind over this job's own checkpoint store.
        // Attached here and nowhere else: helpers compose their own child
        // toolset and `saya run` composes `RunTools`, so neither advertises it.
        direct.attach_rewind(super::root_rewind::RootRewind::new(Arc::clone(job)));
        // The job's `[tools] allow` list, applied after the composition above
        // so it can only ever take a tool away. `direct.defs` is both the
        // advertisement and the dispatch gate (`advertises`), so one filter
        // covers both and they cannot drift.
        crate::agent::tools::narrow(&mut direct.defs, job.tools());
        Ok((
            RootTools::WorkingCheckout(Box::new(direct)),
            Some(workspace.verified),
        ))
    }

    /// The definitions the loop advertises: the read-only workspace and
    /// database tools for a job with no working checkout; coordinator tools
    /// plus child workspace for a job with one. A working checkout's list is
    /// what the run has reached so far — the background readers, the plan
    /// posture's write surface and the notebook edit join it as they exist.
    pub(crate) fn definitions(&self) -> Vec<ToolDefinition> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.definitions(),
            RootTools::WorkingCheckout(direct) => direct.advertised_definitions(),
        }
    }
}

#[async_trait]
impl ToolExecutor for RootTools {
    /// The loop re-reads this before each request, so a cause that genuinely
    /// cannot be known earlier grows the list at most once: an approved plan's
    /// write surface, a background start's two readers, a notebook's edit. A
    /// job with no working checkout keeps the list the loop was given.
    fn advertised_definitions(&self) -> Option<Vec<ToolDefinition>> {
        match self {
            RootTools::ReadOnly(_) => None,
            RootTools::WorkingCheckout(direct) => Some(direct.advertised_definitions()),
        }
    }

    fn turn_context_note(&self) -> Option<String> {
        let RootTools::WorkingCheckout(direct) = self else {
            return None;
        };
        let block = crate::interactive::session::tasks::render::render_tasks_block(
            &direct.tasks.take_changed()?,
        )?;
        Some(saya_agent::render_untrusted_block(&block))
    }

    async fn settle_invocation(&self) -> Result<(), ToolError> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.settle_invocation().await,
            RootTools::WorkingCheckout(direct) => direct.workspace.settle_invocation().await,
        }
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        match self {
            RootTools::ReadOnly(read_only) => {
                read_only
                    .execute_with_context(name, arguments, context)
                    .await
            }
            RootTools::WorkingCheckout(direct) => {
                let name = crate::agent::tools::workspace::canonical_name(name);
                if !direct.advertises(name) {
                    return Err(ToolError::UnsupportedTool);
                }
                let started =
                    crate::agent::tools::shell_policy::starts_background(name, &arguments);
                let outcome = direct
                    .executor_for(name)
                    .execute_with_context(name, arguments, context)
                    .await;
                direct.note_background_start(started, &outcome);
                super::root_progress::note(&direct.coordinator.job, name);
                outcome
            }
        }
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.execute(name, arguments).await,
            RootTools::WorkingCheckout(direct) => {
                let name = crate::agent::tools::workspace::canonical_name(name);
                if !direct.advertises(name) {
                    return Err(ToolError::UnsupportedTool);
                }
                let started =
                    crate::agent::tools::shell_policy::starts_background(name, &arguments);
                let outcome = direct.executor_for(name).execute(name, arguments).await;
                direct.note_background_start(started, &outcome);
                super::root_progress::note(&direct.coordinator.job, name);
                outcome
            }
        }
    }

    fn take_background_exits(&self) -> Vec<saya_agent::BackgroundExit> {
        match self {
            RootTools::ReadOnly(_) => Vec::new(),
            RootTools::WorkingCheckout(direct) => direct
                .shell
                .as_ref()
                .map(RootShell::take_background_exits)
                .unwrap_or_default(),
        }
    }

    fn background_live(&self) -> bool {
        match self {
            RootTools::ReadOnly(_) => false,
            RootTools::WorkingCheckout(direct) => direct
                .shell
                .as_ref()
                .is_some_and(RootShell::background_live),
        }
    }

    fn background_notice_pending(&self) -> bool {
        match self {
            RootTools::ReadOnly(_) => false,
            RootTools::WorkingCheckout(direct) => direct
                .shell
                .as_ref()
                .is_some_and(RootShell::background_notice_pending),
        }
    }

    async fn wait_background_exit(
        &self,
        deadline: std::time::Instant,
        cancellation: &saya_agent::CancellationToken,
    ) -> bool {
        match self {
            RootTools::ReadOnly(_) => false,
            RootTools::WorkingCheckout(direct) => match &direct.shell {
                Some(shell) => shell.wait_background_exit(deadline, cancellation).await,
                None => false,
            },
        }
    }
}

#[async_trait]
impl ApprovalDecider for RootTools {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        // A chat-surface run decides its database tools in the plugin's lane,
        // before the variant split: the read-only surface allows every name it
        // advertises unasked, so the lane has to come first to be the ask it is.
        if let Some(allowed) = self.db_lane_approve(tool, arguments).await {
            return allowed;
        }
        match self {
            RootTools::ReadOnly(read_only) => read_only.approve(tool, arguments).await,
            RootTools::WorkingCheckout(direct) => {
                // Coordinator-routed tools are approved against the advertised
                // definitions; hidden first-ship names are not advertised. The
                // exact-write decider handles workspace tools, and the root
                // shell and the fetch tool each decide a call the way their own
                // mode says.
                if routes_to_root_owned(&tool.name) {
                    return direct.approves_coordinator(tool);
                }
                if DirectCodingTools::is_shell(&tool.name) {
                    return match &direct.shell {
                        Some(shell) => shell.approve(tool, arguments).await,
                        None => false,
                    };
                }
                if crate::agent::tools::web::is_web_tool(&tool.name) {
                    return match &direct.web {
                        Some(web) => web.approve(tool, arguments).await,
                        None => false,
                    };
                }
                // An MCP tool is a call to a program saya cannot bound, so it
                // is decided the way the root shell is: by its own lane.
                if crate::mcp::client::McpTools::owns(&tool.name) {
                    return match &direct.mcp {
                        Some(mcp) => mcp.approve(tool, arguments).await,
                        None => false,
                    };
                }
                // The exact-write admission decides what this call may
                // touch, and the job's mode then decides whether its consent
                // covers a write: only an `ask` job with an operator asks. A
                // write this surface does not offer at all is refused before
                // the question, because the executor would refuse it too.
                match write_ask::for_root(&direct.coordinator.job) {
                    Some(_) if hidden_write(tool, |name| direct.advertises(name)) => false,
                    Some(ask) => ask.approve(&direct.decider, tool, arguments).await,
                    None => direct.decider.approve(tool, arguments).await,
                }
            }
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        // The lane's ask is the ask a `PermissionRequest` hook may pre-empt, so
        // the gate reads the same table the call is decided by.
        if let Some(asked) = self.db_lane_would_ask(tool, arguments) {
            return Some(asked);
        }
        match self {
            RootTools::ReadOnly(read_only) => read_only.would_ask(tool, arguments),
            RootTools::WorkingCheckout(direct) => {
                // A coordinator-routed tool is approved by its advertised
                // definition, not by an operator: no ask to pre-empt.
                if routes_to_root_owned(&tool.name) {
                    return Some(false);
                }
                match write_ask::for_root(&direct.coordinator.job) {
                    Some(_) if hidden_write(tool, |name| direct.advertises(name)) => Some(false),
                    Some(ask) => ask.would_ask(direct.decider_for(tool), tool, arguments),
                    None => direct.decider_for(tool).would_ask(tool, arguments),
                }
            }
        }
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.refusal_detail(tool, arguments),
            RootTools::WorkingCheckout(direct) => {
                direct.decider_for(tool).refusal_detail(tool, arguments)
            }
        }
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        match self {
            RootTools::ReadOnly(read_only) => {
                read_only.immutable_authority_refusal(tool, arguments)
            }
            RootTools::WorkingCheckout(direct) => direct
                .decider_for(tool)
                .immutable_authority_refusal(tool, arguments),
        }
    }
}
