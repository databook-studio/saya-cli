#[cfg(test)]
mod status_turn_tests {
    use super::*;
    use saya_agent::ApprovalPolicy;
    use std::path::PathBuf;

    /// `/status` is a read: the headless turn completes, prints one panel with
    /// the five sections in order, and moves nothing — the session keeps its
    /// id, its provider and its model. The facts are the ones the surface
    /// builds from, so the panel this turn printed is pinned here through the
    /// same shared builder (the output itself goes through `print!`, the
    /// receipt-test limit).
    #[test]
    fn a_status_turn_prints_the_panel_and_moves_nothing() {
        let root = std::env::temp_dir().join(format!(
            "saya-status-turn-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the status-turn root");
        let runtime = super::setup_turn_tests::handoff_runtime();
        let store = FsSessionStore::new(root.join("session-store"));
        let state_db = SqliteStateStore::new(PathBuf::new());
        let state = &mut SessionState::new("status-turn", None, "test-model");
        let session = &mut SessionRuntime::acquire(
            &runtime,
            Some(&root),
            true,
            None,
            "status-turn",
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the status-turn session acquires");
        let mut ctx = TurnContext {
            state: &mut *state,
            runtime: Arc::new(runtime),
            store: &store,
            state_db: &state_db,
            format: RenderFormat::Text,
            terminal: false,
            session: &mut *session,
            trusted_echo: None,
            last_turn: None,
            save_refused: false,
            launch_options: GlobalOptions::default(),
            chat: None,
            prepared: None,
        };
        let outcome = handle_line_verbatim("/status", &mut ctx).expect("the turn runs");
        assert!(
            matches!(outcome, TurnOutcome::Completed),
            "a read completes: {outcome:?}"
        );
        assert_eq!(ctx.state.id, "status-turn", "the panel moves nothing");
        assert_eq!(ctx.state.model, "test-model");
        let text = crate::chat::ops_status::panel_text(
            ctx.state,
            &ctx.runtime,
            ctx.store,
            ctx.session.root(),
        );
        let headings: Vec<&str> = text
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with(' '))
            .collect();
        assert_eq!(
            headings,
            vec!["Session", "Model", "Workspace", "Instructions", "Limits"],
            "one panel, five headings in order: {text}"
        );
        assert!(
            text.contains("  Id: status-turn\n"),
            "the Session section names the session: {text}"
        );
        assert!(
            text.contains("  Model: test-model\n"),
            "the Model section names the model in force: {text}"
        );
        // The row asks whether the panel names the live root, not how the path
        // is spelled: the platform may resolve the fixture's temp spelling
        // (`/var/…` for `/private/var/…` on macOS), so both sides are
        // canonicalised before they are compared.
        let canonical = |path: &std::path::Path| {
            std::fs::canonicalize(path)
                .unwrap_or_else(|error| panic!("{} resolves: {error}", path.display()))
        };
        let printed = text
            .lines()
            .find_map(|line| line.strip_prefix("  Root: "))
            .unwrap_or_else(|| panic!("the Workspace section names a root: {text}"));
        assert_eq!(
            canonical(std::path::Path::new(printed)),
            canonical(&root),
            "the Workspace section names the live root: {text}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
