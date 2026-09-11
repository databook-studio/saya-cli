//! Shared assembly of the engine's collaborators: the pieces a fresh run and
//! a resume both need, built from config the way `ask` builds a turn's —
//! the provider from the run's `orchestrator` endpoint, the connection
//! registry from the resolved profile, the executor over the database tools.
//!
//! Runs are headless by construction: the approval decider never prompts
//! (`can_prompt` false), so an `ask`-mode approval denies, `never` denies,
//! and the default is read-only — read-shaped tools run, anything needing an
//! interactive decision or an external side effect is denied. The run's
//! scopes (not the per-call decider) are the approval surface for
//! capabilities.

use crate::agent::runtime::query_data_allowed;
use crate::agent::tools::DatabaseTools;
use crate::agent::{profile, provider};
use crate::config::runtime::RuntimeConfig;
use crate::connection;
use crate::prompt_approval::TerminalApproval;
use saya_agent::{ChatProvider, ToolDefinition};

/// How much workspace the episode brief's manifest walks: names, sizes,
/// digests — never bulk contents. Conservative brief bounds, fixed here so
/// every run brief is built the same way; the workspace's own byte and file
/// budgets are a plan-contract concern.
const MANIFEST_MAX_FILES: usize = 512;
const MANIFEST_MAX_FILE_BYTES: u64 = 256 * 1024;

/// The manifest bounds every run brief is built with.
pub(super) fn manifest_bounds() -> saya_harness::engine::ManifestBounds {
    saya_harness::engine::ManifestBounds {
        max_files: MANIFEST_MAX_FILES,
        max_file_bytes: MANIFEST_MAX_FILE_BYTES,
    }
}

/// Everything the engine's drivers need that the composition root builds
/// once per run or resume: the provider, the executor, the approval decider,
/// the model the episodes call, and the run's full tool universe (the
/// episode driver narrows it per step).
pub(super) struct Pieces {
    pub(super) provider: Box<dyn ChatProvider>,
    pub(super) tools: DatabaseTools,
    pub(super) decider: TerminalApproval,
    pub(super) model: String,
    pub(super) profile_names: Vec<String>,
    pub(super) universe: Vec<ToolDefinition>,
}

/// Assembles the pieces. The model is the `orchestrator` endpoint's — the
/// role every run has — resolved over the `[ai]` block exactly the way
/// endpoint resolution layers it. Errors are configuration or connection
/// problems (exit-code class 3), reported as text for the caller to emit.
pub(super) async fn assemble(
    runtime: &RuntimeConfig,
    scopes: &saya_types::Capabilities,
    workspace: std::sync::Arc<saya_harness::workspace::Workspace>,
    approval: saya_agent::ApprovalPolicy,
) -> Result<Pieces, String> {
    let endpoint = runtime
        .resolved
        .endpoints
        .get(saya_config::ORCHESTRATOR_ROLE)
        .cloned()
        .ok_or_else(|| "no orchestrator endpoint is resolved; check the [ai] config".to_string())?;
    let mut ai = runtime.resolved.ai.clone();
    ai.provider = endpoint.provider;
    ai.model = endpoint.model.clone();
    ai.base_url = endpoint.base_url.clone();
    ai.api_key = endpoint.api_key.clone();
    let provider =
        provider::build(&ai, &runtime.secret_resolver()).map_err(|error| error.to_string())?;
    let (profile_name, profile) =
        profile::selected(runtime, None).map_err(|error| error.to_string())?;
    let (registry, _failures) = match profile.as_ref() {
        Some(primary) => connection::build_registry(
            &runtime.secret_resolver(),
            &runtime.cache_scope,
            runtime.resolved.query_timeout_seconds,
            false,
            profile_name.as_deref().unwrap_or(""),
            primary,
            &[],
        )
        .await
        .map_err(|error| error.to_string())?,
        None => (connection::ConnectionRegistry::new(""), Vec::new()),
    };
    let allow_query_data = query_data_allowed(ai.provider, ai.allow_data_sharing);
    let profile_names = registry
        .names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let tools = DatabaseTools::with_learning(
        registry,
        runtime.resolved.max_rows,
        allow_query_data,
        None,
        None,
    )
    .with_workspace(Some(workspace));
    // The universe is every tool this build can advertise: the episode
    // driver hides a step's unapproved ones. No state store is passed, so
    // contract tools are absent from a run's universe by construction — a
    // run episode is a synthetic conversation, learning pinned off.
    let universe =
        DatabaseTools::definitions(allow_query_data, false, false, scopes.workspace_write);
    Ok(Pieces {
        provider,
        tools,
        decider: TerminalApproval::new(approval, false),
        model: ai.model,
        profile_names,
        universe,
    })
}
