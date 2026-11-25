//! `/usage`: the session's one usage panel, and the ceiling it states.
//!
//! Both fronts answer through [`answer`], so the words, the window the context
//! share is measured against, and the ceiling `--limit` sets cannot differ
//! between them. Nothing here probes: every figure is one the session already
//! holds, and the window is the configuration's.

use crate::config::runtime::RuntimeConfig;
use crate::interactive::session::state::SessionState;
use crate::slash::UsageRequest;

/// The model's context window in force: the configured override, otherwise the
/// built-in table's row for the model. One function, because `/usage` and
/// `/status` divide the last request by the same denominator.
pub(crate) fn window_tokens(runtime: &RuntimeConfig) -> Option<u64> {
    runtime
        .resolved
        .ai
        .context_window_tokens
        .or_else(|| saya_config::context_window_tokens(&runtime.resolved.ai.model))
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
    state
        .usage
        .panel(window_tokens(runtime), state.session_cost_limit_micros)
}
