use super::*;

/// (a) TYPED success: after a real `/sql` capture (and a planted gap), a
/// successful typed `/resume <id>` through the real `tick_pending` clears
/// the capture, the last query, AND the agent-capture gap.
#[test]
fn the_typed_resume_clears_the_previous_sessions_capture_last_query_and_gap() {
    let mut fx = Seed::new("typed-success", "cap-typed-live");
    fx.capture_marker_query();
    fx.app.agent_captures.gap = Some(CaptureGap::Missing);
    block_on(
        fx.store
            .save(SessionState::new("resumed-typed-ok", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.drain_pending("/resume resumed-typed-ok");
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-typed-ok")),
        "the typed resume said it resumed, not {:?}",
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
    assert!(
        fx.app.agent_captures.gap.is_none(),
        "the resumed session inherited the previous session's capture gap"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// (b) TYPED refusal: with the resumed id's lock held (the D4 technique),
/// the refused typed resume changes NOTHING — the capture, the last query,
/// and the gap all survive for the still-live session.
#[test]
fn the_refused_typed_resume_keeps_the_previous_sessions_query_state() {
    let mut fx = Seed::new("typed-refusal", "cap-typed-held-live");
    fx.capture_marker_query();
    fx.app.agent_captures.gap = Some(CaptureGap::Missing);
    let blocker = SessionRuntime::acquire(
        &fx.runtime,
        None,
        false,
        None,
        "resumed-typed-held",
        ApprovalPolicy::Ask,
        &fx.root.join("session-dirs"),
    )
    .expect("the blocker session acquires");
    block_on(
        fx.store
            .save(SessionState::new("resumed-typed-held", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    fx.drain_pending("/resume resumed-typed-held");
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
    assert!(
        matches!(fx.app.agent_captures.gap, Some(CaptureGap::Missing)),
        "the refused resume kept the gap"
    );
    let root = fx.root.clone();
    drop(blocker);
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
