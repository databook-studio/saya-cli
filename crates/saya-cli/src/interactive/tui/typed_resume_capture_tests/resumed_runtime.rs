use super::*;

/// (c) TYPED success re-derives the header facts from the resumed universe:
/// the swap installs the loaded state (its runtime-only `host_composed` and
/// deny list are the serde-skip defaults), while `reacquire` recomposed the
/// universe — the resumed root binds and the lane composes, so the header
/// must say what the resumed session will actually run, with the deny list
/// the new universe carries, never the previous session's stale one.
#[test]
fn the_typed_resume_re_derives_the_header_facts_from_the_resumed_universe() {
    let mut fx = Seed::new("typed-host", "cap-typed-host-live");
    let project = fx.root.join("resumed-project");
    std::fs::create_dir_all(&project).expect("the resumed project dir");
    let mut resumed = SessionState::new("resumed-host-posture", None, "test-model");
    resumed.workspace_root = Some(project.display().to_string());
    block_on(fx.store.save(resumed.redacted())).expect("the resumed record saves");
    // The previous session's launch-derived shape: its header was right for
    // IT — the swap must re-derive, not carry either value across.
    fx.state.host_composed = true;
    fx.state.denied_programs = vec!["curl".to_owned()];

    fx.drain_pending("/resume resumed-host-posture");
    assert!(
        fx.app
            .transcript
            .blocks()
            .iter()
            .any(|block| block.text.contains("Resumed session resumed-host-posture")),
        "the typed resume said it resumed, not {:?}",
        fx.app
            .transcript
            .blocks()
            .iter()
            .map(|b| b.text.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        fx.state.host_composed,
        "the resumed universe composes the lane: the header says host:unsandboxed"
    );
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the resumed universe's fact"
    );
    assert!(
        fx.state.denied_programs.is_empty(),
        "the previous session's stale deny list does not survive the swap: {:?}",
        fx.state.denied_programs
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the resumed universe's fact"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}

/// (d) TYPED success adopts the resumed session's runtime metrics collector:
/// `App.session_metrics` must be the NEW collector `session.session_metrics()`
/// now hands out (`reacquire` replaced the runtime). The enable flag is
/// flipped exactly as local_metrics_tests does; the initial acquire runs
/// while it is off, so the pre-fix app handle (None) cannot vacuously match.
#[test]
fn the_typed_resume_adopts_the_resumed_sessions_metrics_collector() {
    let mut fx = Seed::new("typed-metrics", "cap-typed-metrics-live");
    block_on(
        fx.store
            .save(SessionState::new("resumed-metrics", None, "test-model").redacted()),
    )
    .expect("the resumed record saves");
    set_enabled(true);
    fx.drain_pending("/resume resumed-metrics");
    let adopted = matches!(
        (&fx.app.session_metrics, &fx.session.session_metrics()),
        (Some(app_handle), Some(new_handle)) if Arc::ptr_eq(app_handle, new_handle)
    );
    set_enabled(false);
    assert!(
        adopted,
        "the app kept the previous session's metrics collector (or lost it)"
    );
    let root = fx.root.clone();
    drop(fx);
    wait_for_background_save(&root.join("store"));
    let _ = std::fs::remove_dir_all(&root);
}
