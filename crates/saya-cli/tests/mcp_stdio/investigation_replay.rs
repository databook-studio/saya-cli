/// `investigation_run` replays through the same typed run operation as
/// `saya investigation run`: a fresh review runs and returns the result and
/// evidence; a stale review is refused with the CLI's own message and is
/// never revalidated.
#[test]
fn mcp_investigation_run_happy_and_stale() {
    let fixture = crate_fixture("replay");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    // Save through the CLI: the review binding is recorded locally.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "save",
        "--name",
        "event count",
        "--sql",
        "SELECT count(*) AS n FROM events",
        "--connection",
        "local",
    ]);
    assert_eq!(code, 0, "save failed: {out}{err}");
    let id = out
        .lines()
        .next()
        .expect("the id is the first line")
        .trim()
        .to_string();

    server.handshake();

    // Happy path: the replay returns rows and evidence.
    let run = server.call(
        2,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert_eq!(run["result"]["isError"], false);
    let payload = &run["result"]["structuredContent"];
    assert_eq!(payload["result"]["row_count"], 1);
    assert_eq!(payload["connection"], "local");
    assert!(
        payload["evidence"]["execution_id"].is_string(),
        "the replay carries evidence: {payload}"
    );

    // Stale path: an edit moves the revision; the replay is refused with the
    // CLI's own stale message and is never revalidated.
    let (code, out, err) = fixture.cli(&["investigation", "edit", &id, "--sql", "SELECT 2 AS n"]);
    assert_eq!(code, 0, "edit failed: {out}{err}");
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert!(
        text.contains("review is stale"),
        "the stale refusal is the CLI's message: {text}"
    );
    assert!(
        !text.contains("revalidated") || text.contains("--revalidate"),
        "the refusal points at --revalidate instead of doing it: {text}"
    );

    // The binding was not rewritten by the MCP call: the CLI run still needs
    // --revalidate.
    let (code, out, err) = fixture.cli(&["investigation", "run", &id]);
    assert_ne!(code, 0, "MCP must not have revalidated: {out}{err}");

    server.close_and_expect_exit(0);
}

/// `investigation_run` binds declared parameters: `params` maps names to
/// string values, parsed by the same typed parser the CLI uses; a missing
/// required parameter is an isError listing the names and types, and no
/// error ever echoes a supplied value.
#[test]
fn mcp_investigation_run_binds_parameters() {
    const SENTINEL: &str = "HVNS3NT1NEL42";
    let fixture = crate_fixture("replay-params");
    fixture.seed_events();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    // Save through the CLI with a required integer parameter.
    let (code, out, err) = fixture.cli(&[
        "investigation",
        "save",
        "--name",
        "events up to",
        "--sql",
        "SELECT id, label FROM events WHERE id <= :max_id ORDER BY id",
        "--param-spec",
        "max_id:integer:required",
        "--connection",
        "local",
    ]);
    assert_eq!(code, 0, "save failed: {out}{err}");
    let id = out
        .lines()
        .next()
        .expect("the id is the first line")
        .trim()
        .to_string();

    server.handshake();

    // Happy path: params bind as typed values and the result is filtered;
    // the replay keeps its saved-investigation evidence source and records
    // the bound parameters' names.
    let run = server.call(
        2,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"max_id":"1"}}}}"#),
    );
    assert_eq!(
        run["result"]["isError"],
        false,
        "{}",
        serde_json::to_string(&run).unwrap()
    );
    let payload = &run["result"]["structuredContent"];
    assert_eq!(
        payload["result"]["rows"],
        serde_json::json!([[1, "first"]]),
        "the bound parameter filtered the replay: {}",
        serde_json::to_string(payload).unwrap()
    );
    assert_eq!(payload["evidence"]["source"]["kind"], "saved_investigation");
    assert_eq!(
        payload["evidence"]["param_names"],
        serde_json::json!(["max_id"])
    );

    // Missing required parameter: isError with the names and types.
    let text = server.call_error_text(
        3,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local"}}"#),
    );
    assert!(
        text.contains("missing required parameter(s)")
            && text.contains("max_id")
            && text.contains("integer"),
        "the refusal lists the required names with types: {text}"
    );

    // A malformed value is the typed parser's refusal, and the value itself
    // is never echoed — not in this refusal, and not for an unknown name.
    let text = server.call_error_text(
        4,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"max_id":"{SENTINEL}"}}}}"#),
    );
    assert!(
        text.contains("max_id") && text.contains("not a valid integer"),
        "the refusal is the typed parser's: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the value is never echoed: {text}"
    );

    let text = server.call_error_text(
        5,
        "investigation_run",
        &format!(r#"{{"id":"{id}","profile":"local","params":{{"nope":"{SENTINEL}"}}}}"#),
    );
    assert!(
        text.contains("no parameter named"),
        "the refusal names the undeclared parameter: {text}"
    );
    assert!(
        !text.contains(SENTINEL),
        "the value is never echoed: {text}"
    );

    // A non-string value in the params map is a JSON-RPC parameter error,
    // and the server keeps serving.
    server.send(&format!(
        r#"{{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{{"name":"investigation_run","arguments":{{"id":"{id}","profile":"local","params":{{"max_id":1}}}}}}}}"#
    ));
    let refused = server.next_json(10);
    assert_eq!(refused["id"], 6);
    assert_eq!(
        refused["error"]["code"], -32602,
        "a non-string param value is a parameter error: {refused}"
    );
    server.send(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#);
    let ping = server.next_json(5);
    assert_eq!(ping["id"], 7);
    assert!(ping["result"].is_object());

    server.close_and_expect_exit(0);
}
