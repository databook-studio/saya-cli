use super::*;

use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::run_panel::{PendingPlanApproval, RunPanel};
use crate::interactive::tui::run_worker::RunWorker;
use crate::interactive::tui::types::PendingApproval;
use saya_agent::ApprovalChoice;
use saya_agent::CancellationToken;
use tokio::sync::oneshot::error::TryRecvError;

/// A pending tool-approval modal with its answer channel, so a test can
/// watch exactly what reaches the agent's decider.
fn pending_approval(grant: Option<&str>) -> (App, tokio::sync::oneshot::Receiver<ApprovalChoice>) {
    let mut app = idle_app();
    let (respond, answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "workspace_write".into(),
        detail: Some("write a file".into()),
        grant: grant.map(str::to_owned),
        respond,
    });
    (app, answer)
}

/// A held Ctrl means the press is an editing chord, never consent: the
/// modal can appear mid-typing, and start-of-line Ctrl+A must not approve.
#[test]
fn ctrl_a_does_not_approve() {
    let (mut app, mut answer) = pending_approval(None);
    app.input.set_text("draft");
    handle_key(&mut app, KeyCode::Char('a'), KeyModifiers::CONTROL);
    let decision = answer.try_recv();
    assert!(
        app.request.pending_approval.is_some(),
        "a Ctrl-modified key answered the modal: {decision:?}"
    );
    assert!(
        matches!(decision, Err(TryRecvError::Empty)),
        "a Ctrl-modified key sent a decision: {decision:?}"
    );
    assert_eq!(app.input.text(), "draft", "the draft was disturbed");
}

/// `Ctrl+S` is the persistent-grant hazard: it must never record a session
/// token, the one answer that survives the turn.
#[test]
fn ctrl_s_does_not_grant_a_session() {
    let (mut app, mut answer) = pending_approval(Some("workspace-write"));
    handle_key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
    let decision = answer.try_recv();
    assert!(
        app.request.pending_approval.is_some(),
        "a Ctrl-modified key answered the modal: {decision:?}"
    );
    assert!(
        matches!(decision, Err(TryRecvError::Empty)),
        "a Ctrl-modified key granted a session: {decision:?}"
    );
}

/// The deny keys under Ctrl are editing chords too (Ctrl+D is EOF); a
/// held modifier must not turn them into a denial the user never made.
#[test]
fn ctrl_n_and_ctrl_d_do_not_deny() {
    for (code, name) in [(KeyCode::Char('n'), "n"), (KeyCode::Char('d'), "d")] {
        let (mut app, mut answer) = pending_approval(None);
        handle_key(&mut app, code, KeyModifiers::CONTROL);
        let decision = answer.try_recv();
        assert!(
            app.request.pending_approval.is_some(),
            "Ctrl+{name} denied the pending modal: {decision:?}"
        );
        assert!(
            matches!(decision, Err(TryRecvError::Empty)),
            "Ctrl+{name} sent a deny: {decision:?}"
        );
    }
}

/// Every bare key answers exactly as before, and Shift stays allowed
/// (`Y`/`N` are the same answers their bare forms are). `s` with no token
/// offered still answers nothing at all.
#[test]
fn bare_keys_still_answer() {
    for (code, mods, grant, want) in [
        (
            KeyCode::Char('y'),
            KeyModifiers::NONE,
            None,
            Some(ApprovalChoice::AllowOnce),
        ),
        (
            KeyCode::Char('a'),
            KeyModifiers::NONE,
            None,
            Some(ApprovalChoice::AllowOnce),
        ),
        (
            KeyCode::Char('s'),
            KeyModifiers::NONE,
            Some("workspace-write"),
            Some(ApprovalChoice::AllowSession {
                token: "workspace-write".to_owned(),
            }),
        ),
        (
            KeyCode::Char('n'),
            KeyModifiers::NONE,
            None,
            Some(ApprovalChoice::Deny),
        ),
        (
            KeyCode::Char('d'),
            KeyModifiers::NONE,
            None,
            Some(ApprovalChoice::Deny),
        ),
        (
            KeyCode::Esc,
            KeyModifiers::NONE,
            None,
            Some(ApprovalChoice::Deny),
        ),
        (
            KeyCode::Char('Y'),
            KeyModifiers::SHIFT,
            None,
            Some(ApprovalChoice::AllowOnce),
        ),
    ] {
        let (mut app, mut answer) = pending_approval(grant);
        handle_key(&mut app, code, mods);
        let decision = answer.try_recv();
        assert_eq!(
            decision.ok(),
            want,
            "{code:?} with {mods:?} changed its answer"
        );
        if want.is_some() {
            assert!(
                app.request.pending_approval.is_none(),
                "{code:?} answered but the modal stayed"
            );
        } else {
            assert!(
                app.request.pending_approval.is_some(),
                "{code:?} answered nothing but the modal left"
            );
        }
    }
    // No token offered: `s` invents no grant; the modal stays.
    let (mut app, mut answer) = pending_approval(None);
    handle_key(&mut app, KeyCode::Char('s'), KeyModifiers::NONE);
    assert!(
        matches!(answer.try_recv(), Err(TryRecvError::Empty)),
        "an unoffered `s` invented a grant"
    );
    assert!(app.request.pending_approval.is_some());
}

/// Any held Alt or Super is rejected like Ctrl, and Ctrl+Shift is not a
/// Shift-shaped escape from the check.
#[test]
fn alt_and_super_held_keys_do_not_answer() {
    for (code, mods) in [
        (KeyCode::Char('a'), KeyModifiers::ALT),
        (KeyCode::Char('s'), KeyModifiers::SUPER),
        (
            KeyCode::Char('s'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        ),
    ] {
        let (mut app, mut answer) = pending_approval(Some("workspace-write"));
        handle_key(&mut app, code, mods);
        let decision = answer.try_recv();
        assert!(
            matches!(decision, Err(TryRecvError::Empty)),
            "{code:?} with {mods:?} answered the modal: {decision:?}"
        );
        assert!(app.request.pending_approval.is_some());
    }
}

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
