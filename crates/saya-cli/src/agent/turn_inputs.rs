//! Building a turn's injectable inputs from config — the testability seam for
//! `run_prompt_with_inputs`. Production goes through [`prepare_turn`];
//! tests construct [`TurnInputs`] directly with a mock provider and an idle
//! registry so the recall→emit→provider ordering is assertable without a live
//! database.
//!
//! This is a distinct concern from running the turn (recall + emit + loop),
//! which lives in [`super::runtime`].

use super::{provider, runtime::effective_ai};
use crate::config::runtime::RuntimeConfig;
use crate::connection::ConnectionRegistry;
use saya_agent::ChatProvider;
use saya_config::ResolvedAi;

pub(crate) use super::runtime::AgentRuntimeError;

/// The provider, the resolved AI, and the connection registry a turn runs
/// against — everything [`super::run_prompt_with_inputs`] needs that
/// production builds from config (via [`prepare_turn`]) and tests inject
/// directly (a mock provider, an idle registry) so the recall→emit→provider
/// ordering is assertable without a live database.
pub(crate) struct TurnInputs {
    pub(crate) ai: ResolvedAi,
    pub(crate) provider: Box<dyn ChatProvider>,
    pub(crate) registry: ConnectionRegistry,
    /// Secondaries that failed to connect, surfaced as `skipped database`
    /// assistant-text before recall.
    pub(crate) failures: Vec<(String, String)>,
}

/// Builds [`TurnInputs`] from config: the resolved AI + provider, and the
/// live connection registry (primary plus secondaries). This is the only
/// production caller of `provider::build` and `build_registry`; splitting it
/// out leaves [`super::run_prompt_with_inputs`] provider- and
/// registry-injectable for tests. Cancellation covers the whole turn because
/// [`super::run_prompt_with_sink`] awaits this inside its pinned future.
pub(crate) async fn prepare_turn(
    runtime: &RuntimeConfig,
    overrides: &super::runtime::PromptOverrides,
    can_prompt: bool,
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
            crate::connection::build_registry(
                &runtime.secret_resolver(),
                &runtime.cache_scope,
                runtime.resolved.query_timeout_seconds,
                can_prompt,
                primary_name,
                primary_profile,
                &secondaries,
            )
            .await?
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
