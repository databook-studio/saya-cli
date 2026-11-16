//! Shift+Tab: the key that queues a permission-mode cycle, the spelling
//! terminals report it as, and the refusal while a prompt owns the keyboard.

use super::*;
use crate::interactive::tui::application::tests_support::idle_app;
use crate::interactive::tui::types::PendingApproval;

/// The words a refusal lands as — the whole transcript, joined, so the test
/// reads what the user reads.
fn said(app: &App) -> String {
    app.transcript
        .blocks()
        .iter()
        .map(|block| block.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

/// A tool-approval modal, wired the way the agent worker hands one back: a
/// live oneshot the modal would answer through.
fn app_with_tool_modal() -> App {
    let mut app = idle_app();
    let (respond, _answer) = tokio::sync::oneshot::channel();
    app.request.pending_approval = Some(PendingApproval {
        tool: "write".into(),
        detail: None,
        grant: Some("workspace-write".into()),
        scroll: 0,
        respond,
    });
    app
}

/// Shift+Tab queues the cycle; it does not switch here, because the switch
/// needs the session's one policy, which this layer does not hold.
#[test]
fn backtab_queues_a_permission_cycle() {
    let mut app = idle_app();
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert!(app.pending_permission_cycle, "BackTab is the cycle's key");
}

/// Some terminals report the press as `Tab` with the shift modifier rather
/// than as `BackTab`; both spellings are the one press.
#[test]
fn tab_with_shift_is_the_same_press() {
    let mut app = idle_app();
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::SHIFT);
    assert!(
        app.pending_permission_cycle,
        "Tab+SHIFT must queue the same cycle as BackTab"
    );
}

/// A bare Tab stays what it was: the popup's accept key, never a switch.
#[test]
fn a_bare_tab_is_not_a_cycle() {
    let mut app = idle_app();
    handle_key(&mut app, KeyCode::Tab, KeyModifiers::NONE);
    assert!(
        !app.pending_permission_cycle,
        "Tab alone must not move the session's mode"
    );
}

/// A prompt that owns the keyboard refuses the press — and says so. Nothing
/// moves: the switch would set the posture under a decision the user is still
/// giving, and the modal is the answer they owe next.
#[test]
fn a_tool_modal_refuses_the_cycle_with_a_notice() {
    let mut app = app_with_tool_modal();
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert!(
        !app.pending_permission_cycle,
        "no switch behind an open decision"
    );
    let said = said(&app);
    assert!(
        said.contains("Permission mode unchanged") && said.contains("Shift+Tab"),
        "the refusal says what happened and what to do: {said}"
    );
    assert!(
        app.request.pending_approval.is_some(),
        "the modal is untouched"
    );
}

/// The startup trust question owns the keyboard the same way.
#[test]
fn the_trust_modal_refuses_the_cycle_with_a_notice() {
    let mut app = idle_app();
    app.overlays.trust = Some(crate::interactive::tui::types::TrustPrompt::default());
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert!(!app.pending_permission_cycle);
    assert!(said(&app).contains("Permission mode unchanged"));
}

/// The completion popup is an input affordance, not a decision: it does not
/// own the keyboard, and Shift+Tab beside it is still the user's switch.
#[test]
fn the_completion_popup_does_not_own_the_keyboard() {
    let mut app = idle_app();
    app.input.set_text("/mo");
    app.refresh_menu();
    assert!(app.overlays.menu.is_some(), "the popup is open");
    handle_key(&mut app, KeyCode::BackTab, KeyModifiers::NONE);
    assert!(
        app.pending_permission_cycle,
        "the popup must not eat the cycle key"
    );
    assert!(app.overlays.menu.is_some(), "the popup stays open");
}
