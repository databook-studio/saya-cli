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

use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::engine::StepToolset;
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchDestination, FetchLimits, FetchPolicy, FetchTools,
    FetchTransport, http_download_definition, http_fetch_definition,
};
use saya_harness::runner::RunProgram;
use saya_harness::runner::{SharedCredentialSource, StaticCredentialSource};
use saya_harness::scratch::ScratchSql;
use saya_harness::workspace::Workspace;
use saya_types::StepSpec;

use crate::agent::tools::DatabaseTools;

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

/// The run's composite executor: the four fixed harness tool names route to
/// member executors, everything else falls through to the shared
/// `DatabaseTools`.
struct RunTools {
    database: Arc<DatabaseTools>,
    /// The run's scratch database, present in this step's composite only
    /// when the step's capabilities asked for scratch. Its absence makes the
    /// narrowing real: a step without scratch refuses the name as an unknown
    /// tool, before any permit is consulted.
    scratch: Option<Arc<ScratchSql>>,
    /// The step's fetch member, present only when the step's capabilities
    /// asked for fetch, over the run's shared wiring. Its absence refuses
    /// both fetch names as unknown tools, before any permit is consulted.
    fetch: Option<Arc<FetchTools>>,
    /// The step's runner member, present only when the step's capabilities
    /// asked for the runner, over the run's proven spawn. Its absence
    /// refuses `run_program` as an unknown tool, before any permit is
    /// consulted — an unproven host never has the tool at all.
    runner: Option<Arc<RunProgram>>,
}

#[async_trait]
impl ToolExecutor for RunTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match name {
            "scratch_sql" => match &self.scratch {
                Some(scratch) => scratch.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "http_fetch" | "http_download" => match &self.fetch {
                Some(fetch) => fetch.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            "run_program" => match &self.runner {
                Some(runner) => runner.execute(name, arguments).await,
                None => Err(ToolError::UnsupportedTool),
            },
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// The run-level collaborators the toolset builder narrows per step: what
/// the composition root built once per run (the shared database tools, the
/// scope-gated admissions — already `None` unless the run approved the
/// scope — the workspace, the privacy gate, and the cancellation the
/// runner's children answer).
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
            // clone (cloning grants nothing), the *step's* narrowed
            // `RunnerScope` — never the run's union — the resolved default
            // timeout, and the run's cancellation.
            let runner = runner
                .filter(|_| step.capabilities.runner.is_some())
                .map(|wiring| {
                    let scope =
                        step.capabilities.runner.as_ref().expect(
                            "the builder only builds a runner member for a step that asked",
                        );
                    Arc::new(
                        RunProgram::new(
                            wiring.spawn.clone(),
                            scope.clone(),
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
                executor: Arc::new(RunTools {
                    database: Arc::clone(database),
                    scratch: scratch.cloned(),
                    fetch: fetch.map(|run_fetch| {
                        Arc::new(fetch_tools(run_fetch, &step.capabilities, workspace))
                    }),
                    runner,
                }),
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
        Arc::clone(workspace),
        run_fetch.budget.clone(),
    )
}
