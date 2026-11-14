//! The plan-approval modal shares the approval seam and the same held-key
//! rule: a held Ctrl must not deny the plan, while a bare `n` still does.

use super::*;

use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::run_panel::{PendingPlanApproval, RunPanel};
use crate::interactive::tui::run_worker::RunWorker;
use saya_agent::CancellationToken;
use tokio::sync::oneshot::error::TryRecvError;

/// The plan-approval modal has the same seam: a held Ctrl must not deny
/// the plan, while a bare `n` still does.
#[test]
fn plan_approval_ignores_ctrl_n() {
    let mut app = idle_app();
    let (_tx, rx) = std::sync::mpsc::channel();
    let mut panel = RunPanel::new(
        RunWorker {
            rx,
            cancel: CancellationToken::new(),
        },
        "r-plan".into(),
        "goal".into(),
    );
    let (respond, mut answer) = tokio::sync::oneshot::channel();
    panel.plan_approval = Some(PendingPlanApproval {
        view_text: "1. read the schema".into(),
        respond,
    });
    app.run_panel = Some(panel);
    handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::CONTROL);
    let decision = answer.try_recv();
    assert!(
        app.run_panel
            .as_ref()
            .is_some_and(|panel| panel.plan_approval.is_some()),
        "a Ctrl-modified key answered the plan modal: {decision:?}"
    );
    assert!(
        matches!(decision, Err(TryRecvError::Empty)),
        "a Ctrl-modified key sent a plan decision: {decision:?}"
    );
    handle_key(&mut app, KeyCode::Char('n'), KeyModifiers::NONE);
    assert_eq!(
        answer.try_recv(),
        Ok(false),
        "a bare n still denies the plan"
    );
}
