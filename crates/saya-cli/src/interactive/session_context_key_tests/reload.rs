/// The `/reload` arms' shape at the helper level: the re-seed runs after the
/// `state.apply` comparison, so the key is captured before it and the shared
/// abandon step runs after a successful reload. A re-seeded model (an
/// untouched session's copy) moves the key and retires the pending
/// question; a live-only change (max_rows) touches nothing and the pending
/// question survives — the exact decision the headless REPL arm mirrors.
#[test]
fn a_reload_s_reseeded_model_retires_the_pending_question_and_a_live_only_change_preserves_it() {
    use crate::config::reload::{apply_plan, plan_reload};

    // Reseeded model: the key moves, the question authority retires.
    let mut h = Harness::new("reload-model");
    h.state.model = h.runtime.resolved.ai.model.clone();
    let pending = h.pend("reload model");
    let before = h.state.context_key();
    let mut new = test_runtime();
    new.resolved.ai.model = "m2".into();
    let plan = plan_reload(&h.runtime, &new, &h.state);
    apply_plan(&plan, &mut h.state);
    h.state
        .abandon_question_when_context_changed(&before, &mut h.session);
    assert!(
        pending.cancellation().is_cancelled(),
        "the re-seeded model moved the answering context: the question is abandoned"
    );
    let next = h.begin("reload model");
    assert!(
        !next.is_same_question(&pending),
        "the next begin mints a different question after the re-seed"
    );
    drop(h);

    // Live-only change (max_rows): nothing is re-seeded, the key stands.
    let mut h = Harness::new("reload-rows");
    h.state.model = h.runtime.resolved.ai.model.clone();
    let pending = h.pend("reload rows");
    let before = h.state.context_key();
    let mut new = test_runtime();
    new.resolved.max_rows = 7;
    let plan = plan_reload(&h.runtime, &new, &h.state);
    apply_plan(&plan, &mut h.state);
    h.state
        .abandon_question_when_context_changed(&before, &mut h.session);
    let next = h.begin("reload rows");
    assert!(
        next.is_same_question(&pending),
        "a live-only reload preserves the pending question"
    );
    assert!(
        !next.cancellation().is_cancelled(),
        "the preserved question's token stays live"
    );
}
