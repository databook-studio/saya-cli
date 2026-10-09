use super::*;

/// The real wiring: the flag OFF leaves the create byte-identical to today
/// — no `recovery` key, no checkpoint data — and flipping the session's
/// flag ON makes a guarded replace commit exactly one checkpoint whose
/// before-image is the pre-edit bytes, under the acquired session id.
#[tokio::test]
async fn flag_off_matches_today_and_flag_on_records_the_replace() {
    let root = temp_dir("wiring");
    let project = temp_dir("wiring-project");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let session = SessionRuntime::acquire(
        &runtime,
        Some(&project),
        true,
        None,
        "attach-session",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the session acquires with an explicit workspace");

    // One handle per acquisition: the acquired id, flag off, and the
    // universe exposes it only because a workspace is bound.
    let handle = session.recovery().clone();
    assert_eq!(handle.parts().1, "attach-session", "the acquired id");
    assert!(
        !handle.is_enabled(),
        "recovery is opt-in: OFF at acquisition"
    );
    let universe = session.universe();
    let exposed = universe
        .recovery_handle()
        .expect("a bound workspace exposes the handle");
    assert!(
        Arc::ptr_eq(&exposed.parts().2, &handle.parts().2),
        "the universe carries the same flag cell"
    );

    // Flag OFF: the create through the session executor is byte-identical
    // to today — no `recovery` key, no checkpoints directory.
    let created = Arc::new(attached_tools(&universe));
    let result = universe
        .executor(Arc::clone(&created), &CancellationToken::new(), false)
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes.txt", "content": "first line\n"}),
        )
        .await
        .expect("the create succeeds");
    assert!(
        result.get("recovery").is_none(),
        "flag off: no recovery field"
    );
    assert!(
        !sessions_root
            .join("attach-session")
            .join("checkpoints")
            .exists(),
        "flag off: no checkpoint data is stored"
    );
    let digest = result["digest"]
        .as_str()
        .expect("the create names the digest")
        .to_owned();

    // Flag ON — the session's own cell, what the toggle will flip: a
    // guarded replace records the pre-edit bytes exactly, once.
    handle.set_enabled(true);
    let replaced = Arc::new(attached_tools(&universe));
    let result = universe
        .executor(Arc::clone(&replaced), &CancellationToken::new(), false)
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "notes.txt",
                "content": "second line\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the guarded replace succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let (store, session_id, _) = handle.parts();
    assert_eq!(session_id, "attach-session");
    let records = store
        .list_checkpoints("attach-session")
        .expect("the listing reads back");
    assert_eq!(records.len(), 1, "exactly the replace was recorded");
    assert_eq!(records[0].state, CheckpointState::Committed);
    let image = store
        .read_before_image("attach-session", records[0].id.as_str())
        .expect("the before-image reads back");
    assert_eq!(
        image, b"first line\n",
        "the stored image is the pre-edit bytes"
    );
    drop(session);
    let _ = fs::remove_dir_all(&root);
}
