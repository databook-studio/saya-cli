//! What a session op besides the run's own does on the port: the continuation a
//! paused turn asks for, the rollover a context change performs on the run the
//! operator is talking to, and the shared operation a read hands off the loop's
//! thread.
//!
//! A context change is the line's own operation, performed on the line's thread
//! ([`super::line_surface`]): `/clear` and `/compact` are the chat layer's
//! rollover and go there as they are, and a state change — the model, the
//! provider, the connection, the privacy setting — rolls the open run into a
//! successor composed from the state the command stated, seeded with the
//! conversation so far; before the first turn it gives the run up instead, as
//! the line front gives up the run it prepared ahead of its first line. What
//! stays here is the port's side: the words the session-command layer stated,
//! said now, and the operation handed on.

use std::sync::Arc;

use super::msg::Worker;
use super::ops::Asked;
use super::run_cmd_port::SessionPort;
use super::run_front::Forward;
use crate::chat::{ContextChange, RolloverRequest};
use crate::interactive::session::line_chat::Said;

/// Says what the line said after a turn crossed its boundary, in the surface's
/// own voice: the compaction's line, the failure that latched it, or nothing.
pub(crate) fn settle(front: &Forward, said: Said) {
    match said {
        Said::Message(words) => front.note(words),
        Said::Error(words) => front.deliver(Worker {
            direct: Some(super::direct::DirectOutcome::Error(words)),
            ..Worker::default()
        }),
        Said::Nothing => {}
    }
}

impl SessionPort {
    /// A context change the chat layer states as a rollover: on the live run,
    /// through the line's own operations, which wait for a turn in flight —
    /// exactly as the line front's rule says. The state this surface keeps has
    /// already been moved by the session-command layer, and the words it stated
    /// are said first.
    pub(super) fn roll(&self, request: RolloverRequest, said: String) {
        if !said.is_empty() {
            self.front.say(said);
        }
        match request.change {
            ContextChange::Clear => self.line.clear(),
            ContextChange::Compact | ContextChange::Rotate => self.line.compact(),
            // A record is opened by the resume operation both surfaces share; the
            // rollover table states it as `Asked::Resume` before this arm, so this
            // one only keeps the match total.
            ContextChange::Resume => self.resume(request.value.unwrap_or_default()),
            // The run already open cannot absorb one: its request, its prompt
            // prefix and its provider were composed once, at open
            // (`chat::rollover`). The line rolls it into a successor composed
            // from the new state, seeded with the conversation so far.
            ContextChange::Connect
            | ContextChange::Include
            | ContextChange::Exclude
            | ContextChange::Provider
            | ContextChange::Model
            | ContextChange::Privacy => {
                self.line.restate(self.state().prompt_overrides());
                self.restated_row();
            }
        }
    }

    /// The session restated what it is on: the status row names the model and
    /// the database the session states from now on, not the ones the launch
    /// resolved. The facts are the session's own (`status_segments`, the view the
    /// launch paints its facts from), never a second reading of the command.
    fn restated_row(&self) {
        let state = self.state();
        let view = crate::interactive::session_prompt::status_segments(&state);
        self.front.restated(view.model, state.profile.clone());
        // The model the session is on now is the one whose window the meter
        // divides by; an unknown window keeps the row quiet rather than guessing.
        let runtime = self.deps.runtime.current();
        self.front
            .set_window(super::live::context_window(&runtime, &state.model));
    }

    /// One shared operation: it takes as long as it takes, so it runs on a
    /// blocking task and hands its entry back through the front.
    pub(super) fn work(&self, work: super::ops_work::Work) {
        let deps = Arc::clone(&self.deps);
        let front = Arc::clone(&self.front);
        tokio::task::spawn_blocking(move || match super::ops_work::perform(&deps, work) {
            Asked::Notice(words) => front.deliver(Worker {
                direct: Some(super::direct::DirectOutcome::Notice(words)),
                ..Worker::default()
            }),
            Asked::Error(words) => front.deliver(Worker {
                direct: Some(super::direct::DirectOutcome::Error(words)),
                ..Worker::default()
            }),
            // A shared read answers with words only; anything else is the
            // caller's, never this thread's.
            _ => {}
        });
    }
}
