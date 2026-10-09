/// Composes a session over a git-worktree fixture and returns the pieces a
/// preference turn needs. The worktree id derives from the session's bound
/// canonical root — the identity the store must agree with.
async fn compose_fixture(
    tag: &str,
    context_byte_budget: usize,
    privacy_gate_closed: bool,
) -> (
    RuntimeConfig,
    Arc<SessionUniverse>,
    PathBuf,
    PathBuf,
    WorktreeId,
) {
    let root = stamp(tag);
    let state_dir = root.join("state");
    let project = root.join("project");
    write_file(&project.join(".git").join("HEAD"), "ref: refs/heads/main\n");
    write_file(&project.join("AGENTS.md"), "project guidance text");
    let runtime = runtime_config(context_byte_budget, privacy_gate_closed);
    std::fs::create_dir_all(&state_dir).expect("create the session state directory");
    let session = Arc::new(
        SessionUniverse::compose(&runtime, None, None, true, &project, &state_dir)
            .expect("the session universe composes"),
    );
    let worktree = worktree_id_for(session.root().expect("the session binds a root"))
        .expect("the canonical root derives an id");
    (runtime, session, root, state_dir, worktree)
}

async fn run_turn(
    runtime: &RuntimeConfig,
    session: Arc<SessionUniverse>,
    privacy_gate_closed: bool,
    store: SqliteStateStore,
) -> (
    Result<AgentOutput, crate::agent::runtime::AgentRuntimeError>,
    Arc<Captured>,
) {
    let captured = Arc::new(Captured::default());
    let inputs = TurnInputs {
        ai: ai_for(runtime.resolved.ai.context_byte_budget, privacy_gate_closed),
        provider: Box::new(RequestRecorder {
            captured: Arc::clone(&captured),
        }),
        registry: registry_for("analytics"),
        failures: Vec::new(),
    };
    let result = run_prompt_with_inputs(
        runtime,
        inputs,
        "orders by month",
        ApprovalPolicy::ReadOnly,
        false,
        false,
        Vec::new(),
        &NoopEventSink,
        CancellationToken::new(),
        Some(store),
        None,
        None,
        Some(session),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    (result, captured)
}

#[tokio::test]
async fn preferences_ride_after_the_instructions_and_never_in_the_system_prompt() {
    let budget = 64 * 1024;
    let (runtime, session, root, state_dir, worktree) =
        compose_fixture("wiring", budget, false).await;
    let store = SqliteStateStore::new(state_dir.join("state.sqlite3"));
    seed(&store, &worktree, "prefer tabs over spaces", now_ms()).await;
    seed(
        &store,
        &worktree,
        "always qualify table names",
        now_ms() + 1,
    )
    .await;
    let (result, captured) = run_turn(&runtime, Arc::clone(&session), false, store).await;
    result.expect("the turn completes");
    assert_eq!(
        captured.request_count(),
        1,
        "one answering request, no tools"
    );
    let messages = captured.first_messages().expect("the request was recorded");
    let system = &messages[0];
    let user = messages.last().unwrap();
    assert_eq!(system.role, "system");
    assert_eq!(user.role, "user");
    assert!(
        !system.content.contains("prefer tabs over spaces"),
        "preference text must never reach the system message: {}",
        system.content
    );
    assert!(
        !system.content.contains("project-preferences"),
        "no preference label in the system message"
    );
    assert_eq!(
        user.content.matches("project-preferences").count(),
        1,
        "exactly one labelled block: {}",
        user.content
    );
    let preferences_at = user.content.find("project-preferences").unwrap();
    let instructions_at = user
        .content
        .find("project-instructions (AGENTS.md)")
        .expect("the AGENTS.md block is labelled");
    let continuation_at = user
        .content
        .find("application-continuation")
        .expect("the session continuation block rides the same turn");
    assert!(
        instructions_at < preferences_at && preferences_at < continuation_at,
        "instructions first, then preferences, then the volatile hints"
    );
    let segment = &user.content[preferences_at..continuation_at];
    assert!(
        segment.contains(UNTRUSTED_PREFIX),
        "the same untrusted framing: {segment}"
    );
    assert!(
        segment.contains(&format!("\n{HEADER}")),
        "the header: {segment}"
    );
    let first = segment
        .find("- prefer tabs over spaces")
        .expect("first item");
    let second = segment
        .find("- always qualify table names")
        .expect("second item");
    assert!(first < second, "oldest first: {segment}");
    let request_bytes: usize = messages
        .iter()
        .map(|message| message.role.len() + message.content.len())
        .sum();
    assert!(
        request_bytes <= budget * 3 / 4,
        "the turn still reserves a quarter of the budget: {request_bytes} bytes"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn preferences_from_another_worktree_do_not_ride() {
    let (runtime, session, root, state_dir, _) =
        compose_fixture("elsewhere", 64 * 1024, false).await;
    let store = SqliteStateStore::new(state_dir.join("state.sqlite3"));
    let elsewhere_id = format!(
        "/saya-prefs-elsewhere-{}",
        root.file_name().unwrap().to_string_lossy()
    );
    let elsewhere =
        WorktreeId::from_canonical_path(&elsewhere_id).expect("the synthetic worktree shape");
    seed(
        &store,
        &elsewhere,
        "ELSEWHERE-PREF-SENTINEL keep index hints",
        now_ms(),
    )
    .await;
    let (result, captured) = run_turn(&runtime, Arc::clone(&session), false, store).await;
    result.expect("the turn completes");
    let messages = captured.first_messages().expect("the request was recorded");
    for message in &messages {
        assert!(
            !message.content.contains("ELSEWHERE-PREF-SENTINEL")
                && !message.content.contains("project-preferences"),
            "another worktree's preferences never ride: {}",
            message.content
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn forget_is_visible_on_the_next_request() {
    let (runtime, session, root, state_dir, worktree) =
        compose_fixture("forget", 64 * 1024, false).await;
    let path = state_dir.join("state.sqlite3");
    let store = SqliteStateStore::new(&path);
    seed(&store, &worktree, "prefer tabs over spaces", now_ms()).await;
    let (result, captured) = run_turn(&runtime, Arc::clone(&session), false, store).await;
    result.expect("the first turn completes");
    let user = captured.first_messages().unwrap().last().unwrap().clone();
    assert!(
        user.content.contains("project-preferences"),
        "the first turn carries the block"
    );
    // Forget — the very next request rebuilds its blocks, so the block is
    // gone without any cache to clear.
    let store = SqliteStateStore::new(&path);
    let forgotten = store
        .forget_all_project_preferences(&worktree)
        .await
        .expect("the forget lands");
    assert_eq!(forgotten, 1);
    let (result, captured) = run_turn(&runtime, session, false, store).await;
    result.expect("the second turn completes");
    let user = captured.first_messages().unwrap().last().unwrap().clone();
    assert!(
        !user.content.contains("project-preferences"),
        "no block after forget: {}",
        user.content
    );
    assert!(
        !user.content.contains("prefer tabs over spaces"),
        "no preference text after forget"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[tokio::test]
async fn preferences_bypass_the_database_privacy_gate() {
    // The frozen property: preferences are user-authored guidance, not
    // database data. Flipped flag: `ResolvedAi.allow_data_sharing = false`
    // with a non-local provider, so the turn's `allow_query_data` is false —
    // recall blocks and memory close, the preferences block does not.
    let budget = 64 * 1024;
    let (runtime, session, root, state_dir, worktree) =
        compose_fixture("privacy", budget, true).await;
    let store = SqliteStateStore::new(state_dir.join("state.sqlite3"));
    seed(&store, &worktree, "prefer tabs over spaces", now_ms()).await;
    let (result, captured) = run_turn(&runtime, Arc::clone(&session), true, store).await;
    result.expect("the turn completes");
    let user = captured.first_messages().unwrap().last().unwrap().clone();
    assert!(
        user.content.contains("project-preferences")
            && user.content.contains("- prefer tabs over spaces"),
        "the block rides even with the database privacy gate closed: {}",
        user.content
    );
    let _ = std::fs::remove_dir_all(&root);
}
