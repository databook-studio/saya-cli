//! The child composition boundary: the factory composes one child's provider,
//! toolset, definitions and approval from ONE immutable authority view. The
//! approval decider wraps the child's coarse effect gate with exact write
//! admission, so a refused write is denied before execution; the executor
//! keeps the same admission as defense in depth.

use std::sync::Arc;

use saya_config::{ResolvedAi, SecretResolver};
use saya_harness::coding::child::{
    ChildComponents, ChildExecutionSpec, ChildFactory, ChildFactoryError, OwnerFailureSnapshot,
};
use saya_harness::coding::policy::ChildAuthorityView;

use super::{ChildTools, TrustedCheckoutPair, child_toolset, decider};
use crate::agent::provider;
use crate::agent::tools::consent::OwnedToolApproval;
use crate::agent::tools::shell_policy::ShellTool;
use crate::commands::coding::database::CodingDatabaseTools;

/// Composes one verified, confined child's runtime dependencies from pieces
/// already prepared by the composition root: a trusted checkout pairing, the
/// resolved AI configuration, and the secret resolver providers need.
///
/// The approval decider is the child's coarse effect gate wrapped with exact
/// write-path admission (`decider::ExactWriteChildApproval`): a refused write
/// is denied before execution, so it admits no permit and settles no
/// uncertain effect; the executor keeps the same admission as defense in
/// depth.
pub(crate) struct CliChildFactory {
    /// The trusted pairing of the exact checkout id with the exact workspace
    /// root; the factory never re-derives it from model input.
    pair: TrustedCheckoutPair,
    /// The resolved AI configuration; the child request's model replaces its
    /// model before the provider is built.
    ai: ResolvedAi,
    /// The resolver `provider::build` uses to turn secret references into
    /// provider credentials.
    resolver: Arc<dyn SecretResolver>,
    /// The helper's `shell`, built by the composition root over the shared
    /// working checkout with this helper's scratch tree and subject line, or
    /// `None` when the job's config and mode allow no shell for this helper.
    shell: Option<Arc<ShellTool>>,
    /// The helper's web lane, built by the composition root over the job's own
    /// egress with this helper's subject line, or `None` when the job's mode
    /// reaches no host.
    web: Option<Arc<crate::agent::tools::web::WebTools>>,
    /// The helper's own MCP lane: the job's configured servers, started for
    /// this helper and closed when it ends. `None` where the job's mode
    /// reaches no server or the configuration names none.
    mcp: Option<Arc<crate::mcp::client::McpTools>>,
    /// The job's database surface, when it selected a profile. The factory
    /// attaches it only where the view admits at least one of them.
    databases: Option<Arc<CodingDatabaseTools>>,
    coding: Option<Arc<crate::commands::coding::checks::ChildCodingOperations>>,
    /// The job's `[tools] allow` set as a helper sees it: the job's list minus
    /// `helpers`, which only the root may call. Unstated narrows nothing, so a
    /// job that states no list composes exactly what it always did.
    tools: saya_config::ResolvedTools,
    /// The job's hook engine, when the user configured hooks: the same engine
    /// the root runs, over the same working checkout. A helper's calls are the
    /// job's calls, so a `PreToolUse` block covers them too.
    hooks: Option<Arc<crate::agent::hooks::Hooks>>,
    /// The in-place job's approved `--write` scope, enforced on every write
    /// door this factory composes. `None` is a managed checkout.
    write_scope: Option<saya_harness::coding::integration::AssignedPaths>,
    /// The run's read-witness store, shared with the shell composed beside
    /// this toolset.
    read_witnesses: Option<Arc<crate::agent::tools::workspace::ReadWitnesses>>,
    /// The job's own shell-output spool directory, the one place outside the
    /// workspace this helper's `read` and `grep` may reach.
    spool_dir: Option<std::path::PathBuf>,
}

impl CliChildFactory {
    /// Narrows this helper to the job's tool set. Called by the composition
    /// root, which holds the job: a helper never resolves a list of its own.
    pub(crate) fn with_tools(mut self, tools: saya_config::ResolvedTools) -> Self {
        self.tools = tools;
        self
    }

    /// Puts the job's hook engine in front of this helper's dispatch path.
    /// `None` — the job configured no hooks, or composed no engine — leaves
    /// the helper's executor exactly as it was.
    pub(crate) fn with_hooks(mut self, hooks: Option<Arc<crate::agent::hooks::Hooks>>) -> Self {
        self.hooks = hooks;
        self
    }

    pub(crate) fn with_databases(mut self, databases: CodingDatabaseTools) -> Self {
        self.databases = Some(Arc::new(databases));
        self
    }

    pub(crate) fn with_shell(mut self, shell: Option<Arc<ShellTool>>) -> Self {
        self.shell = shell;
        self
    }

    /// Joins this helper's workspace tools to the same read-witness store its
    /// shell records into.
    pub(in crate::agent::tools) fn with_read_witnesses(
        mut self,
        witnesses: Arc<crate::agent::tools::workspace::ReadWitnesses>,
    ) -> Self {
        self.read_witnesses = Some(witnesses);
        self
    }

    /// Attaches the job's own shell-output spool directory.
    pub(crate) fn with_spool_dir(mut self, dir: Option<std::path::PathBuf>) -> Self {
        self.spool_dir = dir;
        self
    }

    pub(crate) fn with_web(mut self, web: Option<Arc<crate::agent::tools::web::WebTools>>) -> Self {
        self.web = web;
        self
    }

    /// Puts the job's MCP lane beside this helper's other egressing tools.
    /// `None` — the job's mode reaches no server, or none is configured —
    /// leaves the helper's toolset exactly as it was.
    pub(crate) fn with_mcp(mut self, mcp: Option<Arc<crate::mcp::client::McpTools>>) -> Self {
        self.mcp = mcp;
        self
    }

    pub(crate) fn with_coding(
        mut self,
        operations: Arc<crate::commands::coding::checks::ChildCodingOperations>,
    ) -> Self {
        self.coding = Some(operations);
        self
    }

    /// Bounds every write this factory composes to the in-place job's
    /// approved `--write` scope. `None` leaves a managed checkout unbounded.
    pub(crate) fn with_write_scope(
        mut self,
        scope: Option<saya_harness::coding::integration::AssignedPaths>,
    ) -> Self {
        self.write_scope = scope;
        self
    }

    /// Assembles the factory from pieces prepared by `compose_child` in the
    /// same use-time verification flow.
    pub(crate) fn new(
        pair: TrustedCheckoutPair,
        ai: ResolvedAi,
        resolver: Arc<dyn SecretResolver>,
    ) -> Self {
        Self {
            pair,
            ai,
            resolver,
            shell: None,
            web: None,
            mcp: None,
            coding: None,
            databases: None,
            tools: saya_config::ResolvedTools::default(),
            hooks: None,
            write_scope: None,
            read_witnesses: None,
            spool_dir: None,
        }
    }
}

impl CliChildFactory {
    /// Builds the definitions, the concrete `ChildTools` executor, and the
    /// `ExactWriteChildApproval` decider for one authority view, WITHOUT the
    /// provider. This is the toolset half a child and the direct-mode root
    /// share; `build` adds the provider on top for a child run. The
    /// approval decider wraps the child's coarse effect gate with exact
    /// write-path admission, so an unassigned write is denied before any
    /// permit is admitted; the executor's identical admission stays as defense
    /// in depth.
    pub(crate) fn build_toolset(
        &self,
        view: &ChildAuthorityView,
    ) -> Result<
        (
            Vec<saya_agent::ToolDefinition>,
            ChildTools,
            decider::ExactWriteChildApproval,
        ),
        ChildFactoryError,
    > {
        let (mut definitions, tools) =
            child_toolset(view, &self.pair).map_err(ChildFactoryError::new)?;
        let mut tools = tools.with_write_scope(self.write_scope.clone());
        if let Some(witnesses) = &self.read_witnesses {
            tools = tools.with_read_witnesses(Arc::clone(witnesses));
        }
        tools = tools.with_spool_dir(self.spool_dir.clone());
        if let Some(coding) = &self.coding {
            super::super::workspace::canonicalize_tool_names(&mut definitions);
            definitions.extend(tools.attach_coding(Arc::clone(coding)));
        }
        // Attached when it advertises anything at all: a profile this view
        // admits, or the clarification ask an operator can answer — which
        // needs no profile and so survives an empty selection.
        if let Some(databases) = &self.databases
            && (!view.database_profiles().is_empty() || databases.advertises_clarification())
        {
            definitions.extend(databases.definitions(Some(view)));
            tools.attach_databases(Arc::clone(databases));
        }
        // The helper's shell is advertised only when the composition root
        // built one: config off, mode `never`, or no proven sandbox under
        // `read-only` all mean no shell at all.
        if let Some(shell) = &self.shell {
            definitions.extend(tools.attach_shell(Arc::clone(shell)));
        }
        // The helper's web lane is advertised only when the composition root
        // built one: a mode that reaches no host advertises none at all, and
        // the lane itself states whether it also holds `web_search`.
        if let Some(web) = &self.web {
            definitions.extend(tools.attach_web(Arc::clone(web)));
        }
        // The helper's MCP lane is advertised only when the composition root
        // built one: a mode that reaches no server composes none, and neither
        // does a configuration that names none. Appended after the lanes that
        // existed before it, so the tools that did keep the positions they
        // had.
        if let Some(mcp) = &self.mcp {
            definitions.extend(tools.attach_mcp(Arc::clone(mcp)));
        }
        // The notebook edit was appended after the workspace block, and the
        // database, shell and web lanes above were appended after it: it is
        // moved back to the end so the tools that existed before it keep the
        // positions they had, and the request's cached prefix stays intact.
        super::super::workspace::notebook_edit_last(&mut definitions);
        // The job's `[tools] allow` list, applied after the composition above
        // so it can only ever take a tool away, and with `helpers` already
        // removed: only the root delegates. The executor carries the same set,
        // so its dispatch gate and this advertisement cannot drift.
        let tools_allow = self.tools.for_helper();
        crate::agent::tools::narrow(&mut definitions, &tools_allow);
        tools.narrow_to(tools_allow);
        tools.hold_definitions(&definitions);
        let approval = decider::ExactWriteChildApproval::new(view, self.pair.checkout_id().clone())
            .with_write_scope(self.write_scope.clone());
        Ok((definitions, tools, approval))
    }
}

impl ChildFactory for CliChildFactory {
    /// Builds the provider, toolset, definitions and approval for one child
    /// from its immutable authority view.
    ///
    /// The approval decider wraps the child's coarse effect gate with exact
    /// write-path admission, so an unassigned write is denied before any
    /// permit is admitted; the executor's identical admission stays as
    /// defense in depth.
    fn build(
        &self,
        spec: &ChildExecutionSpec,
        _cancellation: &saya_agent::CancellationToken,
    ) -> Result<ChildComponents, ChildFactoryError> {
        let view = spec.authority();
        let (mut definitions, mut tools, approval) = self.build_toolset(view)?;
        // The notebook edit is offered only where the checkout actually holds
        // a notebook. A helper's list is fixed for its whole run, so the
        // bounded presence check is made once, here — and the executor carries
        // the same answer, so a call to the edit while the list leaves it out
        // is refused as an unknown tool. The direct-mode root shares
        // `build_toolset` but not this gate: its list is re-read every turn,
        // so it keeps the definition and filters it at advertisement
        // (`DirectCodingTools::advertises_notebook`).
        let notebooks = super::super::workspace::notebook::present(Some(self.pair.root.as_ref()));
        if !notebooks {
            super::super::workspace::notebook::drop_notebook_edit(&mut definitions);
        }
        tools.offer_notebook_edit(notebooks);
        // The request's own evidence decides the optional tools: `list`,
        // `delete`, the web pair and `helpers` are advertised only when it asks
        // for them or the user named them in `[tools] allow`.
        tools.apply_evidence(crate::agent::tools::advertisement::Evidence::new(
            spec.request().prompt.clone(),
            self.web.as_ref().is_some_and(|web| web.search_configured()),
        ));
        // The helper's list is the one it has reached: the composed definitions
        // minus the notebook edit where the checkout holds none, and minus the
        // background readers until a run exists. The executor re-states it
        // before each request, so the loop and the dispatch agree.
        let definitions = tools.advertisement();
        let mut ai = self.ai.clone();
        ai.model = spec.request().model.clone();
        let provider =
            provider::build(&ai, self.resolver.as_ref()).map_err(ChildFactoryError::new)?;
        // A `shell` call is decided by the shell's own lane policy — host
        // versus sandbox, and who is asked — and a web call by the tool's own
        // consent rule, while every other call keeps the child's exact-write
        // answer.
        let mut owners: Vec<Arc<dyn crate::agent::tools::consent::OwnApproval>> = Vec::new();
        if let Some(shell) = &self.shell {
            owners.push(Arc::clone(shell) as Arc<dyn crate::agent::tools::consent::OwnApproval>);
        }
        if let Some(web) = &self.web {
            owners.push(Arc::clone(web) as Arc<dyn crate::agent::tools::consent::OwnApproval>);
        }
        // An MCP call is decided by the lane's own consent rule — the mode,
        // and who is asked — exactly as a root's is, never by the child's
        // coarse effect gate.
        if let Some(mcp) = &self.mcp {
            owners.push(Arc::clone(mcp) as Arc<dyn crate::agent::tools::consent::OwnApproval>);
        }
        let approval: Box<dyn saya_agent::ApprovalDecider> = if owners.is_empty() {
            Box::new(approval)
        } else {
            Box::new(OwnedToolApproval::new(owners, Arc::new(approval)))
        };
        // The job's hooks sit outermost, exactly as they do on the root: every
        // check the child's own executor makes — the authority view, the
        // `[tools] allow` list, the exact-write admission — has already run by
        // the time a `PreToolUse` hook sees the call, so a helper's hook can
        // refuse a call and can never let one through.
        let executor: Box<dyn saya_agent::ToolExecutor> = match self.hooks.as_ref() {
            Some(hooks) => Box::new(crate::agent::hooks::HookGate::new(
                Arc::clone(hooks),
                Arc::new(tools),
            )),
            None => Box::new(tools),
        };
        Ok(ChildComponents::new(
            provider,
            executor,
            definitions,
            approval,
        ))
    }

    fn owner_failure(&self) -> Option<OwnerFailureSnapshot> {
        self.coding
            .as_ref()
            .and_then(|operations| operations.owner_failure.snapshot())
    }
}
