use super::*;

#[test]
fn enter_never_approves_a_pending_modal() {
    // The modal can appear while the user is mid-thought; an implicit
    // Enter (e.g. submitting their next prompt) must never allow SQL.
    assert_eq!(approval_answer(KeyCode::Enter), None);
}

#[test]
fn only_explicit_y_approves_n_and_esc_deny() {
    assert_eq!(approval_answer(KeyCode::Char('y')), Some(true));
    assert_eq!(approval_answer(KeyCode::Char('Y')), Some(true));
    assert_eq!(approval_answer(KeyCode::Char('n')), Some(false));
    assert_eq!(approval_answer(KeyCode::Char('N')), Some(false));
    assert_eq!(approval_answer(KeyCode::Esc), Some(false));
    assert_eq!(approval_answer(KeyCode::Tab), None);
}

/// Property 2 (key half): `approvals_are_untouched` — grouping leaves the
/// approval keys untouched. The group toggle rides Enter on an empty line
/// (`handle_key` line 231); the modal answers ride `y`/`a`/`s`/`n`/`d`/Esc
/// (`approval_choice`), and Enter is never an answer. A collapsed group
/// therefore cannot steal a consent keystroke, and a consent keystroke
/// cannot toggle a group: the modal arm runs first and returns.
#[test]
fn grouping_leaves_approval_keys_untouched() {
    assert_eq!(
        approval_choice(KeyCode::Enter, Some("workspace-write")),
        None,
        "Enter never answers, so it can keep toggling groups"
    );
    assert_eq!(
        approval_choice(KeyCode::Char('e'), None),
        None,
        "bare e stays a typed character, never a toggle"
    );
    for code in [
        KeyCode::Char('y'),
        KeyCode::Char('a'),
        KeyCode::Char('s'),
        KeyCode::Char('n'),
        KeyCode::Char('d'),
        KeyCode::Esc,
    ] {
        assert!(
            approval_choice(code, Some("workspace-write")).is_some(),
            "consent keys stay answers: {code:?}"
        );
    }
}

/// The tool modal's three answers: `y`/`a` allow once, `s` grants exactly
/// the offered token, `n`/`d`/Esc deny. Enter is still not an approval,
/// and an unoffered `s` is not an answer at all — the modal never offered
/// a grant, so the key must not invent one.
#[test]
fn the_tool_modal_answers_grant_only_the_offered_token() {
    use saya_agent::ApprovalChoice;
    assert_eq!(
        approval_choice(KeyCode::Char('y'), Some("workspace-write")),
        Some(ApprovalChoice::AllowOnce)
    );
    assert_eq!(
        approval_choice(KeyCode::Char('a'), None),
        Some(ApprovalChoice::AllowOnce)
    );
    assert_eq!(
        approval_choice(KeyCode::Char('s'), Some("workspace-write")),
        Some(ApprovalChoice::AllowSession {
            token: "workspace-write".to_owned()
        })
    );
    assert_eq!(
        approval_choice(KeyCode::Char('s'), None),
        None,
        "no token offered, no grant to take: the modal stays"
    );
    assert_eq!(
        approval_choice(KeyCode::Char('n'), None),
        Some(ApprovalChoice::Deny)
    );
    assert_eq!(
        approval_choice(KeyCode::Char('d'), None),
        Some(ApprovalChoice::Deny)
    );
    assert_eq!(
        approval_choice(KeyCode::Esc, None),
        Some(ApprovalChoice::Deny)
    );
    assert_eq!(
        approval_choice(KeyCode::Enter, Some("workspace-write")),
        None,
        "Enter is still never an approval"
    );
}

/// Enter on an empty line toggles the latest foldable chapter (or the
/// tool group first), and it must never answer a modal while doing so.
#[test]
fn enter_still_never_approves_while_a_chapter_is_foldable() {
    use crate::interactive::tui::application::tests_support::idle_app;
    use crate::interactive::tui::transcript::BlockKind;

    assert_eq!(
        approval_choice(KeyCode::Enter, Some("workspace-write")),
        None,
        "Enter never answers, so it can keep toggling chapters"
    );
    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript
        .push(BlockKind::Assistant, "the red orders total 42");
    app.transcript.push(BlockKind::User, "and the blue ones");
    let unfolded: Vec<String> = app
        .transcript
        .wrapped(80)
        .iter()
        .map(|row| row.text.clone())
        .collect();
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        app.transcript.is_folded(1),
        "Enter on an empty line folds the finished chapter"
    );
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    let reopened: Vec<String> = app
        .transcript
        .wrapped(80)
        .iter()
        .map(|row| row.text.clone())
        .collect();
    assert_eq!(unfolded, reopened, "the same keystroke reopens it");
}

/// Enter prefers the tool group: with both a collapsed group and a
/// foldable chapter, the group toggles and the chapter stays open.
#[test]
fn enter_toggles_the_tool_group_before_any_chapter() {
    use crate::interactive::tui::application::tests_support::idle_app;
    use crate::interactive::tui::transcript::BlockKind;

    let mut app = idle_app();
    app.transcript.push(BlockKind::User, "count the red orders");
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "SELECT 1"}),
        None,
    );
    assert!(
        app.transcript
            .buffer_tool_completion("bounded_sql_query", "finished call one")
    );
    app.transcript.buffer_tool_request(
        "bounded_sql_query".into(),
        serde_json::json!({"sql": "SELECT 2"}),
        None,
    );
    assert!(
        app.transcript
            .buffer_tool_completion("bounded_sql_query", "finished call two")
    );
    app.transcript.flush_tool_buffer(
        |name, _| vec![format!("→ {name}")],
        |name, summary| format!("✓ {name}: {summary}"),
    );
    app.transcript.push(BlockKind::User, "and the blue ones");
    handle_key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
    assert!(
        !app.transcript.is_folded(1),
        "the tool group wins; the chapter stays open"
    );
}
