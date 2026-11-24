//! A front that records every question it is put and answers only when the test
//! says so: the seam the operator's single-pending rule is driven through.
//!
//! One front answers questions one at a time and only when told — a person is
//! present, and no deadline ends the wait. The other has nobody to ask, so it
//! denies at once. The front also counts how many of its asks are live at the
//! same instant: that count is how "one question pending" is observed from the
//! side a person would be on.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use async_trait::async_trait;
use saya_agent::{AgentEventSink, NoopEventSink};
use saya_types::{AgentId, JobId};
use serde_json::Value;
use tokio::sync::{mpsc, oneshot};

use super::operator::SessionOperator;
use super::{ChatEvent, ChatFront, Question};
use crate::agent::tools::consent::{Answer, Caller, Operator};

/// The root's own subject line, in the shell tool's words.
pub(super) fn root_subject() -> String {
    Caller::Root { job_id: job() }.line("Run a shell command", "run a shell command")
}

/// A helper's subject line: it names the helper and the job it works for.
pub(super) fn helper_subject() -> String {
    Caller::Helper {
        agent_id: AgentId::parse("helper-2").expect("the fixture's helper id parses"),
        job_id: job(),
    }
    .line("Run a shell command", "run a shell command")
}

fn job() -> JobId {
    JobId::parse("job-a").expect("the fixture's job id parses")
}

/// Puts one call to the operator from its own task, as a tool of the turn does,
/// and reports which yes came back.
pub(super) fn ask(
    operator: &Arc<SessionOperator>,
    subject: String,
    arguments: Value,
) -> tokio::task::JoinHandle<Answer> {
    let operator: Arc<dyn Operator> = operator.clone();
    tokio::spawn(async move { operator.answer(&subject, &arguments).await })
}

/// One question the front is holding, with the answer the test decides.
pub(super) struct Pending {
    pub(super) question: Question,
    respond: oneshot::Sender<Answer>,
}

impl Pending {
    /// Answers the open question, the way a front's keys do.
    pub(super) fn answer(self, answer: Answer) {
        let _ = self.respond.send(answer);
    }
}

/// A front and the questions it is holding.
pub(super) struct Fixture {
    front: Arc<FakeFront>,
    pending: mpsc::UnboundedReceiver<Pending>,
}

/// The front with a person attached: its asks wait for an answer.
pub(super) fn interactive() -> Fixture {
    Fixture::new(true)
}

/// The front with nobody to ask: its asks deny at once.
pub(super) fn non_interactive() -> Fixture {
    Fixture::new(false)
}

impl Fixture {
    fn new(interactive: bool) -> Self {
        let (asked, pending) = mpsc::unbounded_channel();
        let front = Arc::new(FakeFront {
            interactive,
            state: Mutex::new(State::default()),
            asked,
        });
        Self { front, pending }
    }

    /// The front an operator is built over.
    pub(super) fn front(&self) -> Arc<dyn ChatFront> {
        self.front.clone()
    }

    /// The next question the front is holding: only reached once one is put.
    pub(super) async fn next(&mut self) -> Pending {
        self.pending
            .recv()
            .await
            .expect("the operator put a question to the front")
    }

    /// Lets every started task run, then reports that the front was asked
    /// nothing further — a question waiting its turn is not delivered.
    pub(super) async fn nothing_further(&mut self) {
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert!(
            self.pending.try_recv().is_err(),
            "the front was handed a second question while the first was open"
        );
    }

    /// The questions the front has been put so far.
    pub(super) fn seen(&self) -> usize {
        self.state().seen.len()
    }

    /// The most asks the front has ever held at the same instant.
    pub(super) fn most_at_once(&self) -> usize {
        self.state().most_at_once
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.front
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

#[derive(Default)]
struct State {
    seen: Vec<Question>,
    live: usize,
    most_at_once: usize,
}

struct FakeFront {
    interactive: bool,
    state: Mutex<State>,
    asked: mpsc::UnboundedSender<Pending>,
}

impl FakeFront {
    /// Counts this ask as live until its future ends — including by being
    /// dropped, which is how a cancelled wait leaves the front.
    fn enter<'a>(&'a self, question: &Question) -> Live<'a> {
        {
            let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
            state.seen.push(question.clone());
            state.live += 1;
            state.most_at_once = state.most_at_once.max(state.live);
        }
        Live { state: &self.state }
    }
}

/// One ask in flight: the count it was entered in ends with it.
struct Live<'a> {
    state: &'a Mutex<State>,
}

impl Drop for Live<'_> {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.live = state.live.saturating_sub(1);
    }
}

#[async_trait]
impl ChatFront for FakeFront {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, question: &Question) -> Answer {
        let _live = self.enter(question);
        if !self.interactive {
            return Answer::Deny;
        }
        let (respond, answer) = oneshot::channel();
        let pending = Pending {
            question: question.clone(),
            respond,
        };
        if self.asked.send(pending).is_err() {
            return Answer::Deny;
        }
        answer.await.unwrap_or(Answer::Deny)
    }

    /// The front with a person attached can obtain a per-call approval; the
    /// non-interactive one cannot. It is the fact the run's advertisement reads.
    fn can_obtain_approval(&self) -> bool {
        self.interactive
    }

    fn emit(&self, _event: ChatEvent) {}
}
