use super::*;

/// One table, one walk: every transition `settle_question` may take, keyed
/// by WHICH scope completed — the scope `begin_question` handed out settles
/// itself, a stale scope's completion is ignored outright. The rows share
/// one real session and evolve its state in order; each row's asserts carry
/// the row label.
#[test]
fn settle_question_transitions_by_completing_scope_identity() {
    let table: Vec<Row> = vec![
        (
            "Idle + begin -> Active, the scope's token live",
            Box::new(|h, label| {
                let a = h.begin(label);
                assert!(
                    !a.cancellation().is_cancelled(),
                    "{label}: a begun question's token is live"
                );
            }),
        ),
        (
            "Active + overlapping begin -> refused, state unchanged",
            Box::new(|h, label| {
                h.begin_refused(label);
                let a = h.begun[0].clone();
                assert!(
                    !a.cancellation().is_cancelled(),
                    "{label}: the refused begin left the active question untouched"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=true, token live) -> Pending; the next begin re-begins the SAME question",
            Box::new(|h, label| {
                let a = h.begun[0].clone();
                h.session.settle_question(&a, true);
                let b = h.begin(label);
                assert!(
                    b.is_same_question(&a),
                    "{label}: pending clarification re-begins the same question"
                );
                assert!(
                    !b.cancellation().is_cancelled() && !a.cancellation().is_cancelled(),
                    "{label}: the re-begun question keeps its live token"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=false) -> Idle; the next begin mints a DIFFERENT question",
            Box::new(|h, label| {
                let a = h.begun[0].clone();
                h.session.settle_question(&a, false);
                let c = h.begin(label);
                assert!(
                    !c.is_same_question(&a),
                    "{label}: the settled-out question is gone; the next begin is a different question"
                );
            }),
        ),
        (
            "Active + settle(same, clarification=true, token cancelled) -> Idle",
            Box::new(|h, label| {
                let c = h.begun[2].clone();
                c.cancellation().cancel();
                h.session.settle_question(&c, true);
                let d = h.begin(label);
                assert!(
                    !d.is_same_question(&c),
                    "{label}: a cancelled settle cannot pend; the next begin is a different question"
                );
                assert!(
                    !d.cancellation().is_cancelled(),
                    "{label}: the new question's token is live"
                );
            }),
        ),
        (
            "Active(current) + settle(STALE other scope, clarification=true) -> ignored: current stays Active, its token stays live",
            Box::new(|h, label| {
                let stale = h.begun[2].clone();
                let current = h.begun[3].clone();
                h.session.settle_question(&stale, true);
                h.begin_refused(label);
                assert!(
                    !current.cancellation().is_cancelled(),
                    "{label}: the stale completion did not cancel the live question"
                );
                // The live question is still `current`: a same-scope
                // clarification=false settle lands (so the next begin mints
                // a fresh question), which only happens when the live scope
                // was this one.
                h.session.settle_question(&current, false);
                h.begin(label);
            }),
        ),
        (
            "Pending + abandon -> Idle and the old token is cancelled",
            Box::new(|h, label| {
                let e = h.begun[4].clone();
                h.session.settle_question(&e, true);
                h.session.abandon_question();
                assert!(
                    e.cancellation().is_cancelled(),
                    "{label}: abandoning the pending question cancels its token"
                );
                let f = h.begin(label);
                assert!(
                    !f.is_same_question(&e),
                    "{label}: abandon left Idle; the next begin is a different question"
                );
            }),
        ),
        (
            "Pending + settle(stale) -> still Pending, the pending scope and token untouched",
            Box::new(|h, label| {
                let f = h.begun[5].clone();
                h.session.settle_question(&f, true);
                let stale = h.begun[4].clone();
                h.session.settle_question(&stale, true);
                let g = h.begin(label);
                assert!(
                    g.is_same_question(&f),
                    "{label}: the stale settle was ignored; the question is still pending"
                );
                assert!(
                    !g.cancellation().is_cancelled(),
                    "{label}: the stale settle neither cancelled nor finished the pending question"
                );
                assert!(
                    stale.cancellation().is_cancelled(),
                    "{label}: the stale scope stays settled-out; nothing resurrected it"
                );
            }),
        ),
    ];
    let mut harness = Harness::new();
    for (label, step) in &table {
        step(&mut harness, label);
    }
    let root = harness.sessions_root.clone();
    drop(harness);
    let _ = std::fs::remove_dir_all(&root);
}
