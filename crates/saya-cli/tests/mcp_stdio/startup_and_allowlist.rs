/// Without `--profile`, the allowlist is the configured default profile.
#[test]
fn mcp_stdio_list_profiles_uses_the_configured_default() {
    let fixture = crate_fixture("default");
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve"]);

    server.handshake();
    let call = server.call(5, "list_profiles", "{}");
    assert_eq!(
        call["result"]["structuredContent"]["profiles"],
        serde_json::json!([{"name": "local", "dialect": "sqlite"}]),
        "the configured default profile forms the allowlist"
    );

    server.close_and_expect_exit(0);
}

/// A `--profile` naming nothing in the resolved connections is refused at
/// startup with a usage error, before any protocol byte is written.
#[test]
fn mcp_stdio_unknown_profile_is_refused_at_startup() {
    let fixture = crate_fixture("unknown");
    let mut server = TestServer::spawn(&fixture, &["mcp", "serve", "--profile", "missing"]);

    let code = server.wait_for_exit();
    assert_eq!(code, 2, "unknown profile is a usage error");
    if let Ok(line) = server.responses.recv_timeout(Duration::from_millis(300)) {
        panic!("no protocol bytes on a refused startup, got: {line}");
    }
    let stderr = server.stderr();
    assert!(
        stderr.contains("missing"),
        "the refusal names the profile: {stderr}"
    );
}

/// The client cannot widen the startup allowlist: a configured profile that
/// was not allowlisted is "profile not available" on every tool, and
/// `list_profiles` never reveals it.
#[test]
fn mcp_client_cannot_expand_profile_allowlist() {
    let fixture = crate_fixture("allowlist");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let profiles = server.call(2, "list_profiles", "{}");
    assert_eq!(
        profiles["result"]["structuredContent"]["profiles"],
        serde_json::json!([{"name": "local", "dialect": "sqlite"}]),
        "only the allowlisted profile is listed, though 'other' is configured"
    );

    for (id, name, arguments) in [
        (3, "schema", r#"{"profile":"other"}"#),
        (4, "query", r#"{"profile":"other","sql":"SELECT 1"}"#),
        (5, "contracts", r#"{"profile":"other"}"#),
    ] {
        let text = server.call_error_text(id, name, arguments);
        assert!(
            text.contains("profile not available"),
            "{name} refuses a non-allowlisted profile: {text}"
        );
    }

    server.close_and_expect_exit(0);
}
