//! The surface's end of today's job plumbing (`interactive::tui::coding`): the
//! rows the drawer lists, the calls a job puts to the operator, and the panel a
//! job's control goes through.
//!
//! Nothing here is a second job runner. A job's worker reports with today's
//! [`CodingMessage`] — its activity lines and the calls it asks — and today's
//! `CodingPanel::cancel` is how `x` reaches it. What this module adds is the
//! surface's side of that plumbing: which job a line belongs to, the question a
//! call becomes on the card, and the waiting end the card's answer resolves.

use std::collections::HashMap;

use saya_agent::ApprovalChoice;
use tokio::sync::oneshot;

use super::jobs::JobReport;
use super::msg::Worker;
use super::run_front::Forward;
use super::view::jobs::{Job, State, WAITING_FOR_OPERATOR};
use crate::agent::tools::consent::Answer;
use crate::chat::Question;
use crate::interactive::tui::{CodingMessage, CodingPanel};

/// The longest summary a row keeps, in characters: an activity line is a
/// worker's own message, and a drawer row is one line.
const MAX_SUMMARY: usize = 120;

/// The longest facts one consent states, in characters: the proposal is
/// bounded where it is built, and the operator answers for every row of it.
const MAX_FACTS: usize = 16 * 1024;

/// What the surface says when an `x` reaches no job of this run.
pub(crate) const NO_SUCH_JOB: &str = "no job of that id is running in this surface";

/// The waiting end of one call, as the plumbing asked it: the panel's own
/// yes/no about a job's scope, or an egressing call's choice.
pub(super) enum Waiting {
    /// The job's scope question: a yes is a yes.
    Scope(oneshot::Sender<bool>),
    /// One egressing call the job asks about.
    Call(oneshot::Sender<ApprovalChoice>),
}

/// The run's jobs as today's job plumbing states them: one row per job, and the
/// waiting end of every call one of them has put to the operator.
#[derive(Default)]
pub(crate) struct JobDesk {
    rows: Vec<Job>,
    /// The calls waiting for an answer, by the question's own id.
    waiting: HashMap<u64, Waiting>,
    /// The surface's own number for the next call, so two jobs' calls cannot
    /// land on one card id.
    next_id: u64,
}

impl JobDesk {
    /// The rows the drawer lists.
    pub(super) fn rows(&self) -> &[Job] {
        &self.rows
    }

    /// Reads one message from a job's worker as the drawer's own: its rows as
    /// they now are, or one call to put on the card. The waiting end stays here,
    /// where the card's answer finds it.
    pub(super) fn read(&mut self, job: &str, message: CodingMessage) -> Option<JobReport> {
        match message {
            CodingMessage::Activity(line) => {
                self.note(job, State::Running, summary(&line));
                Some(JobReport::Listed(self.rows.clone()))
            }
            CodingMessage::Approval { text, respond } => {
                // A job's scope question is not a one-line call: its facts are
                // the consent's own rows — the job, the paths, one row per
                // database identity — so the card states them as they were
                // written, never folded into a summary.
                let question = self.question(job, facts(&text));
                let id = question.id;
                self.waiting.insert(id, Waiting::Scope(respond));
                self.owed(job, id);
                Some(JobReport::Ask {
                    job: job.to_owned(),
                    question,
                })
            }
            CodingMessage::Question {
                subject,
                target,
                respond,
            } => {
                let question = self.question(&format!("{job}: {subject}"), summary(&target));
                let id = question.id;
                self.waiting.insert(id, Waiting::Call(respond));
                self.owed(job, id);
                Some(JobReport::Ask {
                    job: job.to_owned(),
                    question,
                })
            }
        }
    }

    /// Answers the call asked under `id`, when it is one of this desk's: the
    /// job's own waiting end is resumed, and nothing is answered twice.
    pub(super) fn answer(&mut self, id: u64, answer: Answer) -> bool {
        match self.waiting.remove(&id) {
            Some(Waiting::Scope(respond)) => respond.send(answer != Answer::Deny).is_ok(),
            Some(Waiting::Call(respond)) => {
                let choice = match answer {
                    Answer::Deny => ApprovalChoice::Deny,
                    Answer::Once => ApprovalChoice::AllowOnce,
                    // The job's own operator records the token its call's
                    // arguments name (`worker::chat_operator`), so the surface
                    // only has to report *that* a session-wide yes was given.
                    Answer::Session => ApprovalChoice::AllowSession {
                        token: String::new(),
                    },
                };
                respond.send(choice).is_ok()
            }
            None => false,
        }
    }

    /// The question one call becomes on the card: the caller's line, the call's
    /// own facts, and no grant of the surface's — the job's operator records the
    /// token its own call names.
    fn question(&mut self, caller: &str, facts: String) -> Question {
        self.next_id += 1;
        Question {
            id: self.next_id,
            caller: caller.to_owned(),
            facts,
            grant: None,
        }
    }

    /// The job's row, with the call it now waits on.
    fn owed(&mut self, job: &str, call: u64) {
        let row = self.row(job);
        row.call = Some(call);
        row.summary = String::from(WAITING_FOR_OPERATOR);
    }

    /// The job's row, in the state its last word states.
    fn note(&mut self, job: &str, state: State, summary: String) {
        let row = self.row(job);
        row.state = state;
        row.summary = summary;
    }

    /// The row of `job`, added when the run has not listed it yet: a job that
    /// says something is a job (`02` §8's `● running` until it says otherwise).
    fn row(&mut self, job: &str) -> &mut Job {
        let at = match self.rows.iter().position(|row| row.id == job) {
            Some(at) => at,
            None => {
                self.rows.push(Job {
                    id: job.to_owned(),
                    ..Job::default()
                });
                self.rows.len() - 1
            }
        };
        &mut self.rows[at]
    }
}

/// One activity line as a row keeps it: redacted, one line, and bounded.
pub(super) fn summary(line: &str) -> String {
    saya_types::redact(line)
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .take(MAX_SUMMARY)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// One consent text as the card's facts state it: redacted, its own rows kept —
/// a consent is a list an operator reads row by row, not one line — and
/// bounded.
pub(super) fn facts(text: &str) -> String {
    saya_types::redact(text)
        .chars()
        .filter(|character| *character == '\n' || !character.is_control())
        .take(MAX_FACTS)
        .collect::<String>()
        .trim_end()
        .to_owned()
}

impl Forward {
    /// Puts one call a job asks to the operator: the question the card shows,
    /// and today's plumbing's own waiting end, resolved by the same answer.
    #[allow(dead_code)]
    pub(crate) fn ask_job(
        &self,
        job: String,
        question: Question,
        respond: oneshot::Sender<ApprovalChoice>,
    ) {
        let id = question.id;
        self.desk_lock().waiting.insert(id, Waiting::Call(respond));
        self.deliver(Worker {
            job: Some(JobReport::Ask { job, question }),
            ..Worker::default()
        });
    }

    /// Takes one message from a job's worker and hands what it comes to to the
    /// loop: the rows, or the call the card must ask.
    #[allow(dead_code)]
    pub(crate) fn deliver_job(&self, job: &str, message: CodingMessage) {
        let report = self.desk_lock().read(job, message);
        if let Some(report) = report {
            self.deliver(Worker {
                job: Some(report),
                ..Worker::default()
            });
        }
    }

    /// Cancels the job this id names through today's panel (`02` §8's `x`), and
    /// says what the panel said when it could not.
    pub(crate) fn cancel_job(&self, id: &str) {
        let refused = match self.panel().as_mut() {
            Some(panel) => panel.cancel(id).err(),
            None => Some(NO_SUCH_JOB),
        };
        if let Some(reason) = refused {
            self.say(reason);
        }
    }

    /// Takes the panel a job of this run reports through. The old TUI's `App`
    /// owns one per running command; this surface's launch (T19) hands it here,
    /// so one panel serves whichever surface is live.
    #[allow(dead_code)]
    pub(crate) fn attach_job_panel(&self, panel: CodingPanel) {
        *self.panel() = Some(panel);
    }
}
