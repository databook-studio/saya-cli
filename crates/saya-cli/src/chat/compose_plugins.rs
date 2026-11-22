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
use crate::plugins::database::{
    ChatDatabaseFlags, DatabaseBudget, DatabasePlugin, LearningSource as DatabasePluginLearning,
    RecallSource as DatabasePluginRecall,
};

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
    /// The database budget a turn of this run spends once it uses the database:
    /// the chat's `[limits.database]`, or the one-shot ask's override of it.
    pub(super) budget: DatabaseBudget,
    /// The provider the run's post-turn learning pass calls. `None` for a run
    /// that learns nothing: the plugin then attaches no learning source and
    /// `on_turn_finish` announces nothing.
    pub(super) learning: Option<Arc<dyn saya_agent::ChatProvider>>,
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
        learning,
        budget,
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
    // What this session's memory mode attaches: the write permission and the
    // observation log the learning pass reads, both absent with memory off.
    let setup_learning = crate::agent::learning::LearningSetup::from(memory);
    // Whether this run has a database side at all: the same fact the plugin's
    // own activation rule reads, taken before the registry moves into the tools.
    let has_profile = !connections.is_empty();
    // The log the tools record the model's own overrides into; the learning
    // pass reads the same one, so an override of a recalled claim is seen.
    let override_log = Arc::new(crate::agent::tools::OverrideLog::new());
    let tools = Arc::new(
        DatabaseTools::with_learning(
            connections,
            runtime.resolved.max_rows,
            allow_query_data,
            Some(application.store().clone()),
            setup_learning.observations.clone(),
        )
        .with_recall_receipt(None, Some(Arc::clone(&override_log))),
    );
    let mut plugin = DatabasePlugin::for_chat(
        Arc::clone(&tools),
        ChatDatabaseFlags {
            allow_query_data,
            // The session keeps the state store the contract tools read and
            // write; no tool of this surface proposes a candidate write.
            has_state_store: true,
            permit_candidate_writes: setup_learning.permit_candidate_writes,
        },
        budget,
        prompt,
        capture,
    );
    // A turn recalls the session's stored knowledge before it is sent, through
    // the one function the chat's turn calls — the same store the session opened,
    // the same bounds its `[memory]` resolved and the same conversation byte
    // budget, over the registry the run's tools query. Off attaches no source at
    // all: nothing is recalled and the turn's receipt stays the empty one. The
    // source states no system prompt because the hook receives only the turn's
    // prompt; the run assembles its system prompt itself, after this point.
    if memory != MemoryMode::Off && has_profile {
        plugin = plugin.with_recall(DatabasePluginRecall::new(
            None,
            allow_query_data,
            runtime.resolved.memory.clone(),
            tools.registry_handle(),
            Some(application.store().clone()),
            runtime.resolved.ai.context_byte_budget,
        ));
    }
    // A turn that produced an answer learns after it, through the one function
    // the chat's turn calls — the same policy the TUI and the plain loop used.
    if let Some(provider) = learning {
        plugin = plugin.with_learning(DatabasePluginLearning::new(
            setup_learning.permit_candidate_writes,
            Arc::clone(&tools),
            setup_learning.observations,
            override_log,
            provider,
            runtime.resolved.ai.model.clone(),
        ));
    }
    let plugin = Arc::new(plugin) as Arc<dyn RunPlugin>;
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
