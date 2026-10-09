use super::*;

use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::types::PendingApproval;
use saya_agent::ApprovalChoice;

#[path = "approval_modifier_tests/bare_keys.rs"]
mod bare_keys;
#[path = "approval_modifier_tests/esc_question.rs"]
mod esc_question;
#[path = "approval_modifier_tests/held_modifier_keys.rs"]
mod held_modifier_keys;
#[path = "approval_modifier_tests/plan_approval.rs"]
mod plan_approval;

/// A pending tool-approval modal with its answer channel, so a test can
/// watch exactly what reaches the agent's decider.
fn pending_approval(grant: Option<&str>) -> (App, tokio::sync::oneshot::Receiver<ApprovalChoice>) {
    let mut app = idle_app();
    let (respond, answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "workspace_write".into(),
        detail: Some("write a file".into()),
        grant: grant.map(str::to_owned),
        scroll: 0,
        respond,
    });
    (app, answer)
}
