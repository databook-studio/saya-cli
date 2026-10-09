/// An older session file written before the `arguments` and `result_shape`
/// fields existed still loads: the new fields are `#[serde(default)]`, so a
/// tool record carrying only `name` and `status` deserializes with empty
/// arguments and a `None` shape. Old and new session files interoperate.
#[test]
fn an_old_session_file_without_the_new_tool_fields_still_loads() {
    let root = std::env::temp_dir().join(format!("saya-old-tool-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("old.json"),
        r#"{"version":2,"id":"old","profile_names":["analytics"],"turns":[{"user":"q","assistant":"a","database_derived":true,"tools":[{"name":"bounded_sql_query","status":"completed"}]}],"messages":[]}"#,
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
    let tool = &state.turns[0].tools[0];
    assert_eq!(tool.name, "bounded_sql_query");
    assert_eq!(tool.status, "completed");
    assert_eq!(tool.arguments, "", "missing arguments default to empty");
    assert!(
        tool.result_shape.is_none(),
        "missing result_shape defaults to None"
    );
    std::fs::remove_dir_all(root).unwrap();
}

/// The resume pin: a session whose record carries a workspace root re-opens
/// that root on resume, whatever directory the resume happens from — the
/// root follows the record, not the shell. A session written before the
/// workspace existed carries no root and resumes unbound, exactly its old
/// behaviour.
#[test]
fn the_recorded_workspace_root_rides_the_resume() {
    let root = std::env::temp_dir().join(format!("saya-ws-pin-{}", std::process::id()));
    let store = FsSessionStore::new(&root);
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "pinned".into(),
        workspace_root: Some("/projects/saya".into()),
        ..Default::default()
    }))
    .unwrap();
    let state = load_session(
        &store,
        &cli(),
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert_eq!(
        state.workspace_root.as_deref(),
        Some("/projects/saya"),
        "the pin follows the record, not the resume cwd"
    );
    // A record written before the workspace existed: no root, unbound.
    super::block_on(store.save(RedactedSession {
        version: saya_store::SESSION_VERSION,
        id: "legacy".into(),
        ..Default::default()
    }))
    .unwrap();
    let legacy = Cli {
        options: GlobalOptions {
            resume: Some("legacy".into()),
            ..Default::default()
        },
        command: None,
    };
    let old = load_session(
        &store,
        &legacy,
        &SessionDefaults {
            provider: "openai".into(),
            model: "current-model".into(),
            allow_data_sharing: true,
            approval_mode: "ask".into(),
        },
    )
    .unwrap();
    assert!(
        old.workspace_root.is_none(),
        "no record, no pin: the old session resumes unbound"
    );
    let _ = std::fs::remove_dir_all(root);
}
