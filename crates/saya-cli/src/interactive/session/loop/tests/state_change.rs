#[cfg(test)]
mod state_change_tests {
    use super::*;
    use crate::commands::coding::parent_turn_provider::Scripted;
    use saya_agent::ApprovalPolicy;
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    /// The model the session opens on, and the one the runtime resolves: a
    /// `/model` change moves the first and must move the request with it.
    const MODEL: &str = "line-model";

    /// The profile the fixture's database is named, and the profile its runtime
    /// states as the configured default: two profiles, so `/connect` has one to
    /// move to.
    const DATABASE: &str = "demo";
    const DEFAULT_PROFILE: &str = "other";

    /// The request bodies the lines produced, in order: a real run over a
    /// throwaway folder, composed and opened the way the plain loop opens one,
    /// and driven through the per-turn entry the piped loop and `--turn-file`
    /// run (`handle_line_verbatim`).
    fn drive(label: &str, profile: Option<&str>, lines: &[&str]) -> Vec<String> {
        let provider = Scripted::start();
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target")
            .join(format!("tmp-state-change-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("the fixture root creates");
        let root = root.canonicalize().expect("the fixture root resolves");
        let folder = root.join("repo");
        std::fs::create_dir(&folder).expect("the fixture folder creates");
        let config_home = root.join("config");
        std::fs::create_dir_all(&config_home).expect("the fixture config directory creates");
        let database = block_on(database(&root));
        let connections = config_home.join("connections.toml");
        let entry = |name: &str| {
            format!(
                "[profiles.{name}]\ntype = 'sqlite'\npath = '{}'\n",
                database.display()
            )
        };
        std::fs::write(
            &connections,
            format!("{}{}", entry(DATABASE), entry(DEFAULT_PROFILE)),
        )
        .expect("the fixture connections file writes");
        let options = GlobalOptions {
            connections: Some(connections),
            profile: Some(DEFAULT_PROFILE.to_owned()),
            ..GlobalOptions::default()
        };
        let mut runtime =
            crate::config::runtime::load_with_sources(&options, &folder, &config_home, BTreeMap::new())
                .expect("the fixture resolves a runtime");
        // The scripted endpoint is the one provider the session talks to, and
        // the one its state names, so a `/model` change is the only difference
        // between the two requests.
        runtime.resolved.ai.provider = saya_config::AiProvider::OpenaiCompatible;
        runtime.resolved.ai.model = MODEL.into();
        runtime.resolved.ai.base_url = Some(provider.endpoint.clone());
        runtime.resolved.ai.api_key = None;
        runtime.resolved.ai.retry_delays_ms.clear();
        runtime.resolved.ai.timeout_seconds = 10;
        runtime.resolved.ai.idle_timeout_seconds = 10;
        runtime.resolved.ai.max_output_tokens = Some(4096);
        runtime.resolved.ai.context_window_tokens = Some(128_000);
        let effective = runtime
            .resolved
            .endpoints
            .get_mut(saya_config::ORCHESTRATOR_ROLE)
            .expect("the runtime resolves an orchestrator endpoint");
        effective.provider = saya_config::AiProvider::OpenaiCompatible;
        effective.base_url = Some(provider.endpoint.clone());
        effective.api_key = None;
        let runtime = Arc::new(runtime);
        let store = FsSessionStore::new(root.join("session-store"));
        let state_db = SqliteStateStore::new(root.join("state.sqlite3"));
        let id = format!("state-change-{label}");
        let session = &mut SessionRuntime::acquire(
            &runtime,
            Some(&folder),
            true,
            None,
            &id,
            ApprovalPolicy::Ask,
            &root.join("session-dirs"),
        )
        .expect("the fixture's session acquires");
        let state = &mut SessionState::new(&id, profile.map(str::to_owned), MODEL);
        state.provider = "openai_compatible".to_owned();
        state.provider_endpoint = Some(provider.endpoint.clone());
        state.provider_endpoint_bound = true;
        let mut ctx = TurnContext {
            state,
            runtime: Arc::clone(&runtime),
            store: &store,
            state_db: &state_db,
            format: RenderFormat::Text,
            terminal: false,
            session,
            trusted_echo: None,
            last_turn: None,
            save_refused: false,
            launch_options: options,
            chat: None,
            prepared: None,
        };
        for line in lines {
            let outcome = handle_line_verbatim(line, &mut ctx).expect("the turn runs");
            assert!(
                matches!(outcome, TurnOutcome::Completed),
                "`{line}` completed: {outcome:?}"
            );
        }
        let bodies = provider.bodies();
        // The run holds the folder's lease: dropping the session's chat gives it
        // up, so the throwaway tree goes with everything closed.
        drop(ctx);
        let _ = std::fs::remove_dir_all(&root);
        bodies
    }

    /// The throwaway database the fixture's profiles point at: a real, empty
    /// SQLite file, so a profile opens rather than being skipped.
    async fn database(root: &Path) -> PathBuf {
        let path = root.join("data.sqlite3");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let pool = sqlx::SqlitePool::connect_with(options)
            .await
            .expect("the fixture database opens");
        sqlx::query("CREATE TABLE fixture (id INTEGER)")
            .execute(&pool)
            .await
            .expect("the fixture schema writes");
        pool.close().await;
        path
    }

    /// `/model X` after the run has taken a turn: the next request is a turn of
    /// a run composed from the state the change stated — the new model — and it
    /// still carries the conversation so far.
    #[test]
    fn a_model_change_after_the_first_turn_reaches_the_next_request() {
        let bodies = drive(
            "model",
            None,
            &["first message", "/model chosen-model", "second message"],
        );
        let second = bodies.last().expect("the second turn's request arrived");
        assert!(
            second.contains("\"model\":\"chosen-model\"")
                || second.contains("\"model\": \"chosen-model\""),
            "the request after the change names the model the session states: {second}"
        );
        assert!(
            second.contains("first message") && second.contains("turn 0 answered"),
            "the conversation so far is replayed into the run after the change: {second}"
        );
    }

    /// `/connect` after the run has taken a turn: the next request is a turn of
    /// a run composed over the profile the command named, and it still carries
    /// the conversation so far.
    #[test]
    fn a_connect_after_the_first_turn_reaches_the_next_request() {
        let connect = format!("/connect {DATABASE}");
        let bodies = drive(
            "connect",
            None,
            &["first message", connect.as_str(), "second message"],
        );
        let first = bodies.first().expect("the first turn's request arrived");
        assert!(
            first.contains("other (sqlite)") && !first.contains(&format!("{DATABASE} (sqlite)")),
            "the run opened on the configured default, not the profile the command names: {first}"
        );
        let second = bodies.last().expect("the second turn's request arrived");
        assert!(
            second.contains("use_database")
                && second.contains(&format!("{DATABASE} (sqlite)"))
                && !second.contains("other (sqlite)"),
            "the request after the change targets the profile it named: {second}"
        );
        assert!(
            second.contains("first message"),
            "the conversation so far is replayed into the run after the change: {second}"
        );
    }

    /// `/privacy on` after the run has taken a turn: the next request is a turn
    /// of a run that may send row values, so the query tools appear — and it
    /// still carries the conversation so far.
    #[test]
    fn a_privacy_change_after_the_first_turn_reaches_the_next_request() {
        let bodies = drive(
            "privacy",
            Some(DATABASE),
            &[
                "query the demo database",
                "/privacy on",
                "query the demo database again",
            ],
        );
        let first = bodies.first().expect("the first turn's request arrived");
        assert!(
            first.contains("schema_discovery"),
            "database work advertises the schema tools: {first}"
        );
        assert!(
            !first.contains("bounded_sql_query"),
            "sharing off advertises no query tool: {first}"
        );
        let second = bodies.last().expect("the second turn's request arrived");
        assert!(
            second.contains("bounded_sql_query"),
            "the request after the change may send row values: {second}"
        );
        assert!(
            second.contains("query the demo database"),
            "the conversation so far is replayed into the run after the change: {second}"
        );
    }
}
