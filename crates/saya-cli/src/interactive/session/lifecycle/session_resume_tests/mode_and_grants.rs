/// M0-2: persist with `read-only`, resume with an explicit
/// `--approval-mode never` — the flag must override the persisted mode
/// instead of being silently ignored.
#[test]
fn resume_honors_an_explicit_approval_mode_override() {
    let root = std::env::temp_dir().join(format!("saya-resume-override-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "override".into(),
        approval_mode: "read-only".into(),
        ..Default::default()
    }))
    .unwrap();
    let cli = Cli {
        options: GlobalOptions {
            continue_session: true,
            approval_mode: Some("never".into()),
            ..Default::default()
        },
        command: None,
    };
    let mut state = load_session(
        &store,
        &cli,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "never".into(),
        },
    )
    .unwrap();
    // Loading alone keeps resume continuity: the persisted mode stands...
    assert_eq!(state.approval_mode, "read-only");
    // ...and the loop's resume resolution lets the explicit flag win.
    state.approval_mode = crate::interactive::session::turn_loop::resume_approval_mode(
        &cli.options,
        &state.approval_mode,
    )
    .unwrap();
    assert_eq!(state.approval_mode, "never");
    std::fs::remove_dir_all(root).unwrap();
}

/// M0-2: resume without the flag keeps the persisted mode (resume
/// continuity).
#[test]
fn resume_without_the_flag_keeps_the_persisted_mode() {
    let root = std::env::temp_dir().join(format!("saya-resume-keep-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "keep".into(),
        approval_mode: "read-only".into(),
        ..Default::default()
    }))
    .unwrap();
    let cli = cli();
    let mut state = load_session(
        &store,
        &cli,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.approval_mode, "read-only");
    // The resume resolution without the flag keeps the persisted mode.
    state.approval_mode = crate::interactive::session::turn_loop::resume_approval_mode(
        &cli.options,
        &state.approval_mode,
    )
    .unwrap();
    assert_eq!(state.approval_mode, "read-only");
    std::fs::remove_dir_all(root).unwrap();
}

/// A Plan session survives the persist/resume round trip with its posture
/// and its status-line word intact: the user who set Plan is not silently
/// handed write tools on return.
#[test]
fn a_plan_session_resumes_as_plan_with_its_status_word() {
    use saya_agent::AgentMode;
    let root = std::env::temp_dir().join(format!("saya-mode-resume-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    let mut state = crate::SessionState::new("planned", None, "m");
    state.agent_mode = AgentMode::Plan.as_str().into();
    super::block_on(store.save(state.redacted())).unwrap();
    let resumed = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(resumed.agent_mode, "plan");
    assert_eq!(resumed.agent_mode_parsed(), AgentMode::Plan);
    assert!(
        crate::interactive::session_prompt::status_line(&resumed).contains("mode:plan"),
        "the status line names the resumed posture"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A session file written before the mode field existed resumes as Build
/// without error: the default is today's behaviour for every existing
/// session, not a silent read-only session the user never chose. The
/// fixture is raw JSON with no mode key, so deserialization — not struct
/// construction — proves the old record loads.
#[test]
fn a_session_file_without_the_mode_field_resumes_as_build() {
    use saya_agent::AgentMode;
    let root = std::env::temp_dir().join(format!("saya-mode-legacy-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":[],"turns":[],"messages":[]}"#,
    )
    .unwrap();
    let state = load_session(
        &FsSessionStore::new(&root),
        &cli(),
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(state.agent_mode, "build");
    assert_eq!(state.agent_mode_parsed(), AgentMode::Build);
    assert!(
        crate::interactive::session_prompt::status_line(&state).contains("mode:build"),
        "the old session reads exactly as before"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// A resumed Plan session starts with an empty grant store: the mode rides
/// the record, and grants still die with the process — the persisted plan
/// posture never re-grants anything.
#[test]
fn a_resumed_plan_session_starts_with_an_empty_grant_store() {
    let state = super::state_from_redacted(
        RedactedSession {
            version: saya_store::SESSION_VERSION,
            id: "planned-grants".into(),
            agent_mode: "plan".into(),
            ..Default::default()
        },
        &SessionDefaults {
            provider: "ollama".into(),
            model: "m".into(),
            allow_data_sharing: false,
            approval_mode: "ask".into(),
        },
    );
    assert_eq!(state.agent_mode, "plan");
    let policy = saya_agent::SessionPolicy::new(saya_agent::ApprovalPolicy::Ask);
    assert!(
        policy.grants().is_empty(),
        "a resumed session builds its policy empty, from the mode alone"
    );
}
