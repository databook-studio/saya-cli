//! Keys held with Ctrl, Alt, or Super are editing chords, never consent or
//! denial: the modal can appear mid-typing, and a held modifier must not
//! answer it or escape the check.

use super::*;

use tokio::sync::oneshot::error::TryRecvError;

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
