//! The one adapter from the resolved `[limits]` table to the answering engine's
//! vocabulary.
//!
//! Every answering surface reads its ceilings here and nowhere else. The
//! session and the one-shot `ask` each resolve their own set in `saya-config`;
//! this projects the one that applies into [`AnsweringLimits`] and the elapsed
//! deadline the runtime arms. The engine itself is unchanged — one resolution
//! and one vocabulary, one projection per surface.

use crate::config::runtime::RuntimeConfig;
use saya_agent::AnsweringLimits;
use saya_config::ResolvedAnsweringLimits;
use std::time::Duration;

/// Which surface's ceilings apply to a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnsweringSurface {
    /// The interactive session, and the fallback for a scope-less turn.
    Session,
    /// The one-shot `ask` surface.
    Ask,
}

impl AnsweringSurface {
    fn resolved(self, runtime: &RuntimeConfig) -> ResolvedAnsweringLimits {
        match self {
            Self::Session => runtime.resolved.limits.session,
            Self::Ask => runtime.resolved.limits.ask,
        }
    }
}

/// A retired `[limits.session]`/`[limits.ask]` table is still honoured this
/// release, so a table the operator wrote is worth one line naming what to
/// write instead: a bound silently moved to another table is a bound nobody
/// trusts. Said by the surface that reads the retired table — `saya ask` — and
/// not by every run: a per-run line on every stderr would rewrite every
/// scripted expectation for a table that is merely old.
pub(crate) fn warn_retired_tables(runtime: &RuntimeConfig) {
    for diagnostic in &runtime.resolved.limits.diagnostics {
        eprintln!("warning: {diagnostic}");
    }
}

/// The engine's admission ceilings for one surface: requests, tool calls, and
/// known reported tokens, each `None` when the dimension is unlimited.
pub(crate) fn answering_limits(
    runtime: &RuntimeConfig,
    surface: AnsweringSurface,
) -> AnsweringLimits {
    let limits = surface.resolved(runtime);
    AnsweringLimits {
        max_requests: limits.requests.value(),
        max_tool_calls: limits.tool_calls.value(),
        max_reported_tokens: limits.known_tokens.value(),
    }
}

/// The session's reported-spend ceiling in micro-USD, or `None` when the
/// session is unlimited. The session seeds its own field with it at launch;
/// `/usage --limit` may state another for the rest of the session.
pub(crate) fn session_cost_micros(runtime: &RuntimeConfig) -> Option<u64> {
    runtime.resolved.limits.session_cost_micros
}

/// The wall-clock deadline for one surface, or `None` when it is unlimited.
pub(crate) fn answering_deadline(
    runtime: &RuntimeConfig,
    surface: AnsweringSurface,
) -> Option<Duration> {
    surface.resolved(runtime).elapsed_seconds.duration()
}

#[cfg(test)]
#[path = "limits_tests.rs"]
mod tests;
