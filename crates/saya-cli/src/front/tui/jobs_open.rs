//! The surface's end of a running job: `/start` opens one, the job's verbs
//! steer it, and its worker's queues are read once a frame.
//!
//! Nothing here is a second job runner. The worker is today's
//! (`interactive::tui::coding`), the panel is today's `CodingPanel`, and the
//! consent a start asks for arrives as today's own `CodingMessage::Approval` —
//! which [`super::jobs_desk`] already turns into the card's question. What this
//! module adds is the surface's side of it: which verb opens a job and which
//! one steers the job that is running, and the look that turns what a worker
//! has said into the drawer's rows, the operator's lines, and the card's ask.
//!
//! The look is a thread of its own ([`super::run_input::spawn_jobs`]): a job's
//! queues are `std::sync` ones, and an idle surface paints nothing, so nothing
//! on the frame would ever read them.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use saya_agent::ApprovalPolicy;
use saya_store::SqliteStateStore;

use super::direct::DirectOutcome;
use super::jobs_desk::NO_SUCH_JOB;
use super::live::LiveRuntime;
use super::msg::Worker;
use super::run_cmd_port::SessionPort;
use super::run_front::Forward;
use crate::cli::CodingCommand;
use crate::interactive::tui::{CodingMessage, CodingPanel, Said, started_line};
use crate::slash::JobRequest;

/// What opening a job needs from the launch: the configuration in force, the
/// state store its records and its receipts go to, the folder it roots at, and
/// the session's approval mode — the mode the old surface handed its panel, so a
/// job asks exactly where it did.
pub(crate) struct JobDeps {
    runtime: Arc<LiveRuntime>,
    store: SqliteStateStore,
    root: PathBuf,
    policy: ApprovalPolicy,
}

impl JobDeps {
    /// A job's dependencies. `runtime` is the launch's live configuration — the
    /// same cell a direct command reads, so a job opened after a `/reload`
    /// composes over the configuration then in force. `root` is the session's
    /// own folder — the one the line front's opener states, with the same
    /// fallback when a session bound none.
    pub(crate) fn new(
        runtime: Arc<LiveRuntime>,
        store: SqliteStateStore,
        root: Option<&Path>,
        policy: ApprovalPolicy,
    ) -> Self {
        let root = root
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            runtime,
            store,
            root,
            policy,
        }
    }

    /// Opens `command` through today's worker: one thread, one panel, and the
    /// launch's own runtime, store, folder and mode.
    fn open(&self, command: CodingCommand) -> CodingPanel {
        crate::interactive::tui::spawn_job(
            command,
            self.runtime.current().as_ref().clone(),
            self.store.clone(),
            self.root.clone(),
            self.policy,
        )
    }
}

impl SessionPort {
    /// Performs one job verb on the real port: the deprecated spelling's note is
    /// said once per session (`chat::ops_job`), in the transcript, before the
    /// verb is performed, and a verb the panel could not take is the operator's
    /// business — the front is how the port says anything.
    pub(super) fn perform_job(&self, mut request: JobRequest) {
        {
            let mut notes = self.notes.lock().unwrap_or_else(PoisonError::into_inner);
            crate::chat::claim_job_note(&mut notes, &mut request);
        }
        if let Some(note) = request.note() {
            self.front.note(note.to_owned());
        }
        if let Some(refusal) = self.front.job_command(request.command, self.jobs.as_ref()) {
            self.front.say(refusal);
        }
    }
}

impl Forward {
    /// Performs one job verb on this run's panel. `None` when it was performed;
    /// the surface's own words when it could not be — a mailbox that is full or
    /// closed, or a control with no job of this run to steer.
    ///
    /// With nothing running, today's worker is opened for the verb exactly as
    /// the old surface opened it: a start begins a job, and a reading — `/job
    /// show`, `/job list`, an export — reports through the same one thread and
    /// ends in the transcript. A control steers a job that exists, so with none
    /// it is refused in today's words rather than opening a worker for nothing.
    pub(crate) fn job_command(
        &self,
        command: CodingCommand,
        deps: &JobDeps,
    ) -> Option<&'static str> {
        let mut panel = self.panel();
        if let Some(running) = panel.as_mut().filter(|panel| panel.is_active()) {
            return running.control(command).err();
        }
        if matches!(
            command,
            CodingCommand::Pause { .. }
                | CodingCommand::Continue { .. }
                | CodingCommand::Message { .. }
                | CodingCommand::CancelChild { .. }
                | CodingCommand::Cancel { .. }
        ) {
            return Some(NO_SUCH_JOB);
        }
        let started = started_line(&command);
        *panel = Some(deps.open(command));
        if let Some(line) = started {
            self.note(line);
        }
        None
    }

    /// Looks at the run's panel once: what its worker has said since the last
    /// look, as the drawer's rows, the operator's own lines, or one call on the
    /// card — and the words the job settled with, on the look after it ended.
    /// False once the loop is gone.
    pub(crate) fn pump(&self) -> bool {
        let (job, said, settled) = {
            let mut panel = self.panel();
            match panel.as_mut() {
                Some(panel) => {
                    let job = panel.job_id().map(str::to_owned);
                    let said = panel.drain();
                    let settled = panel.settle();
                    (job, said, settled)
                }
                None => (None, Said::default(), None),
            }
        };
        match job.as_deref() {
            Some(job) => self.report(job, said),
            // A command that runs no root of its own has no job to list: its
            // progress is the surface's own line. It asks nothing — today's
            // worker only gives a job an operator.
            None => {
                for message in said.activity.into_iter().chain(said.receipts) {
                    if let CodingMessage::Activity(text) = message {
                        self.note(text);
                    }
                }
            }
        }
        match settled {
            Some(Ok(words)) => self.note(words),
            Some(Err(words)) => self.deliver(Worker {
                direct: Some(DirectOutcome::Error(words)),
                ..Worker::default()
            }),
            None => {}
        }
        !self.msgs.is_closed()
    }

    /// One look's messages, filed where the operator reads them: a receipt is a
    /// transcript line in the worker's own words, and everything else is the
    /// drawer's row or the card's question.
    fn report(&self, job: &str, said: Said) {
        for message in said.activity {
            self.deliver_job(job, message);
        }
        for message in said.receipts {
            match message {
                CodingMessage::Activity(text) => self.note(text),
                call => self.deliver_job(job, call),
            }
        }
    }
}
