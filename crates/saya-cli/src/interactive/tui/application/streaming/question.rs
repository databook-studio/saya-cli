//! The turn's question lifecycle at stream end: the scope the turn began
//! with settles or abandons by the outcome the stream ended with — the
//! TUI's mirror of the headless loop's begin/settle/abandon seam
//! (`session_loop`). Runtime-only: nothing here rides the session record.

use super::super::super::agent::messages::Stream;
use super::super::super::types::App;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use saya_agent::{AgentEvent, AgentOutput};

impl App {
    /// Drains any queued agent-stream messages into the transcript. Returns
    /// true when the request just finished (so the caller can persist the
    /// session). Test-only seam: the tests drive the drain without a
    /// session runtime, so the turn's question lifecycle is not settled
    /// here — the session-holding tick uses `drain_stream_with_session`.
    #[cfg(test)]
    pub(crate) fn drain_stream(&mut self, state: &mut SessionState) -> bool {
        self.drain_inner(state, None)
    }

    /// Drains and settles the turn's question scope with the session
    /// runtime — the production tick's path (`tick_busy` holds the session):
    /// the scope the turn began with settles or abandons when the stream
    /// ends.
    pub(crate) fn drain_stream_with_session(
        &mut self,
        state: &mut SessionState,
        session: &mut SessionRuntime,
    ) -> bool {
        self.drain_inner(state, Some(session))
    }
}

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
    // The question's collector, when it carried one, is snapshotted at
    // settlement on every outcome — Done ok, err, or cancel — so the
    // print-after-restore seam can say what the turn measured. A question
    // without a collector records nothing.
    // A user-requested stop records its one cancellation-to-local-settlement
    // sample here, before the snapshot below, so the printed section
    // includes the settlement measure. No captured instant records nothing.
    scope.record_stop_settlement();
    if let Some(collector) = scope.metrics() {
        session.remember_question_metrics(collector.snapshot());
    }
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
