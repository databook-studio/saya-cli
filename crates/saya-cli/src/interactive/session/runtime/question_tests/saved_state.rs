use super::*;

/// Saved-state cannot grant question authority. A session whose question is
/// pending clarification saves its state through the real save path; the
/// record round-trips the real loader (`resume_session`, the load all three
/// paths call), which carries no question — the lifecycle never rides
/// `SessionState` — and the runtime side of the successful load is the
/// shared `reacquire` seam. (Resuming the runtime's own id is refused by the
/// single-writer lock — the holder is this very process — so the swap, like
/// any real resume of another record, claims a fresh state directory.) The
/// loaded runtime's first `begin_question` therefore mints a fresh scope
/// from the trusted runtime config — a new token, a new identity, no
/// restored handle/time/count — and nothing auto-runs.
#[test]
fn a_saved_pending_state_grants_no_question_authority() {
    let root = temp_sessions_root_named("saved");
    let runtime = test_runtime();
    let sessions_root = root.join("sessions");
    let store = FsSessionStore::new(root.join("store"));
    let mut session = SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "saved-authority",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("a real session runtime acquires");
    let old = session
        .begin_question(&runtime)
        .expect("the pending question begins");
    session.settle_question(&old, true);
    let state = SessionState::new("saved-authority", None, "test-model");
    block_on(store.save(state.redacted())).expect("the real save path persists the state");
    let loaded = resume_session(
        &store,
        "saved-authority",
        &SessionDefaults {
            provider: "ollama".into(),
            model: "test-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .expect("the record loads")
    .expect("the record exists");
    session
        .reacquire(
            &runtime,
            loaded.workspace_root.as_deref(),
            "resumed-fresh",
            ApprovalPolicy::Ask,
        )
        .expect("the successful load swaps in the loaded runtime");
    let fresh = session
        .begin_question(&runtime)
        .expect("the first begin on the loaded runtime mints a fresh question");
    assert!(
        !fresh.is_same_question(&old),
        "the saved narrative restored no question identity: the first begin is a fresh scope"
    );
    assert!(
        !fresh.cancellation().is_cancelled(),
        "the fresh scope carries a new, live token: no restored cancellation handle"
    );
    assert!(
        old.cancellation().is_cancelled(),
        "the old pending question was abandoned by the successful load, not carried across"
    );
    drop(session);
    drop(store);
    let _ = std::fs::remove_dir_all(root);
}
