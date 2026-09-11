//! The plan-approval gate at `planned → approved` (DESIGN §5.2, §7): the
//! user accepts the plan, its scopes, and the budgets once, before anything
//! runs. Headless, the approval is the `RunSpec` pre-authorization — the
//! engine has already refused any plan outside `--allow`, so the
//! pre-authorization is the decision, made once. Interactive, the request
//! travels over a channel with a oneshot reply (the `tui/agent.rs`
//! pattern): the UI shows the view as a modal and answers, never stdin.

use super::approval_view::{PlanApprovalView, render};
use tokio::sync::oneshot::{Receiver, Sender};

/// The plan-approval decision surface.
pub(super) enum PlanApproval {
    /// Headless: the `RunSpec` pre-authorized the scopes, the engine refused
    /// any plan outside them, and the run cannot prompt — the
    /// pre-authorization is the decision, made once.
    PreAuthorized,
    /// The TUI's channel: the rendered view is sent with a oneshot reply;
    /// the UI's modal answers. A closed channel or a dropped reply is a
    /// refusal — deny by default.
    ViaChannel(tokio::sync::mpsc::UnboundedSender<PlanApprovalRequest>),
}

/// One approval ask: what the modal shows and how it answers.
pub(super) struct PlanApprovalRequest {
    pub(super) view_text: String,
    pub(super) respond: Sender<bool>,
}

/// Decides the plan approval once, at `planned → approved`. The surface
/// outlives any single ask: a mid-run capability re-ask (DESIGN §5.3) rides
/// the same channel rather than inheriting an earlier approval.
pub(super) async fn decide(surface: &PlanApproval, view: &PlanApprovalView) -> bool {
    match surface {
        PlanApproval::PreAuthorized => true,
        PlanApproval::ViaChannel(sender) => {
            let (respond, answer): (_, Receiver<bool>) = tokio::sync::oneshot::channel();
            if sender
                .send(PlanApprovalRequest {
                    view_text: render(view),
                    respond,
                })
                .is_err()
            {
                return false;
            }
            answer.await.unwrap_or(false)
        }
    }
}
