// ---------------------------------------------------------------------------
// the one `[tools] allow` list, at the session surface
// ---------------------------------------------------------------------------

/// The session runtime with a stated `[tools] allow` list, so a test can ask
/// what the surface advertises under it.
fn runtime_under(words: &[&str]) -> crate::config::runtime::RuntimeConfig {
    let mut runtime = session_runtime();
    runtime.resolved.tools = saya_config::ResolvedTools::from_stated(Some(
        &words.iter().map(|word| (*word).to_string()).collect::<Vec<_>>(),
    ))
    .expect("a valid list");
    runtime
}

/// The names the session advertises under `runtime`, at its widest posture.
fn advertised_under(runtime: &crate::config::runtime::RuntimeConfig) -> Vec<String> {
    let project = worktree("tools-allow");
    let state = temp_dir("tools-allow-state");
    let universe = compose(runtime, &project, &state);
    let names = universe
        .definitions(
            AgentMode::Build,
            false,
            ApprovalPolicy::Bypass,
            true,
            true,
            false,
            false,
            &runtime.resolved.tools,
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
    names
}

/// With no list the session advertises exactly what it always did: the
/// default is the composition, unchanged. This is the pin that keeps the
/// setting from narrowing a surface nobody asked to narrow.
#[test]
fn an_unstated_list_advertises_the_whole_composition() {
    let names = advertised_under(&session_runtime());
    for tool in [
        "read",
        "grep",
        "glob",
        "list",
        "write",
        "edit",
        "checklist",
        "shell",
        "schema_discovery",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "the default surface still advertises {tool}: {names:?}"
        );
    }
}

/// A stated list keeps only its names — plus the connection-tied tools, which
/// it does not govern: their availability follows the selected connection, so
/// a narrowed session still sees the schema it is connected to.
#[test]
fn a_stated_list_narrows_the_session_surface() {
    let wide = advertised_under(&session_runtime());
    let names = advertised_under(&runtime_under(&["read", "grep"]));
    for tool in ["read", "grep"] {
        assert!(names.contains(&tool.to_string()), "{tool} survives: {names:?}");
    }
    for tool in ["write", "edit", "glob", "list", "checklist", "shell"] {
        assert!(
            !names.contains(&tool.to_string()),
            "the list removed {tool}: {names:?}"
        );
    }
    // Every connection-tied name the composition advertised is still there:
    // the list does not govern them, so it can neither add nor remove one.
    for tool in &wide {
        if saya_config::is_database_tool(tool) {
            assert!(
                names.contains(tool),
                "the connection-tied {tool} is not the list's to remove: {names:?}"
            );
        }
    }
    assert!(
        wide.iter().any(|tool| saya_config::is_database_tool(tool)),
        "the fixture must advertise at least one connection-tied tool for this to mean \
         anything: {wide:?}"
    );
}

/// The list only ever narrows. `never` withholds the shell on its own, and a
/// list that names it cannot put it back — a name never grants a tool whose
/// own condition failed.
#[test]
fn a_stated_list_never_grants_a_withheld_tool() {
    let project = worktree("tools-allow-never");
    let state = temp_dir("tools-allow-never-state");
    let runtime = runtime_under(&["shell", "write"]);
    let universe = compose(&runtime, &project, &state);
    let names: Vec<String> = universe
        .definitions(
            AgentMode::Build,
            false,
            ApprovalPolicy::Never,
            true,
            true,
            false,
            false,
            &runtime.resolved.tools,
        )
        .into_iter()
        .map(|definition| definition.name)
        .collect();
    assert!(
        !names.contains(&"shell".to_string()),
        "never withholds the shell however the list is spelled: {names:?}"
    );
    assert!(
        !names.contains(&"write".to_string()),
        "never withholds the write tools however the list is spelled: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The dispatch door follows the advertisement: a call to a name the list
/// removed is refused as unknown, not merely unadvertised.
#[tokio::test]
async fn the_session_executor_refuses_a_name_the_list_removed() {
    let project = worktree("tools-allow-exec");
    let state = temp_dir("tools-allow-exec-state");
    let runtime = runtime_under(&["read"]);
    let universe = compose(&runtime, &project, &state);
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
        Arc::clone(&database),
        &CancellationToken::new(),
        None,
        None,
        None,
        runtime.resolved.tools.clone(),
    );
    let refused = executor
        .execute("write", serde_json::json!({"path": "a.txt", "content": "x"}))
        .await
        .expect_err("write was narrowed out");
    assert!(
        format!("{refused:?}").contains("UnsupportedTool"),
        "a removed name is unknown at the door: {refused:?}"
    );
    // The database fall-through is not the list's to remove, so the name still
    // reaches the member — which refuses it for its own reason (no connection).
    let reachable = executor
        .execute("schema_discovery", serde_json::json!({}))
        .await
        .expect_err("no connection is selected in this fixture");
    assert!(
        !format!("{reachable:?}").contains("UnsupportedTool"),
        "schema_discovery is not narrowed away: {reachable:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// the same list, over the MCP lane
// ---------------------------------------------------------------------------

/// A real MCP lane: the client's own shell fixture, started through the
/// client's own connect path, so what the surface advertises under a list is
/// what a user's `[mcp_servers]` table would produce.
fn mcp_lane(label: &str) -> Arc<McpTools> {
    let fixture = crate::mcp::client::support::Fixture::new(label);
    let servers = fixture.servers(&[], crate::mcp::client::support::TIMEOUTS);
    let resolver = fixture.resolver(&[]);
    McpTools::connect(
        &servers,
        &resolver,
        ApprovalPolicy::Bypass,
        None,
        &crate::agent::tools::consent::Caller::Session,
        CancellationToken::new(),
    )
    .expect("a configured server composes a lane")
}

/// The session composed with that lane, through the same composer production
/// uses. The lane is the session's, started once; each turn takes it under the
/// turn's own consent, which is not what this test is about.
fn compose_with_mcp(
    runtime: &crate::config::runtime::RuntimeConfig,
    project: &Path,
    mcp: Arc<McpTools>,
) -> SessionUniverse {
    SessionUniverse::compose_with_launch_and_path(SessionComposition {
        runtime,
        explicit: None,
        pinned_root: None,
        walk_when_unpinned: true,
        cwd: project,
        launch: None,
        path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
        recovery: None,
        mcp: Some(mcp),
    })
    .expect("composition succeeds on a plain worktree")
}

/// `[tools] allow` governs MCP tools exactly as it governs every other name:
/// the server's whole entry keeps every tool of it, an exact
/// `mcp__<server>__<tool>` keeps that one and no other, and a list that names
/// neither takes the family away. The names come from a third-party server, so
/// they are governed by shape rather than by a fixed list — but they are
/// governed, and the drift test in `agent::tools` is what says so.
#[test]
fn a_stated_list_governs_mcp_names() {
    let lane = mcp_lane("tools-allow-mcp");
    let names = |words: &[&str]| -> Vec<String> {
        let runtime = runtime_under(words);
        let project = worktree("tools-allow-mcp");
        let state = temp_dir("tools-allow-mcp-state");
        let universe = compose_with_mcp(&runtime, &project, Arc::clone(&lane));
        let names: Vec<String> = universe
            .definitions(
                AgentMode::Build,
                false,
                ApprovalPolicy::Bypass,
                true,
                true,
                false,
                false,
                &runtime.resolved.tools,
            )
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
        names
    };
    let whole = names(&["mcp__fixture", "read"]);
    assert!(
        whole.contains(&"mcp__fixture__echo".to_string())
            && whole.contains(&"mcp__fixture__secret".to_string()),
        "the server's whole entry keeps every tool of it: {whole:?}"
    );
    let exact = names(&["mcp__fixture__echo", "read"]);
    assert!(
        exact.contains(&"mcp__fixture__echo".to_string()),
        "the exact name survives: {exact:?}"
    );
    assert!(
        !exact.contains(&"mcp__fixture__fail".to_string()),
        "and its siblings do not: {exact:?}"
    );
    let none = names(&["read"]);
    assert!(
        !none.iter().any(|name| name.starts_with("mcp__")),
        "a list that names no server keeps no MCP tool: {none:?}"
    );
}

/// The dispatch door follows the advertisement here too: a call to an MCP name
/// the list removed is refused as unknown, and one the list kept reaches the
/// lane. Without the door check the advertisement would be the only thing
/// standing between a narrowed session and a third-party server.
#[tokio::test]
async fn the_session_executor_refuses_an_mcp_name_the_list_removed() {
    let lane = mcp_lane("tools-allow-mcp-exec");
    let project = worktree("tools-allow-mcp-exec");
    let state = temp_dir("tools-allow-mcp-exec-state");
    let runtime = runtime_under(&["read"]);
    let universe = compose_with_mcp(&runtime, &project, lane);
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(
        Arc::clone(&database),
        &CancellationToken::new(),
        None,
        None,
        None,
        runtime.resolved.tools.clone(),
    );
    let refused = executor
        .execute("mcp__fixture__echo", serde_json::json!({"text": "hi"}))
        .await
        .expect_err("the list removed every MCP name");
    assert!(
        format!("{refused:?}").contains("UnsupportedTool"),
        "a removed MCP name is unknown at the door: {refused:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
