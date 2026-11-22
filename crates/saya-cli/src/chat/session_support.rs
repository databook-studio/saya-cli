//! A run port and a front the session's tests drive: the port answers from a
//! script and holds a turn until the test releases it, so a turn can be running
//! while the test sends another message; the front records what it was told as
//! plain summaries, because these tests read the session's events, not a
//! surface's rendering of them.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{AgentEventSink, NoopEventSink};
use tokio::sync::Notify;

use super::{Admission, ChatSession};
use crate::agent::tools::consent::Answer;
use crate::chat::port::{PortError, Prepared, RunPort};
use crate::chat::{ChatEvent, ChatFront, ChatOutcome, Question};

/// How long one admission is given before the test calls it a wait on the turn.
pub(super) const ADMIT: Duration = Duration::from_millis(250);

/// A plain run: one connected profile, no workspace.
pub(super) fn prepared() -> Result<Prepared, String> {
    Ok(Prepared {
        db: Some("analytics".to_owned()),
        workspace: None,
    })
}

/// The events one front was told, as plain summaries.
#[derive(Default)]
pub(super) struct RecordingFront {
    events: Mutex<Vec<String>>,
}

impl RecordingFront {
    pub(super) fn events(&self) -> Vec<String> {
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(super) fn notices(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|event| event.starts_with("notice: "))
            .collect()
    }

    pub(super) fn turn_ends(&self) -> Vec<String> {
        self.events()
            .into_iter()
            .filter(|event| event.starts_with("turn-ended "))
            .collect()
    }
}

#[async_trait]
impl ChatFront for RecordingFront {
    fn sink(&self) -> Arc<dyn AgentEventSink> {
        Arc::new(NoopEventSink)
    }

    async fn ask(&self, _question: &Question) -> Answer {
        Answer::Deny
    }

    fn emit(&self, event: ChatEvent) {
        let summary = match event {
            ChatEvent::Ready { db, workspace } => {
                format!("ready db={db:?} workspace={workspace:?}")
            }
            ChatEvent::Notice { message } => format!("notice: {message}"),
            ChatEvent::TurnEnded(outcome) => format!("turn-ended {outcome:?}"),
            ChatEvent::Question(_) => "question".to_owned(),
            ChatEvent::QueryCaptured(_) => "query-captured".to_owned(),
            ChatEvent::Agent(_) => "agent".to_owned(),
        };
        self.events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(summary);
    }
}

/// What the fake port has been told to answer, and what it saw.
#[derive(Default)]
struct Script {
    prepares: VecDeque<Result<Prepared, String>>,
    busy: usize,
    hold: bool,
    /// The next turn ends on a progress guard — the pause `/continue`
    /// continues — and the one after it answers normally.
    pause: bool,
    cancelled: bool,
    closed: bool,
    calls: usize,
    prepares_run: usize,
    prompts: Vec<String>,
}

/// A run port the test drives.
#[derive(Default)]
pub(super) struct FakePort {
    script: Mutex<Script>,
    started: Notify,
    release: Notify,
}

impl FakePort {
    /// A port whose prepares answer from `prepares`, in order.
    pub(super) fn new(prepares: impl IntoIterator<Item = Result<Prepared, String>>) -> Arc<Self> {
        let port = Arc::new(Self::default());
        port.script().prepares.extend(prepares);
        port
    }

    /// Holds every turn until the test releases it.
    pub(super) fn hold_turns(&self) {
        self.script().hold = true;
    }

    /// Answers `times` turn calls with the engine's own `Busy`.
    pub(super) fn busy(&self, times: usize) {
        self.script().busy = times;
    }

    /// The next turn ends on a progress guard, as one that paused does; the
    /// turn after it answers normally.
    pub(super) fn pause_turns(&self) {
        self.script().pause = true;
    }

    /// Waits until a turn has been asked for.
    pub(super) async fn wait_started(&self) {
        self.started.notified().await;
    }

    /// Lets the running turn end.
    pub(super) fn release(&self) {
        self.release.notify_one();
    }

    pub(super) fn prompts(&self) -> Vec<String> {
        self.script().prompts.clone()
    }

    pub(super) fn calls(&self) -> usize {
        self.script().calls
    }

    pub(super) fn prepares_run(&self) -> usize {
        self.script().prepares_run
    }

    pub(super) fn closed(&self) -> bool {
        self.script().closed
    }

    fn script(&self) -> MutexGuard<'_, Script> {
        self.script.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[async_trait]
impl RunPort for FakePort {
    async fn prepare(&self) -> Result<Prepared, String> {
        let mut script = self.script();
        script.prepares_run += 1;
        script.prepares.pop_front().unwrap_or_else(prepared)
    }

    async fn next_turn(
        &self,
        _sink: Arc<dyn AgentEventSink>,
        prompt: String,
    ) -> Result<ChatOutcome, PortError> {
        self.started.notify_one();
        let hold = {
            let mut script = self.script();
            script.calls += 1;
            if script.busy > 0 {
                script.busy -= 1;
                return Err(PortError::Busy);
            }
            script.prompts.push(prompt);
            script.hold
        };
        if hold {
            self.release.notified().await;
        }
        let cancelled = self.script().cancelled;
        if cancelled {
            return Ok(ChatOutcome::Cancelled);
        }
        if std::mem::take(&mut self.script().pause) {
            return Ok(ChatOutcome::Paused {
                message: "stopped: the turn made no progress (raise the guard to continue)"
                    .to_owned(),
            });
        }
        Ok(ChatOutcome::Answered)
    }

    async fn cancel_turn(&self) {
        self.script().cancelled = true;
        self.release.notify_one();
    }

    async fn close(&self) {
        self.script().closed = true;
    }
}

/// A session over a fake port and a recording front.
pub(super) fn session(port: &Arc<FakePort>) -> (Arc<ChatSession>, Arc<RecordingFront>) {
    let front = Arc::new(RecordingFront::default());
    let port: Arc<dyn RunPort> = port.clone();
    let sink: Arc<dyn ChatFront> = front.clone();
    (Arc::new(ChatSession::new(port, sink)), front)
}

/// Sends one message on its own task, so a turn can be running while the test
/// sends another.
pub(super) fn send_in_background(
    session: &Arc<ChatSession>,
    message: &str,
) -> tokio::task::JoinHandle<Admission> {
    let session = Arc::clone(session);
    let message = message.to_owned();
    tokio::spawn(async move { session.send(message).await })
}
