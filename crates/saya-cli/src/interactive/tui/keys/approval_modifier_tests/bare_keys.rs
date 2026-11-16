//! Bare presses still answer exactly as before: `y`/`a` allow once, `n`/`d`/
//! Esc deny, Shift keeps the same answers, and `s` invents no grant when no
//! token was offered.

use super::*;

use saya_agent::ApprovalChoice;
use tokio::sync::oneshot::error::TryRecvError;

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
