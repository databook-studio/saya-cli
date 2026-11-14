//! The child tool executor: admission immediately before the existing anchored mutation.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use async_trait::async_trait;
use saya_agent::{LocalStateEffect, ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::integration::AssignedPaths;
use saya_harness::coding::policy::ChildAuthorityView;
use saya_harness::workspace::Workspace;
use saya_types::{ManagedCheckoutId, WorkspacePath};

use super::super::DatabaseTools;
use super::super::shell_policy::{self, BackgroundOwner, ShellTool};
use super::super::workspace;
use super::{ChildToolsError, TrustedCheckoutPair};
use crate::commands::coding::database::CodingDatabaseTools;
use crate::interactive::session::tasks::{SessionTasks, TasksSet};

#[path = "executor/dispatch.rs"]
mod dispatch;
#[path = "executor/post_edit.rs"]
mod post_edit;

/// A tool set bound to one child's verified managed checkout: the workspace
/// tools for the child root plus the narrowed authority that admits writes.
/// The helper's `shell` exists only when the composition root attached one
/// (the job's config and mode allow it), and the standard read-only database
/// tools only when the job selected a profile the view admits.
pub(crate) struct ChildTools {
    tools: DatabaseTools,
    view: ChildAuthorityView,
    checkout_id: ManagedCheckoutId,
    permit_writes: bool,
    /// The helper's own `shell`, composed by the composition root over the
    /// shared working checkout: the same tool the root advertises, with this
    /// helper's scratch tree and subject line.
    shell: Option<Arc<ShellTool>>,
    /// Whether this helper has started a background run yet. The two calls
    /// that reach one — `shell_output` and `shell_kill` — are advertised only
    /// from the moment there is something to reach, exactly as the root's are.
    /// Monotonic, so the list changes once for this cause.
    background_started: AtomicBool,
    /// The definitions this helper's loop advertises, `[tools] allow` already
    /// applied. [`Self::advertisement`] serves the part the helper has reached,
    /// so the list can grow with a background start without rebuilding it.
    definitions: Vec<ToolDefinition>,
    /// The helper's own web lane, composed by the composition root over the
    /// job's own egress: the same tools the root advertises, with this
    /// helper's subject line.
    web: Option<Arc<crate::agent::tools::web::WebTools>>,
    /// The helper's own MCP lane, composed by the composition root over the
    /// job's configured servers. Its consent rule is the lane's own, not this
    /// executor's; this surface only routes the names the lane owns.
    mcp: Option<Arc<crate::mcp::client::McpTools>>,
    databases: Option<Arc<CodingDatabaseTools>>,
    coding: Option<Arc<crate::commands::coding::checks::ChildCodingOperations>>,
    /// This child's own working list. The root keeps a separate one.
    tasks: TasksSet,
    /// The job's `[tools] allow` set as this helper sees it: the job's list
    /// minus `helpers`. Unstated admits every name this executor serves.
    tools_allow: saya_config::ResolvedTools,
    /// Whether this helper's list offers the notebook edit: the composition's
    /// own answer to whether the checkout holds a notebook. A helper's list is
    /// fixed for its whole run, so the factory decides once and drops the
    /// definition where the answer is no; this carries the same answer to the
    /// dispatch. The direct-mode root shares `build_toolset` and so leaves it
    /// open — its list is re-read every turn, and its own advertisement gate
    /// is what refuses the call where no notebook exists.
    notebook_edit: bool,
    /// What the request itself states. The optional tools — `list`, `delete`,
    /// the web pair — are advertised only when this evidence asks for them or
    /// the user named them in `[tools] allow`.
    evidence: crate::agent::tools::advertisement::Evidence,
    /// The in-place job's approved `--write` scope, enforced at this tool
    /// boundary. `None` is a managed checkout, where the working checkout
    /// itself is the ceiling and the operator's review is the check.
    write_scope: Option<AssignedPaths>,
}

/// Builds the definitions and the executor for one child from one view: the
/// two stay in agreement because `permit` is computed once and used for both.
/// Write-shaped child tools always go through the child decider; the loop's
/// own `permit_workspace_writes` limit is projected from the view by the
/// composition root. `shell` is attached separately by [`ChildTools::attach_shell`]
/// (the composition root builds it, because it holds the job's mode and
/// operator), so this surface advertises no shell until one exists.
pub(crate) fn child_toolset(
    view: &ChildAuthorityView,
    pair: &TrustedCheckoutPair,
) -> Result<(Vec<ToolDefinition>, ChildTools), ChildToolsError> {
    // A view narrowed for another checkout is refused at build time.
    if view.managed_checkout_id() != Some(pair.checkout_id()) {
        return Err(ChildToolsError::CheckoutMismatch);
    }
    // Computed once: both the definitions and the executor must agree on it.
    let permit = view.workspace_write();
    let mut definitions = Vec::new();
    workspace::append_definitions(&mut definitions, permit);
    workspace::canonicalize_tool_names(&mut definitions);
    definitions.push(crate::interactive::session::tasks::render::checklist_definition());
    // Last in the workspace family, so a surface that appends its own lanes
    // after this list can move it to the very end (see `notebook_edit_last`)
    // without the tools that existed before it ever shifting.
    workspace::append_notebook_edit(&mut definitions, permit);
    require_decider_for_writes(&mut definitions);
    let tools = DatabaseTools::for_child_checkout(Arc::clone(&pair.root));
    let executor = ChildTools {
        tools,
        view: view.clone(),
        checkout_id: pair.checkout_id().clone(),
        permit_writes: permit,
        shell: None,
        background_started: AtomicBool::new(false),
        definitions: definitions.clone(),
        web: None,
        mcp: None,
        coding: None,
        databases: None,
        tasks: TasksSet::new(SessionTasks::default()),
        tools_allow: saya_config::ResolvedTools::default(),
        notebook_edit: true,
        evidence: crate::agent::tools::advertisement::Evidence::default(),
        write_scope: None,
    };
    Ok((definitions, executor))
}

/// Write-shaped child tools always go through the child decider: every
/// definition whose effect is a workspace write is marked so the loop
/// consults the child's decider before running it; the loop's own
/// `permit_workspace_writes` limit is projected from the view by the
/// composition root.
fn require_decider_for_writes(definitions: &mut [ToolDefinition]) {
    for definition in definitions {
        if matches!(
            definition.effect.local_state,
            LocalStateEffect::WriteWorkspace
        ) {
            definition.effect.requires_approval = true;
        }
    }
}

impl ChildTools {
    /// Narrows this executor to the set its definitions were narrowed to. The
    /// composition root calls it once, so the advertisement and the dispatch
    /// read the same list.
    pub(crate) fn narrow_to(&mut self, tools: saya_config::ResolvedTools) {
        self.tools_allow = tools;
    }

    /// The contained checkout root this executor is bound to — the same
    /// workspace its tools read and write. A composition that must decide
    /// something about the tree before it advertises (the root's notebook
    /// edit) reads it here rather than holding a second copy of the root.
    pub(crate) fn workspace(&self) -> Option<&Workspace> {
        self.tools.workspace.as_deref()
    }

    /// The run's shared read-witness store, so the shell composed beside this
    /// toolset records into the same table its `edit` reads.
    pub(in crate::agent::tools) fn read_witnesses(
        &self,
    ) -> Arc<crate::agent::tools::workspace::ReadWitnesses> {
        self.tools.read_witnesses_handle()
    }

    /// The contained checkout root as an owned handle, for the shell's view
    /// recorder.
    pub(crate) fn workspace_handle(&self) -> Option<Arc<Workspace>> {
        self.tools.workspace_handle()
    }

    /// Joins this toolset to the run's read-witness store.
    pub(in crate::agent::tools) fn with_read_witnesses(
        mut self,
        witnesses: Arc<crate::agent::tools::workspace::ReadWitnesses>,
    ) -> Self {
        self.tools = self.tools.with_read_witnesses(witnesses);
        self
    }

    /// Attaches the job's own shell-output spool directory.
    pub(crate) fn with_spool_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.tools = self.tools.with_spool_dir(dir);
        self
    }

    /// Narrows this executor to the notebook decision its definitions were
    /// narrowed by. The child factory calls it once, with the same answer it
    /// dropped the definition on, so the advertisement and the dispatch read
    /// the same list.
    pub(crate) fn offer_notebook_edit(&mut self, offer: bool) {
        self.notebook_edit = offer;
    }

    /// Applies the request's evidence to this executor's advertisement and
    /// dispatch alike. The definitions it holds were composed before the
    /// request was known, so the optional ones are filtered here; the same
    /// rule runs in [`Self::admits`], so a call and the list cannot disagree.
    pub(crate) fn apply_evidence(
        &mut self,
        evidence: crate::agent::tools::advertisement::Evidence,
    ) {
        self.evidence = evidence;
        self.definitions.retain(|definition| {
            crate::agent::tools::advertisement::advertised(
                &self.evidence,
                &self.tools_allow,
                &definition.name,
            )
        });
    }

    /// Whether this executor serves `name`: the job's `[tools] allow` list,
    /// minus `helpers`, and — for the notebook edit — the composition's own
    /// answer to whether the checkout holds a notebook. The same two
    /// decisions its definitions were narrowed by, so the advertisement and
    /// the dispatch cannot drift.
    fn admits(&self, name: &str) -> bool {
        (self.notebook_edit || name != "notebook_edit")
            && (!shell_policy::reaches_background(name)
                || self.background_started.load(Ordering::Acquire))
            && crate::agent::tools::admits(&self.tools_allow, name)
            && crate::agent::tools::advertisement::advertised(
                &self.evidence,
                &self.tools_allow,
                name,
            )
    }

    /// The definitions this helper has reached, in composition order. The
    /// child loop re-reads this before each request, so a background start
    /// made in one turn puts the two readers in the next turn's list.
    pub(crate) fn advertisement(&self) -> Vec<ToolDefinition> {
        self.definitions
            .iter()
            .filter(|definition| self.admits(&definition.name))
            .cloned()
            .collect()
    }

    /// Holds the composed definitions this executor advertises from. The
    /// composition root calls it once, after `[tools] allow` narrowed them, so
    /// the advertisement and the dispatch read the same list.
    pub(crate) fn hold_definitions(&mut self, definitions: &[ToolDefinition]) {
        self.definitions = definitions.to_vec();
    }

    /// Bounds this executor's write tools to the in-place job's approved
    /// `--write` scope. `None` leaves a managed checkout unbounded by path.
    pub(crate) fn with_write_scope(mut self, scope: Option<AssignedPaths>) -> Self {
        self.write_scope = scope;
        self
    }

    pub(crate) fn attach_databases(&mut self, databases: Arc<CodingDatabaseTools>) {
        self.databases = Some(databases);
    }

    pub(crate) fn attach_coding(
        &mut self,
        coding: Arc<crate::commands::coding::checks::ChildCodingOperations>,
    ) -> Option<ToolDefinition> {
        // An in-place job records edit-time before-images through the same
        // four-method recorder the session path uses. Root and helpers share
        // the job's one store; a managed checkout keeps the session recorder
        // absent.
        if coding.application.is_in_place()
            && let Some(store) = &coding.before_images
            && let Some(workspace_handle) = self.tools.workspace_handle()
        {
            let recorder = workspace::checkpoint::CheckpointRecorder::job(
                Arc::clone(store),
                coding.before_image_turn(),
                workspace_handle,
            );
            self.tools.set_checkpoints(Some(recorder));
        }
        self.coding = Some(coding);
        self.permit_writes.then(workspace::delete_definition)
    }

    /// Attaches the helper's `shell` and returns the family to advertise: the
    /// same start and readers the root advertises, so the two surfaces cannot
    /// drift. The shell is built by the composition root — it holds the job's
    /// mode and operator — and its lane decision is the shell's own, not this
    /// executor's. The family joins the held list here; [`Self::admits`]
    /// withholds the readers until the helper starts a run.
    pub(crate) fn attach_shell(&mut self, shell: Arc<ShellTool>) -> Vec<ToolDefinition> {
        self.shell = Some(shell);
        let family = shell_policy::background_definitions(BackgroundOwner::Helper);
        self.definitions.extend(family.iter().cloned());
        family
    }

    /// Attaches the helper's web lane and returns the definitions to
    /// advertise: the same ones the root advertises, so the two surfaces
    /// cannot drift. Built by the composition root over the job's own egress —
    /// it holds the job's mode and operator — and its consent rule is each
    /// tool's own, not this executor's.
    pub(crate) fn attach_web(
        &mut self,
        web: Arc<crate::agent::tools::web::WebTools>,
    ) -> Vec<ToolDefinition> {
        let definitions = web.definitions();
        self.web = Some(web);
        definitions
    }

    /// Attaches the helper's MCP lane and returns the definitions to
    /// advertise: the same ones the root advertises, so the two surfaces
    /// cannot drift. Built by the composition root over the job's configured
    /// servers — it holds the job's mode and operator — and its consent rule
    /// is the lane's own, not this executor's.
    pub(crate) fn attach_mcp(
        &mut self,
        mcp: Arc<crate::mcp::client::McpTools>,
    ) -> Vec<ToolDefinition> {
        let definitions = mcp.definitions();
        self.mcp = Some(mcp);
        definitions
    }
}

/// The write admission every mutating door runs before the anchored mutation:
/// the owner must still be alive, the view must permit workspace writes, and
/// the path must be one the contained checkout can hold.
impl ChildTools {
    fn admit_mutation(&self, arguments: &serde_json::Value) -> Result<(), ToolError> {
        if self
            .coding
            .as_ref()
            .is_some_and(|coding| coding.failed.load(Ordering::Acquire))
        {
            return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                "the native job owner failed and cannot admit another workspace mutation".into(),
            ))));
        }
        if !self.permit_writes {
            return Err(ToolError::NoEffect(Box::new(ToolError::UnsupportedTool)));
        }
        let path = arguments
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| ToolError::NoEffect(Box::new(ToolError::PathNotString)))?;
        if path.len() > 512 {
            return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                "the path exceeds the child scope bound".into(),
            ))));
        }
        WorkspacePath::try_from(path.to_owned()).map_err(|_| {
            ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                "the path is not an admissible workspace path".into(),
            )))
        })?;
        if let Some(scope) = &self.write_scope
            && !scope.contains(path)
        {
            return Err(ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                super::scope::outside_write_scope(scope),
            ))));
        }
        self.view
            .admit_workspace_write(&self.checkout_id)
            .map_err(|_| {
                ToolError::NoEffect(Box::new(ToolError::WorkspaceWrite(
                    "the child authority does not admit this write".into(),
                )))
            })?;
        Ok(())
    }
}
