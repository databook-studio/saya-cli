//! `/usage`: the session's one usage panel, and the ceiling it states.
//!
//! Both fronts answer through [`answer`], so the words, the window the context
//! share is measured against, and the ceiling `--limit` sets cannot differ
//! between them. Nothing here probes: every figure is one the session already
//! holds, and the window is the configuration's.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::state::SessionState;
use crate::slash::UsageRequest;

/// The window of `model` in force: the configured override, otherwise the
/// built-in table's row for that model. One function, because `/usage`,
/// `/status` and the status row's meter divide the last request by the same
/// denominator — and the model is the session's, which `/model` moves, never
/// the launch-time model the runtime still names.
pub(crate) fn window_tokens(runtime: &RuntimeConfig, model: &str) -> Option<u64> {
    runtime
        .resolved
        .ai
        .context_window_tokens
        .or_else(|| saya_config::context_window_tokens(model))
}

/// The panel, and the ceiling in force after the request: `--limit <usd>`
/// states this session's ceiling, and bare shows the one already in force.
pub(crate) fn answer(
    state: &mut SessionState,
    runtime: &RuntimeConfig,
    request: UsageRequest,
) -> String {
    if let Some(micros) = request.limit_micros {
        state.session_cost_limit_micros = Some(micros);
    }
    state.usage.panel(
        window_tokens(runtime, &state.model),
        state.session_cost_limit_micros,
    )
}

#[cfg(test)]
#[path = "ops_usage_tests.rs"]
mod tests;
