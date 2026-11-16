//! The run's per-step toolsets: the composite executor each step's episodes
//! dispatch through, and the definitions that match it — built together from
//! the same capabilities, so what a step advertises and what its calls
//! dispatch through cannot drift apart.
//!
//! The composite dispatches the four fixed harness tool names to their
//! member executors and falls through to `DatabaseTools` for everything
//! else — whose typed `UnsupportedTool` refusal for unknown names makes the
//! fall-through total and predictable. The scope-gated member is wired the
//! same way — fetch rides the composite of the steps that asked for its
//! scope, and only those, each built from that step's own narrowed
//! scope. The command door is no scope: every step carries `shell`, the same
//! tool the coding root advertises, composed once per run over the run's
//! workspace.

use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_harness::engine::StepToolset;
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchDestination, FetchLimits, FetchPolicy, FetchTools,
    FetchTransport, http_download_definition, http_fetch_definition,
};
use saya_harness::runner::HostPolicy;
use saya_harness::workspace::Workspace;
use saya_types::StepSpec;

use super::step_tools::RunStepTools;
use crate::agent::tools::consent::Prompt;
use crate::agent::tools::shell_policy::{self, Background, ShellPlacement, ShellTool};
use crate::agent::tools::{DatabaseTools, RunTools};

/// The run-level fetch wiring, built once per run at assemble when — and
/// only when — the run approved a fetch scope: the shared transport and the
/// run's download wallet, whose clones go both into the fetch-capable
/// steps' members and into the sink's pause check (clone-shares-state, so a
/// trip anywhere is seen everywhere). `None` means nothing fetch-shaped
/// exists for the run and the sink's download check is inert.
pub(super) struct RunFetch {
    pub(super) transport: Arc<dyn FetchTransport>,
    pub(super) budget: DownloadBudget,
}

/// Builds the run's `shell`: the same tool the coding root advertises
/// (`agent::tools::shell_policy`), composed over the run's workspace. `None`
/// where the mode is `never`, or where the mode is `read-only` and this host
/// proves no sandbox — a read-only run has no shell that could reach outside
/// one. `operator` is the approver a host call under `ask` reaches; a
/// headless run has none, so a host call falls back to the sandbox when one
/// is proven and is refused with a reason when none is.
pub(super) fn run_shell(
    workspace: &Arc<Workspace>,
    policy: ApprovalPolicy,
    prompt: Option<Prompt>,
    host: HostPolicy,
    cancellation: CancellationToken,
    rules: saya_config::CommandRules,
) -> Option<Arc<ShellTool>> {
    if !shell_policy::advertised(policy) {
        return None;
    }
    shell_policy::compose(
        ShellPlacement::root(workspace.root()),
        policy,
        prompt,
        host,
        cancellation,
        // A run's shell outlives its steps but not its process: the family
        // stays the coding root's, where a job owns what it started.
        Background::Off,
        rules,
    )
    .map(Arc::new)
}

/// The run-level collaborators the toolset builder narrows per step: what
/// the composition root built once per run (the shared database tools, the
/// scope-gated admissions — already `None` unless the run approved the
/// scope — the run's shell, the workspace, and the privacy gate). The
/// composite each toolset wraps is the shared `RunTools` (`agent::tools`),
/// composed per step from the step's own capabilities, so a step without a
/// member refuses its names as unknown tools before any permit is consulted.
pub(super) struct ToolsetInputs<'a> {
    pub(super) database: &'a Arc<DatabaseTools>,
    pub(super) fetch: Option<&'a RunFetch>,
    /// The run's `shell`, built once per run. `None` means this run has no
    /// shell at all (mode `never`, or read-only with no proven sandbox), so
    /// no step advertises it.
    pub(super) shell: Option<&'a Arc<ShellTool>>,
    /// The run's MCP lane, built once per run from the user's configured
    /// servers. `None` means the run's mode reaches no server, or the
    /// configuration names none, so no step advertises an `mcp__` tool.
    pub(super) mcp: Option<&'a Arc<crate::mcp::client::McpTools>>,
    pub(super) workspace: &'a Arc<Workspace>,
    pub(super) allow_query_data: bool,
    /// The one `[tools] allow` list this invocation resolved. Applied after
    /// each step's composition, so it can only take a member away: a name it
    /// holds that this step never composed stays refused, exactly as before.
    pub(super) tools: &'a saya_config::ResolvedTools,
    /// The run's hook engine, built once per run from the user's `[hooks]`
    /// table. `None` where the user configured none, or where the run's
    /// admitted mode keeps no host lane — a hook is a host command, and a
    /// `read-only` or `never` run has no lane to run one in.
    pub(super) hooks: Option<&'a Arc<crate::agent::hooks::Hooks>>,
}

/// Builds one toolset per plan step, aligned with the steps, prebuilt by the
/// composition root after the plan binds so admission failures surface
/// before the run starts. Each toolset is built from that step's own
/// capabilities — the thing the approval view shows — so a step that did
/// not ask for a scope never has its definition, and the run-level
/// admissions narrow again per step. The episode driver's own filter stays
/// as the second lock behind construction. The run's `shell` is the one
/// member every step carries, and the run's hooks — when the user stated
/// any — sit in front of every step's composite.
///
/// No state store is passed, so contract tools are absent from a run's
/// universe by construction — a run episode is a synthetic conversation,
/// learning pinned off.
pub(super) fn toolsets(inputs: ToolsetInputs<'_>, steps: &[StepSpec]) -> Vec<StepToolset> {
    let ToolsetInputs {
        database,
        fetch,
        shell,
        mcp,
        workspace,
        allow_query_data,
        tools: tools_allow,
        hooks,
    } = inputs;
    // The notebook edit is offered only where the run's workspace actually
    // holds a notebook. A step's list is fixed for the run — the toolsets are
    // built once, before the first episode — so the bounded presence check is
    // made once here, before any step is composed, and a workspace with no
    // notebook pays the model no context for the tool. `read` of a `.ipynb`
    // keeps working either way; a call to the edit while it is not offered is
    // refused as an unknown tool, the same refusal a member this run never
    // composed gets.
    let notebooks = crate::agent::tools::notebook_present(Some(workspace.as_ref()));
    steps
        .iter()
        .map(|step| {
            // A run's approval is its approved scopes: the database surface
            // and the step's own write permit. The `render_chart` model tool
            // is advertised nowhere, so no egress-derived permit is
            // consulted here.
            let mut definitions = DatabaseTools::definitions(
                allow_query_data,
                false,
                false,
                step.capabilities.workspace_write,
            );
            // The step's fetch member: its own policy built from the
            // destinations *this step* declared — never the run's union —
            // over the run's shared transport, workspace, and wallet.
            let fetch = fetch.filter(|_| step.capabilities.fetch.is_some());
            if fetch.is_some() {
                definitions.push(http_fetch_definition());
                definitions.push(http_download_definition());
            }
            // The step's command door: the run's one shell, shared by clone
            // (cloning grants nothing — the composition already decided what
            // lanes exist), advertised by every step.
            if shell.is_some() {
                definitions.push(shell_policy::definition());
            }
            // The configured servers' tools, appended after the members that
            // existed before them: a mode that reaches no server composed no
            // lane, and the lane's own reports say which servers failed.
            if let Some(mcp) = mcp {
                definitions.extend(mcp.definitions());
            }
            // A run step's notebook edit is a member of the step's composition,
            // not a per-turn decision: this run's workspace holds no notebook,
            // so the step does not offer it at all. Dropping the definition
            // leaves every other tool's position — and so the request's cached
            // prefix — exactly as it was.
            if !notebooks {
                crate::agent::tools::drop_notebook_edit(&mut definitions);
            }
            crate::agent::tools::narrow(&mut definitions, tools_allow);
            // The step's own goal is its request text: `list` and the other
            // default-suppressed tools appear only when it asks for them, or
            // when the user named them in `[tools] allow`.
            let evidence =
                crate::agent::tools::advertisement::Evidence::new(step.goal.clone(), false);
            definitions.retain(|definition| {
                crate::agent::tools::advertisement::advertised(
                    &evidence,
                    tools_allow,
                    &definition.name,
                )
            });
            let step_tools: Arc<dyn ToolExecutor> = Arc::new(RunStepTools {
                shell: shell.cloned(),
                mcp: mcp.cloned(),
                notebook_edit: notebooks,
                evidence,
                tools: tools_allow.clone(),
                inner: Arc::new(
                    RunTools::compose(
                        Arc::clone(database),
                        fetch.map(|run_fetch| {
                            Arc::new(fetch_tools(run_fetch, &step.capabilities, workspace))
                        }),
                    )
                    .with_tools(tools_allow.clone()),
                ),
            });
            // The run's hooks sit outermost, so the step's own refusals — a
            // member this step never composed, a name the list removed, the
            // shell's own lane decision — have all already happened by the
            // time a `PreToolUse` hook sees the call. A hook can refuse a
            // call here; it can never let one through.
            let executor = match hooks {
                Some(hooks) => Arc::new(crate::agent::hooks::HookGate::new(
                    Arc::clone(hooks),
                    step_tools,
                )) as Arc<dyn ToolExecutor>,
                None => step_tools,
            };
            StepToolset {
                executor,
                definitions,
            }
        })
        .collect()
}

/// The step's `FetchTools`: its policy from its own `FetchScope`'s
/// destinations, the lane-bounded fetch limits, and the run's shared
/// download wallet by clone.
fn fetch_tools(
    run_fetch: &RunFetch,
    capabilities: &saya_types::Capabilities,
    workspace: &Arc<Workspace>,
) -> FetchTools {
    let scope = capabilities
        .fetch
        .as_ref()
        .expect("the builder only builds a fetch member for a step that asked");
    let policy = FetchPolicy::new(
        scope
            .destinations
            .iter()
            .map(|destination| FetchDestination::new(&destination.scheme, &destination.host)),
    );
    FetchTools::new(
        policy,
        Arc::clone(&run_fetch.transport),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::clone(workspace)),
        run_fetch.budget.clone(),
    )
}
