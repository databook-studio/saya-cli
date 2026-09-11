//! The run panel's drain: the machinery `poll_run_panel` drives each event
//! loop tick. A run's lifecycle events land here and nowhere else — the
//! step list's live status and the panel's lifecycle line both come from the
//! journal's own stream, and the episode's events render into the panel's
//! own transcript through the same seam the session conversation uses.

use super::run_panel::{PendingPlanApproval, RunPanel, RunStep, RunStepStatus};
use super::run_worker::{RunMsg, RunOutcome};
use super::stream_events::apply_event;
use saya_types::RunEvent;
use std::time::Instant;

impl RunPanel {
    /// Drains the worker's messages and applies them: the panel is never
    /// blocked on the run, and one poll applies everything that landed.
    pub(crate) fn drain(&mut self, show_thinking: bool) {
        let messages = {
            let Some(worker) = self.worker.as_mut() else {
                return;
            };
            let mut messages = Vec::new();
            while let Ok(msg) = worker.rx.try_recv() {
                messages.push(msg);
            }
            messages
        };
        self.spinner = self.spinner.wrapping_add(1);
        let mut outcome = None;
        for msg in messages {
            match msg {
                RunMsg::Event(event) => self.apply_event(event),
                RunMsg::Episode(event) => apply_event(&mut self.episode, event, show_thinking),
                RunMsg::PlanApproval {
                    view_text,
                    steps,
                    respond,
                } => {
                    self.seed_steps(&steps);
                    self.plan_approval = Some(PendingPlanApproval { view_text, respond });
                }
                RunMsg::Done(done) => outcome = Some(done),
            }
        }
        if let Some(done) = outcome {
            self.finish(done);
        }
        // The running step's elapsed ticks with the panel; a finished step's
        // figure was frozen at its boundary event.
        for row in &mut self.steps {
            if row.status == RunStepStatus::Running
                && let Some(started) = row.started
            {
                row.elapsed = Some(started.elapsed());
            }
        }
    }

    /// Applies one journaled event: the step list's live status and the
    /// panel's lifecycle line, both shaped where the wire shapes them.
    fn apply_event(&mut self, event: RunEvent) {
        match &event {
            RunEvent::RunStarted => self.started = Some(Instant::now()),
            RunEvent::StepStarted { step } => {
                self.ensure_step(*step);
                let row = &mut self.steps[*step];
                row.status = RunStepStatus::Running;
                row.started = Some(Instant::now());
            }
            RunEvent::StepCompleted { step } => {
                let row = self.ensure_step(*step);
                row.status = RunStepStatus::Completed;
                row.elapsed = row.started.take().map(|started| started.elapsed());
            }
            RunEvent::StepFailed { step } => {
                let row = self.ensure_step(*step);
                row.status = RunStepStatus::Failed;
                row.elapsed = row.started.take().map(|started| started.elapsed());
            }
            RunEvent::Paused { .. } => {
                self.terminated = true;
                self.paused = true;
            }
            RunEvent::Completed | RunEvent::Failed { .. } | RunEvent::Cancelled => {
                self.terminated = true;
            }
            // Usage and deliverables ride the durable record; the lifecycle
            // line is what the panel's status row is for.
            _ => {}
        }
        match &event {
            RunEvent::Usage { .. } | RunEvent::Deliverables { .. } => {}
            other => {
                self.status = crate::render_run::run_event_text(other)
                    .trim_end()
                    .to_string();
                self.status_is_error = matches!(other, RunEvent::Failed { .. });
            }
        }
    }

    /// Grows the step list to cover `step` when the journal mentions a step
    /// the plan never showed the panel (a defensive bound — the plan binds
    /// before any step starts).
    fn ensure_step(&mut self, step: usize) -> &mut RunStep {
        while self.steps.len() <= step {
            let index = self.steps.len();
            self.steps.push(RunStep {
                goal: format!("step {}", index + 1),
                status: RunStepStatus::Pending,
                elapsed: None,
                started: None,
            });
        }
        &mut self.steps[step]
    }

    /// Seeds the step list from the approval ask: the same goals the modal
    /// showed, in plan order, all pending.
    fn seed_steps(&mut self, goals: &[String]) {
        self.steps = goals
            .iter()
            .map(|goal| RunStep {
                goal: goal.clone(),
                status: RunStepStatus::Pending,
                elapsed: None,
                started: None,
            })
            .collect();
    }

    /// Records the drive's end. The lifecycle events have already said how
    /// the run ended; the captured message only adds news when nothing
    /// reached the panel — a refusal before any event, mostly.
    fn finish(&mut self, outcome: RunOutcome) {
        self.worker = None;
        self.cancelling = false;
        if outcome.message.is_empty() || self.terminated {
            return;
        }
        self.status = outcome.message;
        self.status_is_error = outcome.code != 0;
    }
}

#[cfg(test)]
mod tests {
    use super::super::run_panel::{RunPanel, RunStepStatus};
    use super::super::run_worker::{RunMsg, RunOutcome, RunWorker};
    use saya_agent::CancellationToken;
    use saya_types::RunEvent;

    /// A journal event the plan never showed the panel still gets a row —
    /// the defensive bound that keeps a step index inside the list.
    #[test]
    fn a_step_the_plan_did_not_show_still_gets_a_row() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut panel = RunPanel::new(
            RunWorker {
                rx,
                cancel: CancellationToken::new(),
            },
            "r-test".into(),
            "goal".into(),
        );
        panel.apply_event(RunEvent::StepStarted { step: 2 });
        assert_eq!(panel.steps.len(), 3);
        assert_eq!(panel.steps[2].status, RunStepStatus::Running);
    }

    /// A drive end after a terminal lifecycle event adds nothing: the
    /// events already said how the run ended, and a redundant message would
    /// overwrite the shaper's line with the settle wording.
    #[test]
    fn a_done_after_a_terminal_event_changes_nothing() {
        let (_tx, rx) = std::sync::mpsc::channel();
        let mut panel = RunPanel::new(
            RunWorker {
                rx,
                cancel: CancellationToken::new(),
            },
            "r-test".into(),
            "goal".into(),
        );
        panel.apply_event(RunEvent::Completed);
        panel.drain(false);
        assert_eq!(panel.status, "run completed");
        assert!(!panel.status_is_error);
    }

    /// A drive end with no terminal event is news: the refusal becomes the
    /// panel's line, styled as the exit class says.
    #[test]
    fn a_refusal_lands_as_the_panel_line() {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut panel = RunPanel::new(
            RunWorker {
                rx,
                cancel: CancellationToken::new(),
            },
            "r-test".into(),
            "goal".into(),
        );
        tx.send(RunMsg::Done(RunOutcome {
            code: 2,
            message: "a headless run refuses to start without --allow <scopes>".into(),
        }))
        .unwrap();
        panel.drain(false);
        assert!(panel.status_is_error);
        assert!(panel.status.contains("--allow"));
    }
}
