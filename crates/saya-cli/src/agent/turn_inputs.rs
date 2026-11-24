//! Building a turn's injectable inputs from config — the testability seam for
//! `run_prompt_with_inputs`. Production goes through [`prepare_turn`];
//! tests construct [`TurnInputs`] directly with a mock provider and an idle
//! registry so the recall→emit→provider ordering is assertable without a live
//! database.
//!
//! This is a distinct concern from running the turn (recall + emit + loop),
//! which lives in [`super::runtime`].

use super::{provider, turn_config::AgentRuntimeError, turn_config::effective_ai};
use crate::config::runtime::RuntimeConfig;
use crate::connection::ConnectionRegistry;
use crate::local_metrics::LocalMetrics;
use saya_agent::ChatProvider;
use saya_config::ResolvedAi;

/// The provider, the resolved AI, and the connection registry a turn runs
/// against — everything [`super::run_prompt_with_inputs`] needs that
/// production builds from config (via [`prepare_turn`]) and tests inject
/// directly (a mock provider, an idle registry) so the recall→emit→provider
/// ordering is assertable without a live database.
pub(crate) struct TurnInputs {
    pub(crate) ai: ResolvedAi,
    pub(crate) provider: Box<dyn ChatProvider>,
    pub(crate) registry: ConnectionRegistry,
    /// Databases that could not be opened, by name, with the reason: the
    /// secondaries a run skips, and the primary a session opened without —
    /// surfaced as `skipped database` assistant-text before recall.
    pub(crate) failures: Vec<(String, String)>,
}

/// Builds [`TurnInputs`] from config: the resolved AI + provider, and the
/// live connection registry (primary plus secondaries). This is the only
/// production caller of `provider::build` and `build_registry`; splitting it
/// out keeps the line front's turn provider- and registry-injectable for
/// tests. Cancellation covers the whole turn because the turn awaits this
/// inside its pinned future.
pub(crate) async fn prepare_turn(
    runtime: &RuntimeConfig,
    overrides: &super::turn_config::PromptOverrides,
    can_prompt: bool,
    metrics: Option<&LocalMetrics>,
) -> Result<TurnInputs, AgentRuntimeError> {
    let ai = effective_ai(&runtime.resolved.ai, overrides);
    let provider = provider::build(&ai, &runtime.secret_resolver())
        .map_err(|error| AgentRuntimeError::Provider(error.to_string()))?;
    let (profile_name, profile) = super::profile::selected(runtime, overrides.profile.as_ref())?;

    let mut secondaries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in &overrides.included_profiles {
        if name.is_empty() {
            continue;
        }
        if profile_name.as_deref() == Some(name.as_str()) {
            continue;
        }
        if !seen.insert(name) {
            continue;
        }
        if let Ok(sec_profile) = runtime.named_profile(name) {
            secondaries.push((name.clone(), sec_profile.clone()));
        }
    }

    let (registry, failures) = match profile.as_ref() {
        Some(primary_profile) => {
            let primary_name = profile_name.as_deref().unwrap_or("");
            let built = crate::connection::build_registry(
                (&runtime.secret_resolver(), &runtime.cache_scope),
                runtime.resolved.query_timeout_seconds,
                can_prompt,
                primary_name,
                primary_profile,
                &secondaries,
                metrics,
            )
            .await;
            match built {
                Ok(built) => built,
                // A session opens without its database rather than not at all:
                // the profile is named as skipped, with the reason, and the
                // registry stays empty — a session with no database, where the
                // plugin never activates and `/connect` opens a new run over a
                // profile that is there. A command that cannot run without its
                // connection (`saya query`, `saya run`) builds the registry
                // itself and still refuses.
                Err(error) => (
                    crate::connection::ConnectionRegistry::new(primary_name),
                    vec![(primary_name.to_owned(), error.to_string())],
                ),
            }
        }
        None => (crate::connection::ConnectionRegistry::new(""), Vec::new()),
    };
    Ok(TurnInputs {
        ai,
        provider,
        registry,
        failures,
    })
}

#[cfg(test)]
#[path = "turn_inputs_tests.rs"]
mod tests;
