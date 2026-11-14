#[test]
fn data_sharing_never_moves_for_a_running_session() {
    let off = runtime_from("[ai]\nallow_data_sharing = false\n", "");
    let on = runtime_from("[ai]\nallow_data_sharing = true\n", "");
    let state = state_with("m");
    let widen = plan_reload(&off, &on, &state);
    assert_eq!(widen.pending.len(), 1);
    assert_eq!(widen.pending[0].key, "allow_data_sharing");
    assert_eq!(
        widen.pending[0].note.as_deref(),
        Some(
            "data sharing is fixed for this session; use /privacy to change it here, or start a new session"
        )
    );
    assert_eq!(widen.reseed, StateReseed::default());
    let mut session = state.clone();
    apply_plan(&widen, &mut session);
    assert!(
        !session.allow_data_sharing,
        "a reload must not widen sharing"
    );
    let narrow = plan_reload(&on, &off, &state);
    assert_eq!(narrow.pending[0].key, "allow_data_sharing");
    let mut sharing = state.clone();
    sharing.allow_data_sharing = true;
    apply_plan(&narrow, &mut sharing);
    assert!(
        sharing.allow_data_sharing,
        "a reload must not narrow sharing"
    );
}

#[test]
fn host_command_shaping_waits_for_a_new_session() {
    let old = runtime_from("", "");
    let new = runtime_from("[host_commands]\ntimeout_seconds = 99\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "host_commands");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("takes effect in a new session")
    );
}

#[test]
fn session_denies_wait_for_a_new_session() {
    let old = runtime_from("", "");
    let new = runtime_from("[session_commands]\ndeny = [\"curl\"]\n", "");
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.pending.len(), 1);
    assert_eq!(plan.pending[0].key, "session_commands");
    assert_eq!(
        plan.pending[0].note.as_deref(),
        Some("takes effect in a new session")
    );
}

#[test]
fn a_new_profile_is_one_connections_change_and_reseeds_the_profile_copy() {
    let old = runtime_from("", "");
    let new = runtime_from(
        "",
        "[profiles.analytics]\ntype = \"sqlite\"\npath = \"/tmp/x.sqlite\"\n",
    );
    let plan = plan_reload(&old, &new, &state_with("m"));
    assert_eq!(plan.applied.len(), 1);
    assert_eq!(plan.applied[0].key, "connections");
    assert_eq!(plan.applied[0].before, json!([]));
    assert_eq!(plan.applied[0].after, json!(["analytics"]));
    assert_eq!(plan.reseed.profile, Some(Some("analytics".to_owned())));
    let mut session = state_with("m");
    apply_plan(&plan, &mut session);
    assert_eq!(session.profile.as_deref(), Some("analytics"));
}
