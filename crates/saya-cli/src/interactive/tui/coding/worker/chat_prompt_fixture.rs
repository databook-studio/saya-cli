//! One TUI frame over a job's panel, for the chat-prompt tests: the worker's
//! queues and the app's chat surface, so `poll_coding` is exactly one frame's
//! drain and the approval prompt is the app's own.

use super::super::{CodingMessage, CodingPanel};
use super::chat_operator::ChatOperator;
use crate::agent::tools::consent::{Caller, Operator};
use crate::interactive::tui::types::App;
use crate::interactive::tui::types::app::idle_app;
use saya_agent::{ApprovalChoice, CancellationToken};
use saya_types::{AgentId, JobId};
use serde_json::Value;
use std::sync::{Arc, OnceLock, mpsc};

fn job() -> JobId {
    JobId::parse("job-a").expect("the fixture's job id parses")
}

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

pub(super) struct Frame {
    app: App,
    sender: mpsc::SyncSender<CodingMessage>,
    token: CancellationToken,
    /// Kept alive so the panel never reports its worker gone mid-test.
    _done: mpsc::SyncSender<Result<String, String>>,
    _controls: tokio::sync::mpsc::Receiver<crate::cli::CodingCommand>,
}

pub(super) fn frame() -> Frame {
    let mut app = idle_app();
    let (sender, activity) = mpsc::sync_channel(8);
    let (_receipt_tx, receipts) = mpsc::sync_channel(8);
    let (done_tx, done) = mpsc::sync_channel(1);
    let (commands, controls) = tokio::sync::mpsc::channel(4);
    let token = CancellationToken::new();
    app.coding_panel = Some(CodingPanel {
        id: Some("job-a".into()),
        rx: activity,
        receipts,
        done,
        commands,
        cancellation: token.clone(),
        pending: None,
        worker: None,
        active: true,
        // This fixture's panel runs no job, so nothing is ever stated on it.
        job_mode: Arc::new(OnceLock::new()),
    });
    Frame {
        app,
        sender,
        token,
        _done: done_tx,
        _controls: controls,
    }
}

impl Frame {
    /// The job asking one question, from its own thread: the operator the
    /// panel attaches, asking with the subject the tool built.
    pub(super) fn ask(&self, subject: String, arguments: Value) -> tokio::task::JoinHandle<bool> {
        let operator = Arc::new(self.operator());
        tokio::spawn(async move { operator.ask(&subject, &arguments).await })
    }

    /// The operator the panel attaches for this job.
    pub(super) fn operator(&self) -> ChatOperator {
        ChatOperator::new(self.sender.clone(), self.token.clone())
    }

    /// Frames until the chat prompt holds the job's question.
    pub(super) async fn until_asked(&mut self) {
        for _ in 0..1_000 {
            self.app.poll_coding();
            if self.prompt_is_open() {
                return;
            }
            tokio::task::yield_now().await;
        }
        panic!("the job's question never reached the chat's approval prompt");
    }

    /// Frames over which no question may appear.
    pub(super) async fn frames_without_a_question(&mut self) {
        for _ in 0..64 {
            tokio::task::yield_now().await;
            self.app.poll_coding();
        }
    }

    /// Whether the chat's approval prompt is holding a question.
    pub(super) fn prompt_is_open(&self) -> bool {
        self.app.request.pending_approval.is_some()
    }

    /// The question the chat prompt holds: who asks, the target shown, and
    /// the grammar token a remembered answer would name.
    pub(super) fn question(&self) -> (String, String, Option<String>) {
        let pending = self
            .app
            .request
            .pending_approval
            .as_ref()
            .expect("the chat's approval prompt holds the job's question");
        (
            pending.tool.clone(),
            pending
                .detail
                .clone()
                .expect("the prompt shows the call's target"),
            pending.grant.clone(),
        )
    }

    /// Answers the open question, the way the modal's keys do.
    pub(super) fn answer(&mut self, choice: ApprovalChoice) {
        self.app.answer_approval(choice);
    }

    /// Cancels the job, as `/job cancel` and the panel's own drop do.
    pub(super) fn cancel(&self) {
        self.token.cancel();
    }

    /// The run's token, for the engine's own per-job serialisation gate.
    pub(super) fn token_ref(&self) -> CancellationToken {
        self.token.clone()
    }
}
