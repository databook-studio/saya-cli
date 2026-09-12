//! The run's per-step toolsets: the composite executor each step's episodes
//! dispatch through, and the definitions that match it — built together from
//! the same capabilities, so what a step advertises and what its calls
//! dispatch through cannot drift apart.
//!
//! The composite dispatches the four fixed harness tool names to their
//! member executors and falls through to `DatabaseTools` for everything
//! else — whose typed `UnsupportedTool` refusal for unknown names makes the
//! fall-through total and predictable. All three tool members are wired:
//! scratch (S1), fetch (S2), and the runner (S3) — each rides the composites
//! of the steps that asked for its scope, and only those, each built from
//! that step's own narrowed scope.

use std::sync::Arc;

use saya_agent::CancellationToken;
use saya_harness::engine::StepToolset;
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchDestination, FetchLimits, FetchPolicy, FetchTools,
    FetchTransport, http_download_definition, http_fetch_definition,
};
use saya_harness::runner::{RunProgram, SharedCredentialSource, StaticCredentialSource};
use saya_harness::scratch::ScratchSql;
use saya_harness::workspace::Workspace;
use saya_types::StepSpec;

use crate::agent::tools::{DatabaseTools, RunTools};

use super::runner::RunRunner;

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

/// The run-level collaborators the toolset builder narrows per step: what
/// the composition root built once per run (the shared database tools, the
/// scope-gated admissions — already `None` unless the run approved the
/// scope — the workspace, the privacy gate, and the cancellation the
/// runner's children answer). The composite each toolset wraps is the
/// shared `RunTools` (`agent::tools`), composed per step from the step's
/// own capabilities, so a step without a member refuses its names as
/// unknown tools before any permit is consulted.
pub(super) struct ToolsetInputs<'a> {
    pub(super) database: &'a Arc<DatabaseTools>,
    pub(super) scratch: Option<&'a Arc<ScratchSql>>,
    pub(super) fetch: Option<&'a RunFetch>,
    pub(super) runner: Option<&'a RunRunner>,
    pub(super) workspace: &'a Arc<Workspace>,
    pub(super) allow_query_data: bool,
    pub(super) cancellation: &'a CancellationToken,
}

/// Builds one toolset per plan step, aligned with the steps, prebuilt by the
/// composition root after the plan binds so admission failures surface
/// before the run starts. Each toolset is built from that step's own
/// capabilities — the thing the approval view shows — so a step that did
/// not ask for a tool never has its definition, and the run-level
/// admissions narrow again per step. The episode driver's own filter stays
/// as the second lock behind construction.
///
/// No state store is passed, so contract tools are absent from a run's
/// universe by construction — a run episode is a synthetic conversation,
/// learning pinned off.
pub(super) fn toolsets(inputs: ToolsetInputs<'_>, steps: &[StepSpec]) -> Vec<StepToolset> {
    let ToolsetInputs {
        database,
        scratch,
        fetch,
        runner,
        workspace,
        allow_query_data,
        cancellation,
    } = inputs;
    // The credential seam a runner member carries: nothing declares runner
    // credentials yet, so the resolver resolves nothing — the seam is here
    // so a declaration surface rides the same construction.
    let resolver: SharedCredentialSource = Arc::new(StaticCredentialSource::new(Vec::new()));
    steps
        .iter()
        .map(|step| {
            let mut definitions = DatabaseTools::definitions(
                allow_query_data,
                false,
                false,
                step.capabilities.workspace_write,
            );
            let scratch = scratch.filter(|_| step.capabilities.scratch);
            if scratch.is_some() {
                definitions.push(ScratchSql::definition());
            }
            // The step's fetch member: its own policy built from the
            // destinations *this step* declared — never the run's union —
            // over the run's shared transport, workspace, and wallet.
            let fetch = fetch.filter(|_| step.capabilities.fetch.is_some());
            if fetch.is_some() {
                definitions.push(http_fetch_definition());
                definitions.push(http_download_definition());
            }
            // The step's runner member: the run's proven spawn shared by
            // clone (cloning grants nothing), the *step's* narrowed scopes
            // — never the run's union — the resolved default timeout, and
            // the run's cancellation. Both doors ride the one tool: the
            // step's `RunnerScope` opens the runner door, its
            // `InterpreterScope` the interpreter door, and each is `None`
            // when the step did not ask, so a step that asked for one never
            // holds the other.
            let runner = runner
                .filter(|_| {
                    step.capabilities.runner.is_some() || step.capabilities.interpreter.is_some()
                })
                .map(|wiring| {
                    let runner_scope = step.capabilities.runner.clone();
                    let interpreter_scope = step.capabilities.interpreter.clone();
                    if runner_scope.is_none() && interpreter_scope.is_none() {
                        unreachable!(
                            "the builder only builds a runner member for a step that asked"
                        );
                    }
                    Arc::new(
                        RunProgram::for_step(
                            wiring.spawn.clone(),
                            runner_scope,
                            interpreter_scope,
                            wiring.timeout,
                            Arc::clone(&resolver),
                        )
                        .with_cancellation(cancellation.clone()),
                    )
                });
            if let Some(runner) = &runner {
                definitions.push(runner.definition());
            }
            StepToolset {
                executor: Arc::new(RunTools::compose(
                    Arc::clone(database),
                    scratch.cloned(),
                    fetch.map(|run_fetch| {
                        Arc::new(fetch_tools(run_fetch, &step.capabilities, workspace))
                    }),
                    runner,
                )),
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
