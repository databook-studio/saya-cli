use super::*;

/// `reacquire` is the one seam all three session-load paths share (the
/// headless `/resume`, the TUI slash `/resume`, and the TUI session picker
/// all call it), so its contract decides the outcome. One table, one walk: a
/// successful swap abandons the old question exactly like `abandon_question`
/// — the old token cancelled, the segment finished, Idle — and a refused
/// swap keeps the pending question: same scope, live token.
#[test]
fn reacquire_abandons_the_question_on_success_and_keeps_it_on_failure() {
    let table: Vec<Row> = vec![
        (
            "Pending + FAILED reacquire (the target's lock is held) -> still Pending: same scope, token live",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session.settle_question(&a, true);
                let blocker = SessionRuntime::acquire(
                    &h.runtime,
                    None,
                    false,
                    None,
                    "reacquire-held",
                    ApprovalPolicy::Ask,
                    &h.sessions_root,
                )
                .unwrap_or_else(|error| panic!("{label}: the blocker session acquires: {error}"));
                let error = h
                    .session
                    .reacquire(&h.runtime, None, "reacquire-held", ApprovalPolicy::Ask)
                    .expect_err("{label}: a held target refuses the swap");
                assert!(
                    error.contains("already running"),
                    "{label}: the refusal names the live holder: {error}"
                );
                let b = h.begin(label);
                assert!(
                    b.is_same_question(&a),
                    "{label}: the refused swap left the question pending"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the refused swap left the token live"
                );
                h.session.settle_question(&b, false);
                drop(blocker);
            }),
        ),
        (
            "Pending + SUCCESSFUL reacquire -> the old token is cancelled; the next begin mints a DIFFERENT question",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session.settle_question(&a, true);
                h.session
                    .reacquire(&h.runtime, None, "reacquired", ApprovalPolicy::Ask)
                    .unwrap_or_else(|error| {
                        panic!("{label}: the swap to a saved session succeeds: {error}")
                    });
                assert!(
                    a.cancellation().is_cancelled(),
                    "{label}: the successful swap cancelled the old question's token"
                );
                let b = h.begin(label);
                assert!(
                    !b.is_same_question(&a),
                    "{label}: the abandoned question is gone; the next begin is a different question"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
                h.session.settle_question(&b, false);
            }),
        ),
        (
            "Active + SUCCESSFUL reacquire -> the old token is cancelled, Idle; the next begin is a fresh question",
            Box::new(|h, label| {
                let a = h.begin(label);
                h.session
                    .reacquire(&h.runtime, None, "reacquired-again", ApprovalPolicy::Ask)
                    .unwrap_or_else(|error| panic!("{label}: the second swap succeeds: {error}"));
                assert!(
                    a.cancellation().is_cancelled(),
                    "{label}: the successful swap cancelled the active question's token"
                );
                let b = h.begin(label);
                assert!(
                    !b.is_same_question(&a),
                    "{label}: the abandoned question is gone; the next begin is a different question"
                );
                assert!(
                    !b.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
            }),
        ),
    ];
    let mut harness = Harness::build("reacquire", "reacquire-walk");
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}
