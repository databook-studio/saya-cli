//! The background jobs drawer's half of the model: the job list the surface
//! holds, the calls a job puts to the operator, and the one card slot they
//! share with the session's (`02` §8, §5).
//!
//! Two rules, both T9's. A call is *asked* through the same card as the
//! session's, whoever asks it — the root, a helper, or a job — so there is one
//! place an approval is read and one place it is answered. And one card is
//! open at a time: an ask that arrives while another one is open waits its turn
//! in the order it arrived, and is put on the card the moment the open one is
//! answered, exactly as the chat layer's operator serialises its own asks. A
//! job's ask therefore queues behind an open card and never replaces it.
//!
//! The rows themselves are [`super::view::jobs`]'s: this module only keeps them
//! current and decides which call goes on the card.

use super::keys::Intent;
use super::model::Model;
use super::model_overlay::Overlay;
use super::msg::Cmd;
use super::view::jobs::{CARD_FIRST, Drawer, Job, State, WAITING_FOR_OPERATOR};
use crate::agent::tools::consent::Answer;
use crate::chat::Question;
use crate::interactive::tui::NO_PENDING;

/// What a job's plumbing reports to the surface, on the queue's worker slot.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum JobReport {
    /// The run's jobs as the plumbing now states them: the drawer's rows.
    Listed(Vec<Job>),
    /// One call a job puts to the operator. It enters the same card slot as the
    /// session's and queues behind the card that is open (T9/A5).
    Ask {
        /// The job asking, by the id the drawer lists it under.
        job: String,
        /// The call itself, as the chat layer's operator states it.
        question: Question,
    },
}

impl Model {
    /// The drawer's jobs, whether or not it is open: a job's word is folded
    /// while the drawer is closed too, so opening it shows the run as it is.
    pub(crate) fn jobs(&self) -> &Drawer {
        &self.jobs
    }

    /// Whether the drawer is the pane on screen.
    pub(crate) fn jobs_open(&self) -> bool {
        matches!(self.overlay, Some(Overlay::Jobs))
    }

    /// The drawer, while it is the pane that is open, to be walked.
    pub(crate) fn jobs_mut(&mut self) -> Option<&mut Drawer> {
        self.jobs_open().then_some(&mut self.jobs)
    }

    /// Opens the drawer on the jobs the run has (`02` §8's `Ctrl+J`, `/job`).
    pub(crate) fn open_jobs(&mut self) {
        self.overlay = Some(Overlay::Jobs);
    }

    /// Folds one word from the job's plumbing: the list as it now is, or one
    /// call the job puts to the operator.
    pub(crate) fn note_jobs(&mut self, report: JobReport) {
        match report {
            JobReport::Listed(rows) => {
                self.jobs.rows = rows;
                self.jobs.selected = self.jobs.selected();
            }
            JobReport::Ask { job, question } => {
                self.owe(&job, question.id);
                self.ask(question);
            }
        }
    }

    /// Puts one call to the operator: on the card when none is open, and behind
    /// the open one otherwise — in the order the asks arrived.
    pub(crate) fn ask(&mut self, question: Question) {
        if self.card.is_some() {
            self.held.push(question);
            return;
        }
        self.show_card(question);
    }

    /// One card answered: the job that owed that call no longer waits on it, and
    /// the next ask takes the slot it left.
    pub(crate) fn answered(&mut self, id: u64) {
        for row in &mut self.jobs.rows {
            if row.call == Some(id) {
                row.call = None;
            }
        }
        if self.card.is_none() && !self.held.is_empty() {
            let next = self.held.remove(0);
            self.show_card(next);
        }
    }

    /// States that `job` owes the call asked under `call`, listing the job when
    /// the run's list has not caught up with it yet: a job that asks is a job.
    fn owe(&mut self, job: &str, call: u64) {
        match self.jobs.rows.iter_mut().find(|row| row.id == job) {
            Some(row) => {
                row.call = Some(call);
                row.summary = String::from(WAITING_FOR_OPERATOR);
            }
            None => self.jobs.rows.push(Job {
                id: job.to_owned(),
                state: State::Running,
                summary: String::from(WAITING_FOR_OPERATOR),
                call: Some(call),
                ..Job::default()
            }),
        }
    }
}

/// One job line, read on the surface's side (`02` §8). `/approve` and `/deny`
/// are the consent card's own answers spelled out, so they are answered through
/// that card — never around it — and every other verb is the job's own command.
pub(crate) fn line(model: &mut Model, request: crate::slash::JobRequest, cmds: &mut Vec<Cmd>) {
    match &request.command {
        crate::cli::CodingCommand::Approve { id } => consent(model, id, Answer::Once, cmds),
        crate::cli::CodingCommand::Deny { id } => consent(model, id, Answer::Deny, cmds),
        _ => cmds.push(Cmd::Job(request)),
    }
}

/// The consent one job asked for, answered by name: the same `Cmd::Answer` any
/// card gives, so the job's own waiting end is resumed and the slot is handed
/// on. A consent queued behind another card is never jumped — an approval is
/// the operator's, and the card they are looking at comes first.
fn consent(model: &mut Model, id: &str, answer: Answer, cmds: &mut Vec<Cmd>) {
    let call = model
        .jobs()
        .rows
        .iter()
        .find(|row| row.id == id)
        .and_then(|row| row.call);
    let Some(call) = call else {
        model.say(NO_PENDING);
        return;
    };
    if model.card().map(|card| card.id) != Some(call) {
        model.say(CARD_FIRST);
        return;
    }
    model.take_card();
    model.answered(call);
    cmds.push(Cmd::Answer(call, answer));
}

/// One press on the drawer, carried out (`02` §8). Only `x`'s answered confirm
/// yields a command — the drawer cancels a job and opens a card, and never
/// answers one itself.
pub(crate) fn press(model: &mut Model, intent: Intent, cmds: &mut Vec<Cmd>) {
    match intent {
        Intent::JobsUp => {
            if let Some(drawer) = model.jobs_mut() {
                drawer.up();
            }
        }
        Intent::JobsDown => {
            if let Some(drawer) = model.jobs_mut() {
                drawer.down();
            }
        }
        Intent::JobsOpen => detail(model),
        Intent::JobsApprove => approve(model),
        Intent::JobsCancelAsk => {
            if let Some(drawer) = model.jobs_mut() {
                drawer.ask_cancel();
            }
        }
        Intent::JobsCancelYes => cancel(model, cmds),
        Intent::JobsCancelNo => {
            if let Some(drawer) = model.jobs_mut() {
                drawer.answer(false);
            }
        }
        Intent::JobsClose => model.close_overlay(),
        _ => {}
    }
}

/// `↵`: the selected job's detail in the transcript, where the operator reads.
/// Nothing is asked of the run: a job's detail is a reading, not a command.
fn detail(model: &mut Model) {
    let Some(lines) = model.jobs().detail() else {
        return;
    };
    for message in lines {
        model
            .transcript
            .current()
            .push(crate::front::Entry::Notice(crate::front::Notice {
                message,
            }));
    }
    model.close_overlay();
}

/// `a`: the card for the selected job's call, and nothing else. It never consents
/// blind — with a card already open the ask waits its turn (T9/A5), and the
/// drawer says which one the operator is answering.
fn approve(model: &mut Model) {
    let Some(call) = model.jobs().selected_row().and_then(|row| row.call) else {
        return;
    };
    if model.card().map(|card| card.id) == Some(call) {
        model.close_overlay();
        return;
    }
    model.say(CARD_FIRST);
}

/// `x`, answered `y`: the one command the drawer asks for.
fn cancel(model: &mut Model, cmds: &mut Vec<Cmd>) {
    let Some(id) = model.jobs_mut().and_then(|drawer| drawer.answer(true)) else {
        return;
    };
    cmds.push(Cmd::CancelJob(id));
}
