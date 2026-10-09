use super::*;

/// A universe with a handle but NO workspace exposes none of it: no
/// workspace, no file tools, no recorder — even with the flag on, the
/// attach is a no-op and the write refuses exactly as unattached.
#[tokio::test]
async fn an_unbound_universe_exposes_no_handle_and_attaches_nothing() {
    let root = temp_dir("unbound");
    let plain = temp_dir("unbound-plain");
    let state = temp_dir("unbound-state");
    let runtime = test_runtime();
    let handle = RecoveryHandle::new(&root.join("sessions"), "unbound-session");
    let universe = SessionUniverse::compose_with_launch_and_path(SessionComposition {
        runtime: &runtime,
        explicit: None,
        pinned_root: None,
        walk_when_unpinned: true,
        cwd: &plain,
        state_dir: &state,
        launch: None,
        path: Some("test-path".to_owned()),
        scratch: None,
        recovery: Some(handle.clone()),
    })
    .expect("an unbound session composes");
    assert!(universe.root().is_none(), "outside a worktree, no root");
    assert!(
        universe.recovery_handle().is_none(),
        "no workspace means no handle, whatever the flag says"
    );

    handle.set_enabled(true);
    let args = serde_json::json!({"path": "x.txt", "content": "y"});
    let unattached = DatabaseTools::new(None, 100, true)
        .execute("workspace_write", args.clone())
        .await
        .expect_err("no workspace, no write");
    let attached = attach_recovery(DatabaseTools::new(None, 100, true), Some(&universe))
        .execute("workspace_write", args)
        .await
        .expect_err("the attach changed nothing without a workspace");
    assert_eq!(
        unattached, attached,
        "the unbound universe's attach is a no-op"
    );
    let _ = fs::remove_dir_all(&root);
}

/// The one-shot `ask` path passes no session: the attach is a no-op and
/// the tools behave exactly as unattached ones.
#[tokio::test]
async fn attaching_without_a_session_is_a_no_op() {
    let args = serde_json::json!({"path": "x.txt", "content": "y"});
    let attached = attach_recovery(DatabaseTools::new(None, 100, true), None)
        .execute("workspace_write", args.clone())
        .await
        .expect_err("no workspace, no write");
    let plain = DatabaseTools::new(None, 100, true)
        .execute("workspace_write", args)
        .await
        .expect_err("no workspace, no write");
    assert_eq!(attached, plain, "the ask path attaches nothing");
}

/// The runtime-level guard: the handle names the acquired id and starts
/// off, and a trust rebind — which recomposes the universe — keeps the
/// SAME flag cell, so the session's toggle survives it.
#[test]
fn the_handle_rides_the_acquired_id_and_a_trust_rebind_keeps_the_flag() {
    let root = temp_dir("rebind");
    let trusted = temp_dir("rebind-trusted");
    let sessions_root = root.join("sessions");
    let runtime = test_runtime();
    let mut session = SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "rebind-session",
        ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the fresh session acquires");
    let handle = session.recovery().clone();
    assert_eq!(handle.parts().1, "rebind-session", "the acquired id");
    assert!(!handle.is_enabled(), "OFF at acquisition");
    handle.set_enabled(true);

    session
        .bind_trusted(&runtime, &trusted, None)
        .expect("the trust answer rebinds");
    let rebound = session
        .universe()
        .recovery_handle()
        .expect("the trusted workspace binds");
    assert!(rebound.is_enabled(), "the flag rides the rebind");
    assert_eq!(rebound.parts().1, "rebind-session", "the same id");
    assert!(
        Arc::ptr_eq(&rebound.parts().2, &handle.parts().2),
        "one flag cell per session, across the rebind"
    );
    drop(session);
    let _ = fs::remove_dir_all(&root);
    let _ = fs::remove_dir_all(&trusted);
}
