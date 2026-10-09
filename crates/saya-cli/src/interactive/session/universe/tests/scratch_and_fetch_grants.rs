/// The headless-ask shape for the scratch family: a held `scratch` grant
/// advertises `scratch_sql` — and `scratch_import` exactly where a
/// workspace root binds, the same composition facts `grant_token` reads
/// before offering the token. No other family moves: one token opens one
/// family, and the ungrantable effects (`render_chart`, the fan-out)
/// stay on the approval-surface rule alone.
#[test]
fn a_headless_ask_session_advertises_the_scratch_family_under_its_grant() {
    let project = worktree("grant-scratch");
    let state = temp_dir("grant-scratch-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["scratch"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["scratch_sql", "scratch_import"] {
        assert!(
            names.contains(&tool.to_string()),
            "the held scratch grant advertises {tool} under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "http_fetch",
        "http_download",
        "run_command",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a scratch grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The ungrantable fan-out is a read-surface tool: its advertisement is
    // unchanged by the grant — present exactly where `allow_query_data` put
    // it, widened by nothing.
    assert!(
        names.contains(&"bounded_sql_query_all".to_string()),
        "the fan-out rides the read surface as today, untouched by the grant: {names:?}"
    );
    // The member-composed half: no workspace root, no contained import —
    // scratch_sql still rides the grant, scratch_import does not.
    let plain = temp_dir("grant-scratch-plain");
    let plain_state = temp_dir("grant-scratch-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["scratch"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "scratch needs no root: {names:?}"
    );
    assert!(
        !names.contains(&"scratch_import".to_string()),
        "no root, no contained import, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}

/// The fetch family: a held `fetch:` token advertises `http_fetch` — and
/// `http_download` exactly where a workspace root binds (the download needs
/// a destination, the same condition `grant_token` mirrors). The chart and
/// the fan-out stay hidden: no grant token exists for them, and a fetch
/// grant must never stand in for the approval surface they ride.
#[test]
fn a_headless_ask_session_advertises_the_fetch_family_under_its_grant() {
    let project = worktree("grant-fetch");
    let state = temp_dir("grant-fetch-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["fetch:https+docs.example"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["http_fetch", "http_download"] {
        assert!(
            names.contains(&tool.to_string()),
            "the held fetch grant advertises {tool} under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "run_command",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a fetch grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // No root, no download destination — fetch alone rides the grant.
    let plain = temp_dir("grant-fetch-plain");
    let plain_state = temp_dir("grant-fetch-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["fetch:https+docs.example"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"http_fetch".to_string()),
        "fetch needs no root: {names:?}"
    );
    assert!(
        !names.contains(&"http_download".to_string()),
        "no root, no download destination, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}
