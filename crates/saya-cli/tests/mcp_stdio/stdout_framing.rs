/// The full happy path with data sharing on: initialize negotiates, every
/// advertised tool answers, every stdout line is a JSON-RPC frame, an
/// oversized line gets -32600 without taking the server down, and stdin EOF
/// exits 0.
#[test]
fn mcp_stdout_contains_only_protocol_frames() {
    let fixture = crate_fixture("frames");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &[
            "mcp",
            "serve",
            "--profile",
            "local",
            "--profile",
            "other",
            "--allow-data-sharing",
        ],
    );

    server.handshake();

    // tools/list → the shared toolset, all described and schematized.
    server.send(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
    let tools = server.next_json(10);
    assert_eq!(tools["id"], 2);
    let list = tools["result"]["tools"].as_array().expect("tools array");
    let names: Vec<_> = list
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        names,
        vec![
            "contracts",
            "investigation_run",
            "list_profiles",
            "query",
            "schema"
        ],
        "the shared toolset is advertised in the documented order"
    );
    for tool in list {
        assert!(
            tool["description"].as_str().is_some_and(|d| !d.is_empty()),
            "every tool describes itself: {tool}"
        );
        assert!(tool["inputSchema"].is_object(), "every tool has a schema");
    }
    let replay_tool = list
        .iter()
        .find(|tool| tool["name"] == "investigation_run")
        .expect("the replay tool is advertised");
    assert_eq!(
        replay_tool["inputSchema"]["properties"]["params"]["type"], "object",
        "the replay tool advertises its params map: {replay_tool}"
    );
    assert_eq!(
        replay_tool["inputSchema"]["properties"]["params"]["additionalProperties"]["type"],
        "string",
        "params values are strings, parsed by the typed parser: {replay_tool}"
    );

    // list_profiles → names and dialects only.
    let profiles = server.call(3, "list_profiles", "{}");
    assert_eq!(profiles["result"]["isError"], false);
    assert_eq!(
        profiles["result"]["structuredContent"]["profiles"],
        serde_json::json!([
            {"name": "local", "dialect": "sqlite"},
            {"name": "other", "dialect": "sqlite"}
        ]),
    );

    // schema → the compact tree for the allowed profile.
    let schema = server.call(4, "schema", r#"{"profile":"local"}"#);
    assert_eq!(schema["result"]["isError"], false);
    assert_eq!(schema["result"]["structuredContent"]["profile"], "local");
    let tables = schema["result"]["structuredContent"]["tables"]
        .as_object()
        .expect("compact schema tables");
    assert!(
        tables.contains_key("events"),
        "the seeded table is in the compact tree: {tables:?}"
    );

    // query → bounded read-only rows with mcp-sourced evidence.
    let query = server.call(
        5,
        "query",
        r#"{"profile":"local","sql":"SELECT id, label FROM events ORDER BY id"}"#,
    );
    assert_eq!(query["result"]["isError"], false);
    let payload = &query["result"]["structuredContent"];
    assert_eq!(payload["columns"], serde_json::json!(["id", "label"]));
    assert_eq!(payload["row_count"], 2);
    assert_eq!(payload["truncated"], false);
    assert!(
        payload["source"].is_null(),
        "the source is named by the typed evidence, not an ad-hoc marker: {payload}"
    );
    assert_eq!(
        payload["evidence"]["source"]["kind"], "mcp",
        "the query evidence names this server as its source: {payload}"
    );
    assert!(
        payload["evidence"]["execution_id"].is_string(),
        "the result carries execution evidence: {payload}"
    );
    let rendered = serde_json::to_string(&query).unwrap();
    assert!(
        !rendered.contains("data.sqlite3"),
        "the sqlite path never reaches the client: {rendered}"
    );

    // contracts → the active claim (no identities). The schema cache is
    // refreshed first, then one confirmed claim is remembered through the
    // CLI; both tool forms answer with it.
    let (code, out, err) = fixture.cli(&["connection", "schema", "local", "--refresh"]);
    assert_eq!(code, 0, "schema refresh failed: {out}{err}");
    let (code, out, err) = fixture.cli(&[
        "contracts",
        "remember",
        "data.main.events",
        "--kind",
        "description",
        "--value",
        "orders header table",
        "--profile",
        "local",
    ]);
    assert_eq!(code, 0, "remember failed: {out}{err}");
    let contracts = server.call(6, "contracts", r#"{"profile":"local"}"#);
    assert_eq!(contracts["result"]["isError"], false);
    let listed = contracts["result"]["structuredContent"]["contracts"]
        .as_array()
        .expect("contracts array");
    assert_eq!(listed.len(), 1, "the confirmed claim is listed: {listed:?}");
    assert_eq!(listed[0]["object"], "data.main.events");
    assert_eq!(listed[0]["claims"][0]["status"], "confirmed");
    let one = server.call(
        7,
        "contracts",
        r#"{"profile":"local","table":"data.main.events"}"#,
    );
    assert_eq!(one["result"]["isError"], false);
    assert_eq!(
        one["result"]["structuredContent"]["claims"][0]["value"],
        "orders header table"
    );
    let rendered = serde_json::to_string(&one).unwrap();
    assert!(
        !rendered.contains("p-") && !rendered.contains("identity"),
        "the opaque profile identity never reaches the client: {rendered}"
    );

    // An unknown tool is a JSON-RPC method error, not a crash.
    server.send(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"no_such_tool","arguments":{}}}"#,
    );
    let unknown = server.next_json(10);
    assert_eq!(unknown["id"], 9);
    assert_eq!(
        unknown["error"]["code"], -32601,
        "unknown tool: method not found"
    );

    // An oversized line is answered with -32600 and the server keeps serving.
    let oversized = format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"mcp/padded","params":{{"pad":"{}"}}}}"#,
        "A".repeat(1024 * 1024 + 64),
    );
    assert!(oversized.len() > 1024 * 1024);
    server.send(&oversized);
    let rejected = server.next_json(10);
    assert_eq!(
        rejected["error"]["code"], -32600,
        "oversized line: {rejected}"
    );
    assert_eq!(rejected["error"]["message"], "request too large");
    assert!(
        rejected["id"].is_null(),
        "a discarded line is never parsed, so its id cannot be echoed: {rejected}"
    );

    // The server is still alive after the oversized line.
    server.send(r#"{"jsonrpc":"2.0","id":6,"method":"ping"}"#);
    let ping = server.next_json(5);
    assert_eq!(ping["id"], 6);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}
