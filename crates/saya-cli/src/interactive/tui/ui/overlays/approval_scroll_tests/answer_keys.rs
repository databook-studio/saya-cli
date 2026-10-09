use super::*;

/// Slice A's contract, re-asserted because this slice touches that path:
/// every bare answer key still answers exactly as today, and Enter still
/// answers nothing.
#[test]
fn bare_answer_keys_still_answer() {
    for (code, grant, want) in [
        (KeyCode::Char('y'), None, Some(ApprovalChoice::AllowOnce)),
        (KeyCode::Char('a'), None, Some(ApprovalChoice::AllowOnce)),
        (
            KeyCode::Char('s'),
            Some("fetch:https+api.github.com"),
            Some(ApprovalChoice::AllowSession {
                token: "fetch:https+api.github.com".to_owned(),
            }),
        ),
        (KeyCode::Char('n'), None, Some(ApprovalChoice::Deny)),
        (KeyCode::Char('d'), None, Some(ApprovalChoice::Deny)),
        (KeyCode::Esc, None, Some(ApprovalChoice::Deny)),
        (KeyCode::Enter, None, None),
    ] {
        let (mut app, mut answer) = approval_app(Some(long_detail()), grant);
        handle_key(&mut app, code, KeyModifiers::NONE);
        let decision = answer.try_recv();
        assert_eq!(
            decision.ok(),
            want,
            "{code:?} changed its answer under scrolling work"
        );
        match want {
            Some(_) => assert!(
                app.request.pending_approval.is_none(),
                "{code:?} answered but the modal stayed"
            ),
            None => assert!(
                app.request.pending_approval.is_some(),
                "{code:?} answered nothing but the modal left"
            ),
        }
    }
}

/// Slice A's contract again, extended to the new scroll keys: a held
/// Ctrl/Alt/Super answers nothing — and, chosen here, scrolls nothing
/// either. The panel's paint is byte-identical after a modified arrow.
#[test]
fn ctrl_modified_keys_still_answer_nothing() {
    for (code, mods) in [
        (KeyCode::Char('s'), KeyModifiers::CONTROL),
        (KeyCode::Char('n'), KeyModifiers::CONTROL),
        (KeyCode::Down, KeyModifiers::CONTROL),
        (KeyCode::PageDown, KeyModifiers::ALT),
    ] {
        let (mut app, mut answer) =
            approval_app(Some(long_detail()), Some("fetch:https+api.github.com"));
        let before = render_buffer(&app, &fixed_status(), 80, 24);
        handle_key(&mut app, code, mods);
        let decision = answer.try_recv();
        assert!(
            matches!(decision, Err(TryRecvError::Empty)),
            "{code:?} with {mods:?} answered the modal: {decision:?}"
        );
        assert!(
            app.request.pending_approval.is_some(),
            "{code:?} with {mods:?} answered the modal"
        );
        let after = render_buffer(&app, &fixed_status(), 80, 24);
        assert_eq!(before, after, "{code:?} with {mods:?} moved the panel");
    }
}
