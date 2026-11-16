//! The root's composed toolset for a job with a working checkout: the
//! coordinator's optional delegation tools over the child workspace executor
//! bound to W, behind the exact-write decider. `RootTools` (in
//! `direct.rs`) is the enum the loop holds; this is its `WorkingCheckout`
//! variant.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use saya_agent::{ApprovalDecider, ToolDefinition, ToolError, ToolExecutor};

use crate::agent::tools::child::ChildTools;
use crate::agent::tools::child::decider::ExactWriteChildApproval;
use crate::agent::tools::shell_policy::{self, BackgroundOwner};
use crate::agent::tools::web::{self, WebTools};
use crate::interactive::session::tasks::{SessionTasks, TasksSet};
use crate::mcp::client::McpTools;

use super::parent::CodingTools;
use super::plan_gate::{self, PlanGate};
use super::root_rewind::{REWIND, RootRewind};
use super::root_shell::RootShell;
use super::routes::routes_to_coordinator;

/// The root's composed toolset for a job with a working checkout: the
/// coordinator's optional delegation tools over the child workspace executor
/// bound to W, behind the exact-write decider. `checklist` is the same
/// whole-list session tool the interactive surface already owns — job-local,
/// not persisted. A helper child gets the same list tool on its own executor.
pub(crate) struct DirectCodingTools {
    pub(super) coordinator: CodingTools,
    pub(super) workspace: ChildTools,
    pub(super) decider: ExactWriteChildApproval,
    pub(super) tasks: TasksSet,
    /// The root's `shell`, when the config is on and the mode is not `never`.
    /// Absent means the root advertises no shell at all.
    pub(super) shell: Option<RootShell>,
    /// The root's web lane — `web_fetch`, and `web_search` where a backend is
    /// configured — when the mode reaches the network. Absent means the root
    /// advertises no web tool at all.
    pub(super) web: Option<Arc<WebTools>>,
    /// The root's MCP lane: the tools of the servers the user configured.
    /// Absent means no server is configured, or the mode reaches none, and
    /// then no `mcp__` tool is advertised.
    pub(super) mcp: Option<Arc<McpTools>>,
    /// The root's plan gate, for a job started plan-first. Absent — every job
    /// that never stated `--plan` — means the composition's whole surface is
    /// advertised from the first request, exactly as before this existed.
    /// Present and closed, the advertisement is the read-shaped part of `defs`
    /// plus `checklist` and `submit_plan`; the operator's answer to a
    /// submission opens the rest for the rest of the run.
    pub(super) plan: Option<Arc<PlanGate>>,
    /// The root's model-facing rewind over this job's checkpoint store.
    /// Attached only by `RootTools::build`; absent — every composition that
    /// does not own a run's checkpoints, and every helper — advertises none.
    /// Present, it is advertised for the whole run, from the first request: a
    /// tool that joined the list after the first checkpoint would turn every
    /// later request into a cache miss.
    pub(super) rewind: Option<RootRewind>,
    /// Every definition the composition produced, `[tools] allow` already
    /// applied — the whole advertisement, of which [`Self::advertised`] serves
    /// only the part this run has reached.
    pub(super) defs: Vec<ToolDefinition>,
    /// Whether this job has started a background run yet. The two calls that
    /// reach one — `shell_output` and `shell_kill` — are advertised only from
    /// the moment there is something to reach: before the first successful
    /// background start they are not in the tool list at all, and a call to
    /// either is refused as an unknown tool. Monotonic, so the list changes
    /// once for this cause and is stable afterwards.
    pub(super) background_started: AtomicBool,
    /// Whether this run's workspace has been seen to hold a notebook. The
    /// notebook edit is the one tool offered on a property of the tree rather
    /// than of the composition: it is advertised only where a notebook
    /// exists, re-checked at the start of each turn until one does — the
    /// bounded presence check, which costs nothing once it has answered — and
    /// latched from the moment it is found, so a notebook created during the
    /// run makes the tool appear and the list never flaps back.
    pub(super) notebook_seen: AtomicBool,
    /// What the request itself states: its text, and whether web is configured.
    /// The optional tools (`list`, `delete`, the web pair, `helpers`) are
    /// advertised only when this evidence asks for them or the user named them.
    pub(super) evidence: crate::agent::tools::advertisement::Evidence,
    /// The job's `[tools] allow` set. An explicit name re-advertises a tool the
    /// default set suppresses.
    pub(super) tools: saya_config::ResolvedTools,
}

impl DirectCodingTools {
    /// Combines the child workspace definitions. First-ship leftover
    /// coordinator, check, skill, and task names stay implemented but are
    /// dropped here so they cannot leak back in.
    // Every argument is one distinct, named part of the composition — the
    // coordinator, the workspace, its definitions, the write decider, and the
    // three surfaces that may be absent. Grouping them would only move the same
    // list behind one more name.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        coordinator: CodingTools,
        workspace: ChildTools,
        workspace_definitions: Vec<ToolDefinition>,
        decider: ExactWriteChildApproval,
        shell: Option<RootShell>,
        agents: &saya_config::HelperAgents,
        web: Option<Arc<WebTools>>,
        mcp: Option<Arc<McpTools>>,
        plan: Option<Arc<PlanGate>>,
        request: String,
        tools: saya_config::ResolvedTools,
    ) -> Self {
        let evidence = crate::agent::tools::advertisement::Evidence::new(
            request,
            web.as_ref().is_some_and(|web| web.search_configured()),
        );
        let mut workspace = workspace;
        workspace.apply_evidence(evidence.clone());
        let mut defs: Vec<ToolDefinition> = super::definitions(coordinator.databases.as_ref());
        // The direct-only `helpers` tool. Advertised; its description says
        // to use it only when the user asked for a helper, and it advertises
        // exactly the helper types this job's registry holds.
        defs.push(super::task_tool::definition(agents));
        // Same `checklist` the interactive session already owns. Job-local
        // only: a coding job has no session record to sync. Advertised.
        defs.push(crate::interactive::session::tasks::render::checklist_definition());
        let mut workspace_definitions = workspace_definitions;
        crate::agent::tools::workspace::prepare_coding_workspace_tools(&mut workspace_definitions);
        let existing: HashSet<String> = defs.iter().map(|d| d.name.clone()).collect();
        defs.extend(
            workspace_definitions
                .into_iter()
                .filter(|definition| !existing.contains(&definition.name)),
        );
        // The sandboxed root shell is advertised only when it exists: config
        // off, no operator, or an unproven sandbox all mean no `shell` at all.
        // With it come the two calls that reach a background run, composed
        // here but withheld from the advertisement until a run exists (see
        // `background_started`): the root's shell is the one shell that owns
        // background runs, so it is the one surface that offers them.
        if shell.is_some() {
            defs.extend(crate::agent::tools::shell_policy::background_definitions(
                BackgroundOwner::Job,
            ));
        }
        // The root's web lane, advertised only when the mode reaches the
        // network: `read-only` and `never` advertise none at all. Which of the
        // two tools the lane holds is the lane's own answer — the mode decides
        // `web_fetch`, and the user's `[web_search]` decides `web_search`.
        if let Some(web) = &web {
            defs.extend(web.definitions());
        }
        // The configured MCP servers' tools, advertised only where a lane was
        // composed: a mode that reaches no server composes none, and neither
        // does a configuration that names none. The lane's own reports say
        // which servers failed and which tools it dropped.
        if let Some(mcp) = &mcp {
            defs.extend(mcp.definitions());
        }
        // The plan gate's `submit_plan`, advertised only by a job that started
        // plan-first: it is the one way the posture it is under can end. The
        // gate itself filters the advertisement from the first request (see
        // `advertises`), so `submit_plan` is offered while the write surface is
        // closed and the rest of `defs` joins it once the operator approves.
        if plan.is_some() {
            defs.push(plan_gate::PlanGate::definition());
        }
        // The notebook edit — the one workspace tool added after this list's
        // order was fixed — goes last, after the shell, web, MCP and plan
        // lanes above: every tool that existed before it keeps the position it
        // had, so the request's cached prefix is unchanged. It stays in `defs`
        // even where the workspace holds no notebook: the advertisement
        // filters it (`advertises_notebook`), so a notebook created during the
        // run still makes it appear — at the end, where it already is.
        crate::agent::tools::workspace::notebook_edit_last(&mut defs);
        Self {
            coordinator,
            workspace,
            decider,
            tasks: TasksSet::new(SessionTasks::default()),
            shell,
            web,
            mcp,
            plan,
            rewind: None,
            defs,
            background_started: AtomicBool::new(false),
            notebook_seen: AtomicBool::new(false),
            evidence,
            tools,
        }
    }

    /// Attaches the root's model-facing rewind. Appended last, like every tool
    /// added after the list's order was fixed, so the request's cached prefix
    /// is unchanged; `[tools] allow` is applied by the caller afterwards and
    /// can still take it away.
    pub(crate) fn attach_rewind(&mut self, rewind: RootRewind) {
        self.defs.push(RootRewind::definition());
        self.rewind = Some(rewind);
    }

    /// Whether this run has reached `name`: everything the composition
    /// advertised, except the notebook edit where the workspace holds no
    /// notebook, the two background readers before the run that started a
    /// background shell, and — under a closed plan gate — everything the plan
    /// posture does not reach. The one predicate the advertisement and the
    /// dispatch gate both read, so the two cannot disagree about what the model
    /// was offered.
    pub(super) fn advertises(&self, name: &str) -> bool {
        self.defs.iter().any(|definition| {
            definition.name == name
                && (name != "notebook_edit" || self.advertises_notebook())
                && (!shell_policy::reaches_background(name)
                    || self.background_started.load(Ordering::Acquire))
                && self.plan_permits(definition)
                && crate::agent::tools::advertisement::advertised(&self.evidence, &self.tools, name)
        })
    }

    /// Whether the notebook edit is advertised: the workspace holds a
    /// notebook. Decided by the bounded presence check — the same contained,
    /// gitignore-aware walk the searches use, stopped at the first `.ipynb`
    /// name — at the start of each turn until it answers yes, and latched from
    /// then on, so the list grows once for this cause and is stable
    /// afterwards. A workspace the check could not finish offers no notebook
    /// edit: the tool appears only on a definite yes.
    pub(super) fn advertises_notebook(&self) -> bool {
        if self.notebook_seen.load(Ordering::Acquire) {
            return true;
        }
        if crate::agent::tools::workspace::notebook::present(self.workspace.workspace()) {
            self.notebook_seen.store(true, Ordering::Release);
            return true;
        }
        false
    }

    /// Whether the job's plan posture reaches `definition`. With no gate —
    /// every job that never stated `--plan` — the whole composition is reached,
    /// so nothing here filters. With a closed gate, only the read-shaped part
    /// is: the session's own plan rule (`read_only_permits`), which is what
    /// admits `read`, `list`, `glob`, `grep`, and the two session-local tools
    /// `checklist` and `submit_plan`, and refuses the write family, `delete`,
    /// the shell family, `helpers` and `web_fetch`. The operator's approval
    /// opens the rest, and the gate never closes again.
    fn plan_permits(&self, definition: &ToolDefinition) -> bool {
        match &self.plan {
            None => true,
            Some(plan) => plan.is_open() || saya_agent::read_only_permits(&definition.effect),
        }
    }

    /// The advertisement itself: the composed definitions this run has
    /// reached, in composition order.
    pub(super) fn advertised_definitions(&self) -> Vec<ToolDefinition> {
        self.defs
            .iter()
            .filter(|definition| self.advertises(&definition.name))
            .cloned()
            .collect()
    }

    /// Notes a call's outcome against the advertisement: a `shell` call that
    /// started a background run is what puts `shell_output` and `shell_kill`
    /// in the tool list, from the next request on. A call that failed — an
    /// unknown tool, a refusal, a cancelled start — advertises nothing.
    pub(super) fn note_background_start(
        &self,
        started: bool,
        outcome: &Result<serde_json::Value, ToolError>,
    ) {
        if started && outcome.is_ok() {
            self.background_started.store(true, Ordering::Release);
        }
    }

    /// Whether `name` is served by the root shell: `shell` itself (a model may
    /// say `bash`), or one of the two calls that reach a background run it
    /// started. The shell family's names are the schema's one table, so this
    /// dispatch and the advertisement cannot disagree.
    pub(super) fn is_shell(name: &str) -> bool {
        crate::agent::tools::shell_policy::operation(name).is_some()
    }

    /// Picks the executor for `name`: the root shell for `shell`, the web lane
    /// for either of its tools, `checklist` for the job-local list,
    /// coordinator-routed names to the coordinator executor, everything else
    /// to the workspace.
    pub(super) fn executor_for(&self, name: &str) -> &dyn ToolExecutor {
        if Self::is_shell(name)
            && let Some(shell) = &self.shell
        {
            return shell;
        }
        if web::is_web_tool(name)
            && let Some(web) = &self.web
        {
            return web.as_ref();
        }
        if McpTools::owns(name)
            && let Some(mcp) = &self.mcp
        {
            return mcp.as_ref();
        }
        if PlanGate::owns(name)
            && let Some(plan) = &self.plan
        {
            return plan.as_ref();
        }
        if name == REWIND
            && let Some(rewind) = &self.rewind
        {
            return rewind;
        }
        if name == "checklist" {
            &self.tasks
        } else if routes_to_coordinator(name) {
            &self.coordinator
        } else {
            &self.workspace
        }
    }

    /// Picks the approval decider for `tool`. The root shell and the web lane
    /// each carry their own approval decision; when either is absent its name
    /// is unknown and never reaches here (it is not advertised).
    pub(super) fn decider_for(&self, tool: &ToolDefinition) -> &dyn ApprovalDecider {
        if Self::is_shell(&tool.name)
            && let Some(shell) = &self.shell
        {
            return shell;
        }
        if web::is_web_tool(&tool.name)
            && let Some(web) = &self.web
        {
            return web.as_ref();
        }
        if McpTools::owns(&tool.name)
            && let Some(mcp) = &self.mcp
        {
            return mcp.as_ref();
        }
        if super::routes::routes_to_root_owned(&tool.name) {
            &self.coordinator
        } else {
            &self.decider
        }
    }

    /// Approves a coordinator-routed tool iff it is one of the advertised
    /// definitions. Hidden first-ship names are dropped from `defs`, so they
    /// are not approved here. The coordinator-only `CodingTools::approve` is
    /// unchanged and still recognizes only its own pinned names.
    pub(super) fn approves_coordinator(&self, tool: &ToolDefinition) -> bool {
        self.defs
            .iter()
            .any(|expected| expected.name == tool.name && expected.effect == tool.effect)
    }
}
