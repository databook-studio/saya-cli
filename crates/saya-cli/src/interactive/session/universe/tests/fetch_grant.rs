/// The fetch family: a held `fetch:` token advertises `http_fetch` — and
/// `http_download` exactly where a workspace root binds (the download needs
/// a destination, the same condition `grant_token` mirrors). The chart and
/// the fan-out stay hidden: no grant token exists for them, and a fetch
/// grant must never stand in for the approval surface they ride.
#[test]
fn a_headless_ask_session_advertises_the_fetch_family_under_its_grant() {
    let project = worktree("grant-fetch");
    let state = temp_dir("grant-fetch-state");
    let universe = compose(&session_runtime(), &project, &state);
    granted(&universe, &["fetch:https+docs.example"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["http_fetch", "http_download"] {
        assert!(
            names.contains(&tool.to_string()),
            "the held fetch grant advertises {tool} under headless ask: {names:?}"
        );
    }
    for tool in [
        "write",
        "edit",
        "run_command",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a fetch grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // No root, no download destination — fetch alone rides the grant.
    let plain = std::env::temp_dir();
    let plain_state = temp_dir("grant-fetch-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(),
        None,
        None,
        true,
        &plain)
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
        fs::remove_dir_all(&plain_state),
    );
}
