/// A rotation re-derives the header facts from the SUCCESSOR's universe: the
/// swap installs a state whose runtime-only host facts are whatever the
/// parent state carried (stale false and a stale deny list — the shape a
/// `/resume`d state has while its recomposed universe composes the lane),
/// while `reacquire` recomposed the universe for the successor. The pinned
/// root re-binds, so the successor's lane composes and the header must say
/// so.
#[test]
fn a_rotation_syncs_the_header_facts_from_the_successor_s_universe() {
    let mut fx = Fixture::build_pinned("header-sync");
    assert!(
        fx.session.universe().host_composed(),
        "the pinned root composed the lane for the parent too"
    );
    // The stale shape the swap replaces: a state whose header facts never
    // said what its own universe composes (the resumed state), carrying a
    // deny list its universe does not.
    fx.state.host_composed = false;
    fx.state.denied_programs = vec!["curl".to_owned()];

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Rotated { .. } = outcome else {
        panic!("a compacted session rotates: {outcome:?}");
    };
    assert!(
        fx.state.host_composed,
        "the successor's pinned root re-binds: the lane composes for the header"
    );
    assert_eq!(
        fx.state.host_composed,
        fx.session.universe().host_composed(),
        "the header's lane bit is the successor universe's fact"
    );
    assert!(
        fx.state.denied_programs.is_empty(),
        "the parent's stale deny list does not survive the swap: {:?}",
        fx.state.denied_programs
    );
    assert_eq!(
        fx.state.denied_programs,
        fx.session.universe().deny_programs(),
        "the header's deny list is the successor universe's fact"
    );
}

/// The helper both surfaces share: the header facts are the live universe's,
/// never the state's carried-over values. A composed lane reads true with
/// the deny list copied; an unbound universe reads false.
#[test]
fn sync_host_posture_derives_the_header_facts_from_the_live_universe() {
    // None ⇒ false: the unbound fixture composes no lane.
    let unbound = Fixture::build("posture-unbound");
    let mut state = SessionState::new("s1", None, "m");
    state.host_composed = true;
    state.denied_programs = vec!["curl".to_owned()];
    state.sync_host_posture(&unbound.session.universe());
    assert!(
        !state.host_composed,
        "no workspace root binds: the lane did not compose"
    );
    assert_eq!(
        state.denied_programs,
        unbound.session.universe().deny_programs(),
        "the deny list is copied from the universe"
    );
    drop(unbound);

    // Composed ⇒ true, deny list copied: the pinned fixture composes the
    // lane, and a deny-only launch recomposition puts the names on the
    // universe the helper must copy.
    let mut pinned = Fixture::build_pinned("posture-pinned");
    let launch =
        crate::interactive::session_host::HostLaunch::from_deny_for_tests(vec!["curl".to_owned()]);
    pinned
        .session
        .recompose_with_launch(
            &pinned.runtime,
            pinned.state.workspace_root.as_deref(),
            Some(&launch),
        )
        .expect("the deny-only launch recomposes");
    let mut state = SessionState::new("s2", None, "m");
    state.sync_host_posture(&pinned.session.universe());
    assert!(
        state.host_composed,
        "the pinned root composes the lane: the header says host:unsandboxed"
    );
    assert_eq!(
        state.denied_programs,
        vec!["curl".to_owned()],
        "the deny list is copied from the universe: {:?}",
        state.denied_programs
    );
}

/// A live holder on the successor id stops the swap after the publish: the
/// successor is saved and resumable, the current session keeps its lock,
/// and the message names the resumable id.
#[test]
fn a_live_holder_on_the_successor_keeps_the_current_session() {
    let mut fx = Fixture::build("live-holder");
    let holder = fx
        .reacquire(SUCCESSOR)
        .expect("the holder acquires the successor id first");

    let outcome = rotate_session_with_id(
        &fx.store,
        &fx.runtime,
        &mut fx.state,
        &mut fx.session,
        SUCCESSOR,
    );
    let RotateResult::Incomplete(message) = outcome else {
        panic!("a held successor stops the swap: {outcome:?}");
    };
    assert!(
        message.contains("Rotation was not completed") && message.contains(SUCCESSOR),
        "the message names the resumable id: {message}"
    );
    let error = fx
        .reacquire(OLD)
        .err()
        .expect("the current session's lock is still held");
    assert!(error.contains("already running"), "{error}");

    let defaults = SessionDefaults {
        provider: "ollama".into(),
        model: "m".into(),
        allow_data_sharing: true,
        approval_mode: "ask".into(),
    };
    let loaded = resume_session(&fx.store, SUCCESSOR, &defaults)
        .expect("the successor resumes")
        .expect("the successor record exists");
    assert_eq!(
        loaded.compaction_summary.as_deref(),
        Some(SUMMARY),
        "the carried summary is restored"
    );
    assert_eq!(loaded.compacted_turns, 0);
    drop(holder);
}
