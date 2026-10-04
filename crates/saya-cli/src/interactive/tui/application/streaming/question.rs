//! The turn's question lifecycle at stream end: the scope the turn began
//! with settles or abandons by the outcome the stream ended with — the
//! TUI's mirror of the headless loop's begin/settle/abandon seam
//! (`session_loop`). Runtime-only: nothing here rides the session record.

use super::super::super::agent::messages::Stream;
use crate::interactive::session_runtime::SessionRuntime;
use saya_agent::{AgentEvent, AgentOutput};

/// Settles or abandons the turn's question scope by the outcome its stream
/// ended with. A completed output settles — and only the completed output's
/// typed `ClarificationNeeded` event (never a transient event or a prose
/// substring) makes the question pend; every error — a provider failure,
/// the user's stop, a budget stop — abandons. A stream without a scope
/// settles nothing. The caller reads the stop's confirmation before calling
/// this, so the abandon's token cancel can never reclassify an error as the
/// user's stop.
pub(crate) fn settle_stream_question(
    session: &mut SessionRuntime,
    stream: Option<&Stream>,
    result: &Result<AgentOutput, String>,
) {
    let Some(scope) = stream.and_then(|stream| stream.question_scope.as_ref()) else {
        return;
    };
    match result {
        Ok(output) => {
            let clarification = output
                .events
                .iter()
                .any(|event| matches!(event, AgentEvent::ClarificationNeeded { .. }));
            session.settle_question(scope, clarification);
        }
        Err(_) => session.abandon_question(),
    }
}
