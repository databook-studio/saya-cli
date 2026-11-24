//! The root's tool surface, keyed on whether the job has a working checkout.
//! `ReadOnly` is the read-only workspace toolset over the job's own folder (a
//! job with no approved write path, or an in-place root); `WorkingCheckout`
//! composes the child workspace toolset over the agent-less working checkout W
//! with the coordinator's delegation tools, behind the exact-write decider.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::worktree::VerifiedCheckout;
use serde_json::Value;

use crate::commands::coding::{CodingError, NativeCodingJob};

use super::direct_tools::DirectCodingTools;
use super::parent::CodingTools;
use super::readonly::ReadOnlyTools;
use super::root_shell::RootShell;

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

    /// States the text this run was asked — the first turn's own words — which
    /// the optional tools' advertisement is filtered by. A run over a working
    /// checkout states it once; a read-only run advertises no optional tool at
    /// all, so it has none to state it for.
    pub(crate) fn note_request_text(&self, text: &str) {
        if let RootTools::WorkingCheckout(direct) = self {
            direct.note_request_text(text);
        }
    }

    /// Starts the root shell's held composition, if this surface has one, once
    /// the turn's context is built: the probe it starts mutates W.
    pub(crate) fn start_shell_composition(&self) {
        if let RootTools::WorkingCheckout(direct) = self {
            direct.start_shell_composition();
        }
    }

    /// Gives the run the session's own task cell: the `checklist` tool writes
    /// through it, and the run's turns render the list at the turn's start —
    /// the session's own place — instead of this surface rendering it mid-turn.
    /// A run given none (every job surface) keeps a list of its own.
    pub(crate) fn attach_session_tasks(
        &mut self,
        tasks: crate::interactive::session::tasks::SessionTasks,
    ) {
        if let RootTools::WorkingCheckout(direct) = self {
            direct.attach_session_tasks(tasks);
        }
    }

    /// The session task list this request must carry, when the run works
    /// through a session's own list and the list changed since the last
    /// request carried it. Rendered once, at the turn's start: what the
    /// release's turn body did, so a list that changed mid-turn rides the turn
    /// after it.
    pub(crate) fn take_session_tasks(&self) -> Option<saya_agent::ContextBlock> {
        let RootTools::WorkingCheckout(direct) = self else {
            return None;
        };
        direct.session_tasks.as_ref()?;
        crate::interactive::session::tasks::render::render_tasks_block(
            &direct.tasks.take_changed()?,
        )
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
        // write admission over W is the whole write decision — with the
        // session's own grant store beside it: a family the session already
        // holds a grant for is pre-answered, and the workspace tool's
        // containment stays the last word on the path.
        let decider = workspace.approval.with_grants(job.launch_grants());
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
            workspace.tools.with_grants(job.launch_grants()),
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
        // A session's list is rendered at the turn's start, where the release's
        // turn body rendered it: a list that changed mid-turn rides the next
        // turn, never the request already in flight.
        if direct.session_tasks.is_some() {
            return None;
        }
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
