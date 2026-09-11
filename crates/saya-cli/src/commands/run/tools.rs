//! The run's per-step toolsets: the composite executor each step's episodes
//! dispatch through, and the definitions that match it — built together from
//! the same capabilities, so what a step advertises and what its calls
//! dispatch through cannot drift apart.
//!
//! The composite dispatches the four fixed harness tool names to their
//! member executors and falls through to `DatabaseTools` for everything
//! else — whose typed `UnsupportedTool` refusal for unknown names makes the
//! fall-through total and predictable. The scratch member is wired (S1) and
//! the fetch member (S2): each rides the composites of the steps that asked
//! for its scope, and only those. The runner member lands with its wiring
//! slice (S3).

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::engine::StepToolset;
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchDestination, FetchLimits, FetchPolicy, FetchTools,
    FetchTransport, http_download_definition, http_fetch_definition,
};
use saya_harness::scratch::ScratchSql;
use saya_harness::workspace::Workspace;
use saya_types::StepSpec;

use crate::agent::tools::DatabaseTools;

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
            // The still-unwired harness name reaches the database tools,
            // which refuse it with the typed error the single executor has
            // always returned: fail closed, byte-identical. Wired by its
            // tool's slice (S3 `runner`).
            "run_program" => self.database.execute(name, arguments).await,
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// Builds one toolset per plan step, aligned with the steps, prebuilt by the
/// composition root after the plan binds so admission failures surface
/// before the run starts. Each toolset is built from that step's own
/// capabilities — the thing the approval view showed — so a step that did
/// not ask for a tool never has its definition, and the run-level
/// admissions (scratch db, fetch transport and wallet — already `None`
/// unless the run approved the scope) narrow again per step. The episode
/// driver's own filter stays as the second lock behind construction.
///
/// No state store is passed, so contract tools are absent from a run's
/// universe by construction — a run episode is a synthetic conversation,
/// learning pinned off.
pub(super) fn toolsets(
    database: &Arc<DatabaseTools>,
    scratch: Option<&Arc<ScratchSql>>,
    fetch: Option<&RunFetch>,
    workspace: &Arc<Workspace>,
    allow_query_data: bool,
    steps: &[StepSpec],
) -> Vec<StepToolset> {
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
            StepToolset {
                executor: Arc::new(RunTools {
                    database: Arc::clone(database),
                    scratch: scratch.cloned(),
                    fetch: fetch.map(|run_fetch| {
                        Arc::new(fetch_tools(run_fetch, &step.capabilities, workspace))
                    }),
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
