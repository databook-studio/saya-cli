//! The run's per-step toolsets: the composite executor each step's episodes
//! dispatch through, and the definitions that match it — built together from
//! the same capabilities, so what a step advertises and what its calls
//! dispatch through cannot drift apart.
//!
//! The composite dispatches the four fixed harness tool names to their
//! member executors and falls through to `DatabaseTools` for everything
//! else — whose typed `UnsupportedTool` refusal for unknown names makes the
//! fall-through total and predictable. This seam slice wires no member
//! executor: every step's toolset is the shared `DatabaseTools` behind a
//! fresh composite, advertising the universe the run's scopes always built.
//! Member executors land with their tool's wiring slice (S1 `scratch`, S2
//! `fetch`, S3 `runner`).

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::engine::StepToolset;
use saya_types::Capabilities;

use crate::agent::tools::DatabaseTools;

/// The run's composite executor: the four fixed harness tool names route to
/// member executors, everything else falls through to the shared
/// `DatabaseTools`.
struct RunTools {
    database: Arc<DatabaseTools>,
}

#[async_trait]
impl ToolExecutor for RunTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match name {
            // The four fixed harness names dispatch to their member
            // executors. None is wired in this seam slice — each arrives
            // with its tool's wiring slice (S1 `scratch`, S2 `fetch`, S3
            // `runner`) — so for now every name, the harness ones included,
            // reaches the database tools, which refuse unknown names with
            // the typed error the single executor has always returned:
            // fail closed, byte-identical.
            "scratch_sql" | "http_fetch" | "http_download" | "run_program" => {
                self.database.execute(name, arguments).await
            }
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// Builds one toolset per plan step, aligned with `plan.steps`, prebuilt by
/// the composition root after the plan binds so admission failures surface
/// before the run starts. In this seam slice every step's toolset is the
/// same: the shared `DatabaseTools` behind a fresh composite, advertising
/// the definitions the run's scopes always built — the episode driver still
/// narrows them to the step's capabilities.
///
/// No state store is passed, so contract tools are absent from a run's
/// universe by construction — a run episode is a synthetic conversation,
/// learning pinned off.
pub(super) fn toolsets(
    database: &Arc<DatabaseTools>,
    allow_query_data: bool,
    scopes: &Capabilities,
    steps: usize,
) -> Vec<StepToolset> {
    let definitions =
        DatabaseTools::definitions(allow_query_data, false, false, scopes.workspace_write);
    (0..steps)
        .map(|_| StepToolset {
            executor: Arc::new(RunTools {
                database: Arc::clone(database),
            }),
            definitions: definitions.clone(),
        })
        .collect()
}
