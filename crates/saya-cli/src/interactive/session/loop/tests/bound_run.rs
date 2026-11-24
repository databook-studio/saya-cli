#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod bound_run_tests {
    //! The opposite of an unbound launch: the folder the trust question answers
    //! `y` for binds like `--workspace`, so the run composed over it keeps the
    //! folder's own tools.
    //!
    //! The fixtures are the unbound rows' own ([`super::unbound_run_tests`]), so
    //! the two directions are one rule read twice: the folder the session bound
    //! is what its run's workspace is, and a session that bound none has none.

    use super::unbound_run_tests::{ID, advertises, run_over, runtime_over, scratch};
    use super::*;
    use crate::chat::Workspace;

    #[tokio::test]
    async fn answering_trust_y_binds_the_folder_and_the_next_run_keeps_its_tools() {
        let root = scratch("trusted");
        let runtime = runtime_over(&root);
        let folder =
            std::fs::canonicalize(root.join("launch-dir")).expect("the launch dir canonicalises");
        // A fresh start with no `--workspace`: the trust answer is the launch's
        // only statement, and it binds exactly like one.
        let mut session = SessionRuntime::acquire(
            &runtime,
            None,
            true,
            None,
            ID,
            saya_agent::ApprovalPolicy::Ask,
            &root.join("sessions"),
        )
        .expect("the fresh session acquires");
        session
            .bind_trusted(&runtime, &folder, None)
            .expect("the trust answer binds the launch folder");
        assert_eq!(
            session.root(),
            Some(folder.as_path()),
            "`y` binds the launch folder as the session's workspace"
        );

        let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
        let state = SessionState::new(ID, None, "test-model");
        let composed = surface_parts(&state, &session, &runtime, &state_db, RenderFormat::Text);
        let (has_workspace, tools) = run_over(composed.parts, &root).await;
        assert!(
            has_workspace,
            "the folder the operator trusted is the run's workspace"
        );
        let advertised = tools.definitions();
        for name in ["read", "write", "edit", "glob", "grep", "shell"] {
            assert!(
                advertises(&advertised, name),
                "`{name}` is advertised over the folder the operator trusted: {advertised:?}"
            );
        }
        assert_eq!(
            crate::chat::workspace_shape(&folder, &root.join("coding"), ID).workspace(),
            Workspace::Here,
            "the bound folder is a workspace the rule admits"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
