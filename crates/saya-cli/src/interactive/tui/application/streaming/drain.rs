//! Agent streaming lifecycle.

use super::super::super::agent::StreamMsg;
use super::super::super::stream_events::apply_event;
use super::super::super::transcript::BlockKind;
use super::super::super::types::{
    App, LastQuery, MAX_PENDING_QUERIES, PendingApproval, PendingQuery,
};
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::state::SessionState;
use crate::interactive::tui::capture_agent::{AgentCaptureOutcome, promote_agent_capture};
use crate::render::tool_groups::is_failure_summary;
use saya_agent::{AgentEvent, ProviderRecoveryPhase, UsageCall};

impl App {
    /// The drain loop proper: pulls the stream's channel messages into the
    /// transcript. The two entry points — with and without the session
    /// runtime — live in `question.rs`, beside the question settle they
    /// drive.
    pub(super) fn drain_inner(
        &mut self,
        state: &mut SessionState,
        mut session: Option<&mut SessionRuntime>,
    ) -> bool {
        let mut messages = Vec::new();
        let mut sender_gone = false;
        if let Some(stream) = self.request.stream.as_mut() {
            loop {
                match stream.rx.try_recv() {
                    Ok(msg) => messages.push(msg),
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                        sender_gone = true;
                        break;
                    }
                }
            }
        }
        let prompt = self
            .request
            .stream
            .as_ref()
            .map(|s| s.prompt.clone())
            .unwrap_or_default();
        let drained_work = !messages.is_empty();
        let mut finished = self.drain_messages(messages, state, prompt.clone(), &mut session);
        // A worker that died without ever posting `Done` left the stream
        // busy forever: no settle, `request.stream` stuck Some. Settle once,
        // exactly as a `Done(Err(..))` arm would, on the first drained-empty
        // tick after the sender dropped — a batch that carried work without
        // a `Done` is a mid-turn tick the stream keeps — and the tick loop
        // re-drains within the frame, so the wedge cannot persist.
        if !finished && sender_gone && !drained_work {
            self.settle_done(
                Err("the agent worker stopped unexpectedly".to_string()),
                state,
                prompt.clone(),
                session.take(),
            );
            finished = true;
        }
        if finished {
            self.request.pending_approval = None;
            self.request.stream = None;
            self.request.started = None;
            self.request.activity = None;
            // The turn is over: no pending candidate may survive into the
            // next turn, where it could be promoted by an unmatched
            // completion. The desynchronised mark resets with the FIFO:
            // the next turn pairs from a trusted FIFO again.
            self.pending_queries.clear();
            self.pending_queries_desync = false;
            // No unmatched capture survives the turn either: its completion
            // never came, and pairing it later would attach a foreign result.
            self.agent_captures.clear_queue();
            // The numerator belongs to the turn that reported it; the next
            // turn must not show this one's figure if the provider goes
            // silent.
            self.request.last_answering_input = None;
            self.request.known_answering_usage = None;
        }
        // No forced scroll: when the user is at the bottom the newest lines show
        // automatically; when they've scrolled up to read, streaming leaves them.
        finished
    }
}

/// The connection an unnamed agent `bounded_sql_query` runs on: the session's
/// active profile, or — when none is selected — the profile the session runs
/// on (the runtime's resolved default). Both are readable here, on `App`, at
/// request time; an unnamed query promoted into `LastQuery` must carry it so
/// `/investigation save` can save "the connection that actually ran it".
fn session_connection(state: &SessionState, app: &App) -> Option<String> {
    state
        .profile
        .clone()
        .or_else(|| app.runtime.resolved.profile_name.clone())
}

include!("drain/messages.rs");
