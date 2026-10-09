/// FIX-S1e (a): a resume must not start while a request is running. With a
/// compaction in flight, `tick_resume` refuses: the session id is unchanged,
/// the pending id is consumed, the transcript says why, nothing resumed.
#[test]
fn a_resume_is_refused_while_a_request_is_running() {
    let mut fx = Seed::new("busy", "busy-live");
    block_on(
        fx.store
            .save(SessionState::new("resumed-busy", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    let before = fx.state.id.clone();
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    fx.app.overlays.pending_resume = Some("resumed-busy".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert_eq!(fx.state.id, before, "the busy resume changed the session");
    assert!(
        fx.app.overlays.pending_resume.is_none(),
        "the busy resume kept the pending id"
    );
    let blocks: Vec<String> = fx
        .app
        .transcript
        .blocks()
        .iter()
        .map(|b| b.text.clone())
        .collect();
    assert!(
        blocks.iter().any(|text| text.contains("still running")),
        "the refused resume said why, not {:?}",
        blocks
    );
    assert!(
        !blocks.iter().any(|text| text.contains("Resumed session")),
        "the busy resume resumed the session anyway, not {:?}",
        blocks
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// FIX-S1e (b): a compaction is part of the one-query-at-a-time resource,
/// so `admit_second_replay` refuses while one runs instead of starting a
/// replay beside it (it would clobber the status and capture fields).
#[test]
fn a_second_replay_is_refused_while_a_compaction_runs() {
    let mut fx = Seed::new("replay", "replay-live");
    let (_tx, rx) = std::sync::mpsc::channel();
    fx.app.compact_task = Some(rx);
    assert!(
        matches!(fx.app.admit_second_replay(), SecondSqlDecision::Reject(_)),
        "the replay was admitted beside a running compaction"
    );
    let root = fx.root.clone();
    drop(fx);
    let _ = std::fs::remove_dir_all(&root);
}
