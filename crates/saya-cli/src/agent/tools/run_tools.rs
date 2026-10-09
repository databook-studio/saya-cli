//! The run-tool composite executor: the four fixed harness tool names route
//! to their member executors, everything else falls through to the shared
//! `DatabaseTools` — whose typed `UnsupportedTool` refusal for unknown names
//! makes the fall-through total and predictable. One composite serves both
//! surfaces that carry the run tool members (a run's per-step toolsets and an
//! interactive session), so the dispatch never forks. The loop-assigned
//! `ToolExecutionContext` — the per-call result cap and the turn's
//! cancellation token — rides with the call wherever a member honours it:
//! the scratch and fetch members and the database fall-through receive it
//! through `execute_with_context`, so a cancelled turn reaches every lane it
//! touches. The runner keeps its own composed token; `execute` and
//! `execute_with_result_cap` mint fresh, never-cancelled tokens.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutionContext, ToolExecutor};

use super::DatabaseTools;
use super::host_argv::validate_host_json_argv;
use saya_agent::CancellationToken;
use saya_harness::fetch::FetchTools;
use saya_harness::runner::RunProgram;
use saya_harness::scratch::ScratchSql;

#[cfg(test)]
#[path = "run_tools_cancel_tests.rs"]
mod run_tools_cancel_tests;

/// The composite executor. Each optional member is present only where its
/// surface composed it: a surface without a member refuses that member's
/// names as unknown tools, before any permit is consulted — the
/// hidden-not-advertised discipline applied at dispatch, so an advertised
/// tool and its executor cannot drift apart. The loop-assigned context is
/// forwarded to every member that honours the turn's cancellation — scratch,
/// fetch, and the database fall-through; the runner drives its own composed
/// token instead.
pub(crate) struct RunTools {
    database: Arc<DatabaseTools>,
    chart_save_permit: bool,
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
    /// The host lane, present only where the launch stated it and a
    /// workspace root bound. Its absence refuses `run_command` as an
    /// unknown tool.
    host: Option<HostCommandMember>,
    /// The session deny list: bare program names refused before grant,
    /// prompt, and bypass at every program-named door. Runs never carry it
    /// (deny is session-shaped); an empty list refuses nothing.
    deny: crate::interactive::session::permissions::deny::SessionDeny,
    /// The session's live task list, present only where a session composed
    /// it. Its absence refuses `tasks_set` as an unknown tool — a run has
    /// no session list to write.
    tasks: Option<crate::interactive::session::tasks::SessionTasks>,
    /// The session journal, when this executor belongs to a session: a deny
    /// firing is journalled there before the refusal is relayed, and a host
    /// call before the child spawns. `None` — test shapes — refuses without
    /// journaling.
    journal: Option<std::sync::Arc<saya_store::SessionJournal>>,
}

include!("run_tools/composition.rs");
include!("run_tools/dispatch.rs");
include!("run_tools/host_command.rs");
