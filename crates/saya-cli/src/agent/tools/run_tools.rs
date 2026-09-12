//! The run-tool composite executor: the four fixed harness tool names route
//! to their member executors, everything else falls through to the shared
//! `DatabaseTools` — whose typed `UnsupportedTool` refusal for unknown names
//! makes the fall-through total and predictable. One composite serves both
//! surfaces that carry the run tool members (a run's per-step toolsets and an
//! interactive session), so the dispatch never forks.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutor};

use super::DatabaseTools;
use saya_harness::fetch::FetchTools;
use saya_harness::runner::RunProgram;
use saya_harness::scratch::ScratchSql;

/// The composite executor. Each optional member is present only where its
/// surface composed it: a surface without a member refuses that member's
/// names as unknown tools, before any permit is consulted — the
/// hidden-not-advertised discipline applied at dispatch, so an advertised
/// tool and its executor cannot drift apart.
pub(crate) struct RunTools {
    database: Arc<DatabaseTools>,
    /// The scratch database, present only where scratch was composed. Its
    /// absence refuses `scratch_sql` as an unknown tool.
    scratch: Option<Arc<ScratchSql>>,
    /// The fetch member, present only where fetch was composed. Its absence
    /// refuses both fetch names as unknown tools.
    fetch: Option<Arc<FetchTools>>,
    /// The runner member, present only where a proven spawn exists. Its
    /// absence refuses `run_program` as an unknown tool — an unproven host
    /// never has the tool at all.
    runner: Option<Arc<RunProgram>>,
}

impl RunTools {
    /// Composes the members into the one executor. Every surface that
    /// advertises a harness tool dispatches through this constructor — there
    /// is no second assembly path.
    pub(crate) fn compose(
        database: Arc<DatabaseTools>,
        scratch: Option<Arc<ScratchSql>>,
        fetch: Option<Arc<FetchTools>>,
        runner: Option<Arc<RunProgram>>,
    ) -> Self {
        Self {
            database,
            scratch,
            fetch,
            runner,
        }
    }
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
