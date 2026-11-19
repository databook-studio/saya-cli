//! The child surface's plugin registry: the database plugin over the job's own
//! toolset, active for the surface this toolset serves.
//!
//! The factory composes the registry where it used to attach the database
//! toolset directly, so those names have exactly one owner — the plugin — and
//! the executor routes them through it instead of through a second door of its
//! own.

use std::sync::Arc;

use saya_agent::{
    ActivePlugins, PluginRegistry, RunContext, RunPlugin, RunSurface, ToolDefinition,
};
use saya_harness::coding::child::ChildFactoryError;
use saya_harness::coding::policy::ChildAuthorityView;

use crate::commands::coding::database::CodingDatabaseTools;
use crate::plugins::database::DatabasePlugin;

/// The active plugins for one child toolset: the database plugin over
/// `databases`, activated for `surface` with this view's profile count, beside
/// `core_tool_names` — every other name the surface advertises, which the
/// registry refuses to let a plugin take.
pub(super) fn database_plugins(
    databases: &Arc<CodingDatabaseTools>,
    view: &ChildAuthorityView,
    surface: RunSurface,
    core_tool_names: &[String],
) -> Result<ActivePlugins, ChildFactoryError> {
    let core: Vec<&str> = core_tool_names.iter().map(String::as_str).collect();
    let plugin = Arc::new(DatabasePlugin::for_job(
        Arc::clone(databases),
        Some(view.clone()),
    )) as Arc<dyn RunPlugin>;
    let registry = PluginRegistry::new(vec![plugin], &core).map_err(ChildFactoryError::new)?;
    Ok(registry.active(&RunContext {
        surface,
        database_profiles: view.database_profiles().len(),
    }))
}

/// Every non-database tool name this child surface can advertise: the ones
/// composed so far, plus every name the lanes attached after the database
/// plugin can still add. The registry refuses a plugin tool that would take one
/// of them, so this list is what a collision is checked against.
pub(super) fn core_tool_names(
    composed: &[ToolDefinition],
    later: impl IntoIterator<Item = ToolDefinition>,
) -> Vec<String> {
    composed
        .iter()
        .map(|definition| definition.name.clone())
        .chain(later.into_iter().map(|definition| definition.name))
        .collect()
}
