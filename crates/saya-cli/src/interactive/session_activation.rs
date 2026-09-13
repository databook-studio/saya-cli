//! The bypass activation line: the one visible event when the mode takes
//! effect, in the product's no-euphemism register — the grammar's own word,
//! never "yolo", "danger", or a softened "auto". Emitted at launch, at
//! `/approvals bypass`, and re-printed on resume, always through the
//! existing notice/message paths, so every surface says the same words.
//!
//! The line composes the session's facts (DESIGN §1, §5): the mode's
//! meaning, the interpreter facts — the staged names under the run surface's
//! warning adapted to the session, or the none-staged sentence — and, where
//! the probe refused this host, the same fact the notice seam carries.

use crate::approval_text::interpreter_warning;
use crate::config::runtime::RuntimeConfig;
use crate::interactive::session_runner::PROBE_REFUSED_NOTICE;
use crate::interactive::session_state::SessionState;
use crate::interactive::session_universe::SessionUniverse;

/// The mode fact: what bypass is, and what it does not touch.
const BYPASS_ON: &str =
    "bypass on: every tool call runs without asking; every structural guard still applies.";

/// The interpreter fact when nothing is staged in the trusted config
/// (DESIGN §5, verbatim): the mode's honesty about what it does *not* open.
pub(crate) const NO_INTERPRETERS_STAGED: &str =
    "no interpreters are staged in [jobs.interpreter] allow, so interpreter calls still refuse.";

/// The session interpreter warning's process-fork clause — the measured
/// fact, in place of the run surface's conditional parenthetical, which is
/// false for sessions (`session_runner.rs` grants no process-fork; a forked
/// child dies with `fork: Operation not permitted`, `sandbox/mod.rs`).
const SESSION_FORK_FACT: &str =
    "no process-fork is granted: children an interpreter spawns are refused by the sandbox.";

/// The activation line for a bypass session: the mode fact, then — when
/// interpreters are staged — the shared warning sentence naming them in the
/// session's wording, or the none-staged sentence instead, and the probe's
/// verdict where it refused.
pub(crate) fn bypass_line(staged_interpreters: &[String], probe_refused: bool) -> String {
    let mut line = String::from(BYPASS_ON);
    if staged_interpreters.is_empty() {
        line.push(' ');
        line.push_str(NO_INTERPRETERS_STAGED);
    } else {
        line.push('\n');
        line.push_str(&interpreter_warning(
            "session",
            staged_interpreters,
            SESSION_FORK_FACT,
        ));
    }
    if probe_refused {
        line.push('\n');
        line.push_str(PROBE_REFUSED_NOTICE);
    }
    line
}

/// Whether the session's mode parses to bypass — the one decision every
/// emission site (launch, `/approvals bypass`, resume) consults, so the
/// three cannot drift. An unparseable mode is never bypass.
pub(crate) fn is_bypass_mode(state: &SessionState) -> bool {
    state.approval_mode.parse() == Ok(saya_agent::ApprovalPolicy::Bypass)
}

/// The activation line when the session's mode is bypass, `None` otherwise —
/// the one call every emission site makes (launch, `/approvals bypass`,
/// resume), so the three surfaces cannot drift into different words.
pub(crate) fn line_if_bypass(
    state: &SessionState,
    runtime: &RuntimeConfig,
    universe: &SessionUniverse,
) -> Option<String> {
    is_bypass_mode(state).then(|| {
        bypass_line(
            &runtime.resolved.jobs.interpreter.allow,
            universe.probe_refused,
        )
    })
}

#[cfg(test)]
#[path = "session_activation_tests.rs"]
mod tests;
