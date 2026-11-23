//! What a session op besides the run's own does on the port: the continuation a
//! paused turn asks for, the rollover a context change states, and the shared
//! operation a read hands off the loop's thread.

use std::sync::Arc;

use super::msg::Worker;
use super::ops::{Asked, rollover_boundary};
use super::run_cmd_port::SessionPort;

impl SessionPort {
    /// `/continue`: the paused turn's continuation message, through the chat
    /// session's own path — or the one refusal, sending nothing.
    pub(super) fn continue_turn(&self) {
        let session = Arc::clone(&self.session);
        let front = Arc::clone(&self.front);
        tokio::spawn(async move {
            if let Err(refusal) = session.continue_turn().await {
                front.say(refusal.sentence());
            }
        });
    }

    /// A context change the chat layer states as a rollover. The session's own
    /// command layer has already moved the state this surface keeps; what a
    /// live run can still read of it is stated plainly, so the operator is
    /// never told a conversation changed when the run it is on has not rolled.
    pub(super) fn roll(&self, request: crate::chat::RolloverRequest, said: String) {
        if !said.is_empty() {
            self.front.say(said);
        }
        self.front.say(rollover_boundary(&request));
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
