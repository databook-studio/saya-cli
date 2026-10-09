/// Concurrency and cancellation are bounded: the fifth of five concurrent
/// slow calls is refused as busy, a cancelled in-flight call is never
/// answered, and the server keeps serving after both.
#[test]
fn mcp_cancellation_and_concurrency_are_bounded() {
    let fixture = crate_fixture("bounds");
    fixture.seed_events();
    let mut config = std::fs::read_to_string(&fixture.config).unwrap();
    config.push_str("\n[run]\nquery_timeout_seconds = 4\n");
    std::fs::write(&fixture.config, config).unwrap();
    let slow_replay_id = fixture.save_investigation("slow replay", SLOW_SQL, "local");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    // Five concurrent slow calls: exactly one is refused as busy, the rest
    // run (and each ends within the connector's own query timeout).
    for id in 10..=14 {
        server.send(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"tools/call","params":{{"name":"query","arguments":{{"profile":"local","sql":"{SLOW_SQL}"}}}}}}"#
        ));
    }
    let mut busy = 0;
    for _ in 0..4 {
        let response = server.next_json(60);
        assert!(
            response["error"].is_null() && response["result"]["isError"] == true,
            "every concurrent outcome is a tool-level result: {response}"
        );
        let text = response["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if text.contains("in-flight") {
            busy += 1;
        }
    }
    assert_eq!(
        busy, 1,
        "exactly one of five concurrent calls is refused busy"
    );

    // A cancelled in-flight call gets no response.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":20,"method":"tools/call","params":{{"name":"query","arguments":{{"profile":"local","sql":"{SLOW_SQL}"}}}}}}"#
    ));
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":20,"reason":"client moved on"}}"#);
    server.expect_no_response_for(20, 2500);

    // Exercise the separate saved-replay route over the real RMCP transport:
    // the cancelled request id stays suppressed and the server remains live.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":23,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{slow_replay_id}"}}}}}}"#
    ));
    server.send(r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":23,"reason":"client moved on"}}"#);
    server.expect_no_response_for(23, 2500);

    // Cancellation does not kill the server; another database request works.
    server.send(r#"{"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"query","arguments":{"profile":"local","sql":"SELECT 1 AS still_running"}}}"#);
    let query = server.next_json(10);
    assert_eq!(query["id"], 21);
    assert_eq!(query["result"]["isError"], false, "{query}");
    assert_eq!(query["result"]["structuredContent"]["row_count"], 1);
    assert_eq!(
        query["result"]["structuredContent"]["columns"],
        serde_json::json!(["still_running"])
    );

    // The server also answers protocol traffic after cancellation.
    server.send(r#"{"jsonrpc":"2.0","id":22,"method":"ping"}"#);
    let ping = server.next_json(10);
    assert_eq!(ping["id"], 22);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}
