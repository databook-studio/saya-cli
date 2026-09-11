//! The run panel's application seams: what the event loop calls. `/run`'s
//! parsed tail starts the worker and opens the panel; every loop tick polls
//! it (never blocking); Esc stops the run through its token; the modal
//! answers the plan-approval ask; and a quit is refused while a run is still
//! in flight — a worker this session owns is never orphaned by an exit.

use super::super::run_panel::RunPanel;
use super::super::run_worker::RunJob;
use super::super::types::App;
use crate::interactive::session_state::SessionState;
use crate::render::RenderFormat;
use saya_agent::ApprovalPolicy;

impl App {
    /// Starts a run driven from the panel: the goal, scopes, and budgets the
    /// session's `/run` tail parsed into, on the same path `saya run` takes.
    /// The run id is minted with the same helper, so the panel's title names
    /// the run the journal will record.
    pub(crate) fn start_run_panel(
        &mut self,
        goal: Option<String>,
        allow: Vec<String>,
        budget: Vec<String>,
        format: RenderFormat,
        state: &SessionState,
    ) {
        let run_id = crate::commands::new_run_id();
        let goal_display = goal.clone().unwrap_or_default();
        let approval = state
            .approval_mode
            .parse()
            .unwrap_or(ApprovalPolicy::ReadOnly);
        let worker = super::super::run_worker::spawn(RunJob {
            runtime: std::sync::Arc::clone(&self.runtime),
            state_db: self.state_db.clone(),
            format,
            approval,
            profile: state.profile.clone(),
            request: crate::commands::RunRequest {
                run_id: run_id.clone(),
                goal,
                allow,
                budget,
            },
        });
        self.run_panel = Some(RunPanel::new(
            worker,
            run_id.as_str().to_string(),
            goal_display,
        ));
    }

    /// Polls the run worker (non-blocking): the panel's step list, its
    /// lifecycle line, and its episode transcript advance when the run has
    /// news; the event loop never blocks on the run.
    pub(crate) fn poll_run_panel(&mut self, show_thinking: bool) {
        if let Some(panel) = self.run_panel.as_mut() {
            panel.drain(show_thinking);
        }
    }

    /// Stops the panel's run the way Esc stops an agent stream: the token
    /// cancels and the worker records the stop through the engine path. A
    /// pending plan approval is refused — a run the user is stopping is
    /// never approved mid-stop.
    pub(crate) fn cancel_run_panel(&mut self) {
        let Some(panel) = self.run_panel.as_mut() else {
            return;
        };
        if let Some(worker) = panel.worker.as_ref() {
            worker.cancel.cancel();
            panel.cancelling = true;
        }
        if let Some(request) = panel.plan_approval.take() {
            let _ = request.respond.send(false);
        }
    }

    /// Answers the plan-approval modal. Only an explicit yes approves; a
    /// dropped reply is a refusal, the same rule the drive applies.
    pub(crate) fn answer_plan_approval(&mut self, allow: bool) {
        if let Some(panel) = self.run_panel.as_mut()
            && let Some(request) = panel.plan_approval.take()
        {
            let _ = request.respond.send(allow);
        }
    }

    /// Closes the panel. Only meaningful once the run is over — an in-flight
    /// run is cancelled with Esc, never orphaned by hiding its panel.
    pub(crate) fn close_run_panel(&mut self) {
        self.run_panel = None;
    }

    /// Attempts to quit. An in-flight run is a worker this session owns:
    /// quitting now would orphan it — a journal left mid-flight that only a
    /// resume could reconcile — so quitting is refused until the run is
    /// cancelled (Esc) or finishes. Returns whether the quit may proceed.
    pub(crate) fn try_quit(&mut self) -> bool {
        if self
            .run_panel
            .as_ref()
            .is_some_and(|panel| panel.is_active())
        {
            self.transcript.push(
                super::super::transcript::BlockKind::System,
                "A run is still executing — Esc cancels it first, or wait for it to finish.",
            );
            return false;
        }
        true
    }
}
