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
//!
//! A job's calls are answered here too (`02` §5, §8): its worker reports with
//! today's `CodingMessage`, `super::jobs_desk` reads it as a row or as a
//! question, and the waiting end is the plumbing's own — so one card answers
//! the session's calls and a job's alike.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEvent, AgentEventSink};
use std::sync::Arc;
use tokio::sync::{mpsc::UnboundedSender, oneshot};

use super::jobs_desk::JobDesk;
use super::msg::{Msg, Worker};
use crate::agent::tools::consent::Answer;
use crate::chat::{ChatEvent, ChatFront, ChatOutcome, Question};
use crate::front::line::{SettledTurn, StreamSink, TurnStream};
use crate::interactive::tui::CodingPanel;

/// The surface one session drives: its events in, its calls answered.
pub(crate) struct Forward {
    pub(super) msgs: UnboundedSender<Msg>,
    /// The asks waiting for an answer, by the id they were put under.
    pending: Mutex<HashMap<u64, oneshot::Sender<Answer>>>,
    /// The run's jobs as today's job plumbing states them, and the waiting end
    /// of every call one of them has put to the operator (`02` §8). A job is one
    /// of this surface's workers, and its calls are answered by the same card.
    pub(super) desk: Mutex<JobDesk>,
    /// The panel a job of this run reports through, once one is running.
    pub(super) panel: Mutex<Option<CodingPanel>>,
    /// This turn's stream, read the way the line front reads its own: the
    /// answer the operator saw and what the turn spent, taken once at the
    /// turn's settlement for the session's transcript and books.
    stream: Arc<TurnStream>,
    /// How the last turn ended, as the session stated it: taken once at the same
    /// settlement, so the boundary a turn crosses knows what the turn was.
    ended: Mutex<Option<ChatOutcome>>,
}

impl Forward {
    /// A front over the loop's queue.
    pub(crate) fn new(msgs: UnboundedSender<Msg>) -> Self {
        Self {
            msgs,
            pending: Mutex::new(HashMap::new()),
            desk: Mutex::new(JobDesk::default()),
            panel: Mutex::new(None),
            stream: Arc::new(TurnStream::default()),
            ended: Mutex::new(None),
        }
    }

    /// The turn that just settled, as this front read it off the stream: how it
    /// ended, what it said, what it spent. `None` when the session stated no end
    /// — a turn the run refused, or one still queued.
    pub(crate) fn take_settled(&self) -> Option<SettledTurn> {
        let outcome = self
            .ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()?;
        Some(SettledTurn {
            outcome,
            record: self.stream.take(),
        })
    }

    /// How the last settled turn ended, without taking it: the loop's own
    /// bookkeeping reads it after the boundary has crossed.
    pub(crate) fn last_outcome(&self) -> Option<ChatOutcome> {
        self.ended
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The run's jobs, as today's job plumbing states them.
    pub(super) fn desk_lock(&self) -> MutexGuard<'_, JobDesk> {
        self.desk.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The panel a job of this run reports through.
    pub(super) fn panel(&self) -> MutexGuard<'_, Option<CodingPanel>> {
        self.panel.lock().unwrap_or_else(PoisonError::into_inner)
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

    /// Puts one line in the transcript, in the surface's own voice (`02`
    /// §3.10): a job's control receipt, the words it settled with, or the note
    /// a deprecated spelling owes.
    pub(crate) fn note(&self, line: String) {
        self.deliver(Worker {
            direct: Some(super::direct::DirectOutcome::Notice(line)),
            ..Worker::default()
        });
    }

    /// Hands one finished worker's answer to the loop: the rows a direct
    /// command returned, the sentence it ended in, or its own word about
    /// itself — one `Msg::Worker`, so a slow command can never read as a key.
    pub(crate) fn deliver(&self, worker: Worker) {
        let _ = self.msgs.send(Msg::Worker(worker));
    }

    /// Asks the loop to leave: the one message that ends it (`/exit`). A job of
    /// this run is not orphaned by a quit: the surface says what the old one
    /// said (`interactive::tui::JOB_RUNNING`) and leaves the job to settle, so a
    /// cancelled helper's durable state is written before the process ends.
    pub(crate) fn quit(&self) {
        if self.panel().as_ref().is_some_and(CodingPanel::is_active) {
            self.say(crate::interactive::tui::JOB_RUNNING);
            return;
        }
        let _ = self.msgs.send(Msg::Quit);
    }

    /// Answers the call asked under `id`, which resumes the ask that is waiting
    /// for it. False when nothing waits there: the card was answered twice, or
    /// the asker gave up.
    pub(crate) fn answer(&self, id: u64, answer: Answer) -> bool {
        let waiting = self.pending().remove(&id);
        match waiting {
            Some(respond) => respond.send(answer).is_ok(),
            // A job's call is put to the operator through the same card, and its
            // waiting end is today's plumbing's own (`02` §5, §8).
            None => self.desk_lock().answer(id, answer),
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
        // The turn's stream is read before it is rendered: the session's
        // transcript and books read what the operator saw and what the turn
        // spent, and the loop paints every event unchanged and in order.
        Arc::new(StreamSink::new(
            Arc::new(Sink {
                msgs: self.msgs.clone(),
            }),
            Arc::clone(&self.stream),
        ))
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

    /// Every ask is painted as a card and answered by the key that takes it,
    /// so this surface can obtain a per-call approval: that is the fact a
    /// write-shaped tool's advertisement is decided from, and stating it false
    /// would hide every write-shaped tool the operator asked for (the old
    /// surface stated the same fact through its modal).
    fn can_obtain_approval(&self) -> bool {
        true
    }

    fn emit(&self, event: ChatEvent) {
        // The turn's end is the settlement's own fact: recorded before the event
        // is handed on, so the boundary that follows the turn reads it.
        if let ChatEvent::TurnEnded(outcome) = &event {
            *self.ended.lock().unwrap_or_else(PoisonError::into_inner) = Some(outcome.clone());
        }
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
