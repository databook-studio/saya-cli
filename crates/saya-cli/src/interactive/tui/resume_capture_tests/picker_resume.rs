/// (a) PICKER success: after a real `/sql` capture, a successful
/// `tick_resume` clears BOTH the captured result and the last query, and the
/// resume message is shown.
#[test]
fn the_picker_resume_clears_the_previous_sessions_capture_and_last_query() {
    let mut fx = Seed::new("success", "cap-live");
    fx.capture_marker_query();
    block_on(
        fx.store
            .save(SessionState::new("resumed-ok", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.app.overlays.pending_resume = Some("resumed-ok".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-ok")),
        "the picker resume said it resumed, not {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        fx.app.captured.is_none(),
        "the resumed session inherited the previous session's capture"
    );
    assert!(
        fx.app.last_query.is_none(),
        "the resumed session inherited the previous session's last query"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// (b) PICKER refusal: with the resumed id's lock held (the D4 technique),
/// the refused resume changes NOTHING — both the real captured result and
/// the last query survive.
#[test]
fn the_refused_picker_resume_keeps_the_previous_sessions_capture_and_last_query() {
    let mut fx = Seed::new("refusal", "cap-held-live");
    fx.capture_marker_query();
    let blocker = SessionRuntime::acquire(
        &fx.runtime,
        None,
        false,
        None,
        "resumed-held",
        ApprovalPolicy::Ask,
        &fx.root.join("session-dirs"),
    )
    .expect("the blocker session acquires");
    block_on(
        fx.store
            .save(SessionState::new("resumed-held", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.app.overlays.pending_resume = Some("resumed-held".into());
    tick_resume(
        &mut fx.app,
        &fx.store,
        &mut fx.state,
        fx.runtime.as_ref(),
        &mut fx.session,
    );
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("already running")),
        "the refused resume said why, not {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    let capture = fx
        .app
        .captured
        .as_ref()
        .expect("the refused resume kept the capture");
    assert_eq!(
        capture.result.rows[0][0], MARKER,
        "the kept capture is the real captured result"
    );
    assert!(
        fx.app.last_query.is_some(),
        "the refused resume kept the last query"
    );
    let root = fx.root.clone();
    drop(blocker);
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
