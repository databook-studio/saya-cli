//! The plugin registry one chat session's run is opened with.
//!
//! Everything database-specific is the database plugin's: the section of the
//! run's system prompt, the SQL tools, the recall and the decisions their calls
//! take. This module is the one place a chat run's registry is composed, so the
//! surface it activates for and the databases it activates on are stated once.
//!
//! The registry is composed whether or not the session resolved a database: the
//! plugin's own activation rule reads the run's context, so a chat with no
//! connection activates nothing and contributes zero bytes — while the same
//! registry composed on a run that later states a database needs no second
//! composition path.
//!
//! The primary connection is resolved here too, because it is what a session's
//! "yes for this run" means: a SQL call that names no connection is approved as
//! the primary one, so the grant an answer records is the token that call will
//! ask with next time.

use std::sync::Arc;

use saya_agent::{PluginRegistry, RunContext, RunPlugin, RunSurface};
use saya_config::MemoryMode;

use crate::agent::tools::{CaptureHook, DatabaseTools};
use crate::commands::coding::{CodingApplication, CodingError, NativeCodingJob, RunPlugins};
use crate::connection::ConnectionRegistry;
use crate::plugins::database::prompt::ChatPrompt;
use crate::plugins::database::{ChatDatabaseFlags, DatabasePlugin, SessionBudget};

/// What a session hands the database plugin once, at composition.
pub(super) struct PluginSetup {
    /// The databases the session resolved, primary first.
    pub(super) connections: ConnectionRegistry,
    /// Whether query rows may reach the model.
    pub(super) allow_query_data: bool,
    /// The session's memory mode, and whether it can honour it.
    pub(super) memory: MemoryMode,
    pub(super) memory_reachable: bool,
    /// The session's query-capture hook.
    pub(super) capture: Option<CaptureHook>,
}

/// The registry a session's run activates its plugins out of, and the context
/// they activate for.
pub(super) fn run_plugins(
    application: &CodingApplication,
    job: &NativeCodingJob,
    setup: PluginSetup,
) -> Result<RunPlugins, CodingError> {
    let PluginSetup {
        connections,
        allow_query_data,
        memory,
        memory_reachable,
        capture,
    } = setup;
    let runtime = job.runtime_config();
    // The section is composed from the connection set the run resolved, once,
    // before the registry is moved into the run: a section re-read per turn
    // would move the request prefix a provider's prompt cache is keyed on. The
    // primary is read from the same set, for the same reason — the run's
    // connections are stated once.
    let prompt = ChatPrompt::of(&connections, memory, memory_reachable);
    let context = run_context(&connections);
    let primary = connections.primary().map(str::to_owned);
    let tools = Arc::new(DatabaseTools::with_learning(
        connections,
        runtime.resolved.max_rows,
        allow_query_data,
        Some(application.store().clone()),
        None,
    ));
    let plugin = Arc::new(DatabasePlugin::for_chat(
        tools,
        ChatDatabaseFlags {
            allow_query_data,
            // The session keeps the state store the contract tools read and
            // write; no tool of this surface proposes a candidate write.
            has_state_store: true,
            permit_candidate_writes: false,
        },
        SessionBudget::of(runtime),
        prompt,
        capture,
    )) as Arc<dyn RunPlugin>;
    // The chat's plugin declares database names only, and the core root's own
    // names are composed after this point by the run itself; the plugin filters
    // the core's workspace names out of its own definitions, so no core name can
    // be taken by it.
    let registry = PluginRegistry::new(vec![plugin], &[]).map_err(CodingError::owner)?;
    Ok(RunPlugins {
        registry,
        context,
        primary,
    })
}

/// The run's own context: the chat surface, with the databases it resolved.
fn run_context(connections: &ConnectionRegistry) -> RunContext {
    RunContext {
        surface: RunSurface::Chat,
        database_profiles: connections.len(),
    }
}
