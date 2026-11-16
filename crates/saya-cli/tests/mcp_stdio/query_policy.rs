/// `query` rides the same safety gate the CLI uses: a DELETE is an isError
/// with the read-only message, and a SELECT returns rows.
#[test]
fn mcp_query_uses_same_safety_path() {
    let fixture = crate_fixture("safety");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let text = server.call_error_text(
        2,
        "query",
        r#"{"profile":"local","sql":"DELETE FROM events"}"#,
    );
    assert!(
        text.contains("read-only safety policy") && text.contains("DELETE"),
        "the refusal is the safety layer's own words: {text}"
    );

    let query = server.call(
        3,
        "query",
        r#"{"profile":"local","sql":"SELECT count(*) AS n FROM events"}"#,
    );
    assert_eq!(query["result"]["isError"], false);
    assert_eq!(query["result"]["structuredContent"]["row_count"], 1);

    // The written rows are untouched: the gate runs before execution.
    let (code, out, err) = fixture.cli(&["query", "--sql", "SELECT count(*) AS n FROM events"]);
    assert_eq!(
        code, 0,
        "the database is intact after the refused delete: {out}{err}"
    );

    server.close_and_expect_exit(0);
}

/// Data sharing off (the CLI's own fold): row-returning tools are not listed
/// and are refused if called anyway; config-on and `--no-data-sharing` fold
/// exactly as the CLI folds them.
#[test]
fn mcp_denial_matches_cli_policy() {
    // (a) default: sharing off — row tools absent and refused.
    let fixture = crate_fixture("denial-off");
    fixture.seed_events();
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve", "--profile", "local"]);
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        !listed.contains(&"query".to_string())
            && !listed.contains(&"investigation_run".to_string()),
        "row tools are not listed with sharing off: {listed:?}"
    );
    assert!(listed.contains(&"schema".to_string()));
    assert!(listed.contains(&"contracts".to_string()));
    let text = server.call_error_text(3, "query", r#"{"profile":"local","sql":"SELECT 1"}"#);
    assert!(
        text.contains("data sharing"),
        "the refusal names the gate: {text}"
    );
    let replay =
        server.call_error_text(4, "investigation_run", r#"{"id":"no-such-investigation"}"#);
    assert!(
        replay.contains("data sharing"),
        "investigation_run is refused the same way: {replay}"
    );
    server.close_and_expect_exit(0);

    // (b) the user config allows — row tools are listed. The setting lives in
    // the user layer (the trusted one): an explicit `--config` file loads as
    // the project layer, where security-critical settings are reverted.
    let scenario = crate_fixture("denial-config-on");
    let user_config = scenario.root.join("home/.config/saya/config.toml");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "[ai]\nallow_data_sharing = true\n").unwrap();
    let mut server = TestServer::spawn(&scenario, &["mcp", "serve", "--profile", "local"]);
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        listed.contains(&"query".to_string()) && listed.contains(&"investigation_run".to_string()),
        "config-on lists the row tools: {listed:?}"
    );
    server.close_and_expect_exit(0);

    // (c) the user config allows but `--no-data-sharing` overrides — off
    // again, the same fold the CLI applies.
    let scenario = crate_fixture("denial-cli-override");
    let user_config = scenario.root.join("home/.config/saya/config.toml");
    std::fs::create_dir_all(user_config.parent().unwrap()).unwrap();
    std::fs::write(&user_config, "[ai]\nallow_data_sharing = true\n").unwrap();
    let mut server = TestServer::spawn(
        &scenario,
        &["mcp", "serve", "--profile", "local", "--no-data-sharing"],
    );
    server.handshake();
    let listed = server.listed_tools(2);
    assert!(
        !listed.contains(&"query".to_string()),
        "--no-data-sharing overrides config: {listed:?}"
    );
    server.close_and_expect_exit(0);
}
