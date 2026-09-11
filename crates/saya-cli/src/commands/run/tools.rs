//! The run's per-step toolsets: the composite executor each step's episodes
//! dispatch through, and the definitions that match it — built together from
//! the same capabilities, so what a step advertises and what its calls
//! dispatch through cannot drift apart.
//!
//! The composite dispatches the four fixed harness tool names to their
//! member executors and falls through to `DatabaseTools` for everything
//! else — whose typed `UnsupportedTool` refusal for unknown names makes the
//! fall-through total and predictable. The scratch member is wired (S1):
//! the shared `ScratchSql` rides the composites of the steps that asked for
//! scratch, and only those. The other members land with their tool's wiring
//! slice (S2 `fetch`, S3 `runner`).

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::engine::StepToolset;
use saya_harness::scratch::ScratchSql;
use saya_types::StepSpec;

use crate::agent::tools::DatabaseTools;

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
            // The still-unwired harness names reach the database tools, which
            // refuse them with the typed error the single executor has always
            // returned: fail closed, byte-identical. Each is wired by its
            // tool's slice (S2 `fetch`, S3 `runner`).
            "http_fetch" | "http_download" | "run_program" => {
                self.database.execute(name, arguments).await
            }
            _ => self.database.execute(name, arguments).await,
        }
    }
}

/// Builds one toolset per plan step, aligned with the steps, prebuilt by the
/// composition root after the plan binds so admission failures surface
/// before the run starts. Each toolset is built from that step's own
/// capabilities — the thing the approval view showed — so a step that did
/// not ask for a tool never has its definition, and the run-level `scratch`
/// admission (already `None` unless the run approved the scope) narrows
/// again per step. The episode driver's own filter stays as the second
/// lock behind construction.
///
/// No state store is passed, so contract tools are absent from a run's
/// universe by construction — a run episode is a synthetic conversation,
/// learning pinned off.
pub(super) fn toolsets(
    database: &Arc<DatabaseTools>,
    scratch: Option<&Arc<ScratchSql>>,
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
            StepToolset {
                executor: Arc::new(RunTools {
                    database: Arc::clone(database),
                    scratch: scratch.cloned(),
                }),
                definitions,
            }
        })
        .collect()
}
