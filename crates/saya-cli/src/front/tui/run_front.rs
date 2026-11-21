//! The session's front: what the session says, as messages on the queue.
//!
//! A chat session talks to exactly one thing, and this is the surface's end of
//! it (`chat::ChatFront`). Everything the session says — a prepared run, a call
//! to the operator, one engine event, a turn's end — lands on the queue as
//! [`Msg::Chat`], so the loop folds it in the order the session said it.
//!
//! The one thing a queue cannot carry is an *answer*: a question is put with a
//! key press and resumed when the press happens, possibly many messages later.
//! The front therefore keeps the waiting ends by the question's own id, and
//! [`Cmd::Answer`](super::msg::Cmd::Answer) resolves the one it names.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use super::msg::{Msg, Worker};
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, Question};

/// The surface one session drives: its events in, its calls answered.
pub(crate) struct Forward {
    msgs: UnboundedSender<Msg>,
    /// The asks waiting for an answer, by the id they were put under.
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
}

impl Forward {
    /// A front over the loop's queue.
    pub(crate) fn new(msgs: UnboundedSender<Msg>) -> Self {
        Self {
            msgs,
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Hands one session event to the loop. False when the loop is gone.
    fn send(&self, event: ChatEvent) -> bool {
        self.msgs.send(Msg::Chat(event)).is_ok()
    }

    /// Says something in the surface's own voice: the queue's worker slot, which
    /// the update shows as a toast. Nothing said is nothing to show.
    pub(crate) fn say(&self, message: impl Into<String>) {
        let message = message.into();
        if !message.is_empty() {
            let _ = self.msgs.send(Msg::Worker(Worker {
                message,
                ..Worker::default()
            }));
        }
    }

    /// Hands one finished worker's answer to the loop: the rows a direct
    /// command returned, the sentence it ended in, or its own word about
    /// itself — one `Msg::Worker`, so a slow command can never read as a key.
    pub(crate) fn deliver(&self, worker: Worker) {
        let _ = self.msgs.send(Msg::Worker(worker));
    }

    /// Answers the call asked under `id`, which resumes the ask that is waiting
    /// for it. False when nothing waits there: the card was answered twice, or
    /// the asker gave up.
    pub(crate) fn answer(&self, id: u64, answer: Answer) -> bool {
        let waiting = self.pending().remove(&id);
        match waiting {
            Some(respond) => respond.send(answer).is_ok(),
            None => false,
        }
    }

    /// The asks waiting for an answer. A poisoned lock is the map as it was: a
    /// panicking asker must not take the session's answers with it.
    fn pending(&self) -> MutexGuard<'_, HashMap<u64, oneshot::Sender<Answer>>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl ChatFront for Forward {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(Sink {
            msgs: self.msgs.clone(),
        })
    }

    async fn ask(&self, question: &Question) -> Answer {
        let (respond, answer) = oneshot::channel();
        self.pending().insert(question.id, respond);
        if !self.send(ChatEvent::Question(question.clone())) {
            // Nobody is left to read the card, so nobody will answer it: the
            // ask denies rather than waiting for a queue that is gone.
            self.pending().remove(&question.id);
            return Answer::Deny;
        }
        answer.await.unwrap_or(Answer::Deny)
    }

    fn emit(&self, event: ChatEvent) {
        let _ = self.send(event);
    }
}

/// The engine's own stream, as messages: one [`Msg::Chat`] per event, so the
/// turn's text and tool steps reach the same queue as the keys and can never be
/// mistaken for a press.
struct Sink {
    msgs: UnboundedSender<Msg>,
}

#[async_trait]
impl AgentEventSink for Sink {
    async fn emit(&self, event: AgentEvent) {
        let _ = self.msgs.send(Msg::Chat(ChatEvent::Agent(event)));
    }
}
