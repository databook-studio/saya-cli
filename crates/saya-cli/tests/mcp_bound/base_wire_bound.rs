/// A query whose payload alone fits the 16 MiB bound but whose built reply
/// (the duplicated text content block) does not: the reply LINE on the wire
/// stays within the bound, narrowed by dropping whole rows — `truncated`,
/// the row counts, and the evidence all agree — and the server exits cleanly.
#[test]
fn mcp_query_reply_line_stays_within_the_wire_bound() {
    let fixture = fixture("wire");
    fixture.seed_blobs();
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    let reply_line = server.call_query(json!(2), "SELECT id, payload FROM blobs ORDER BY id");

    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(&reply_line).expect("the reply line is JSON");
    assert_eq!(
        reply["result"]["isError"], false,
        "the oversized answer is narrowed and sent, not refused"
    );
    let payload = &reply["result"]["structuredContent"];
    assert_eq!(
        payload["truncated"], true,
        "the answer is marked truncated: {}",
        payload["note"]
    );
    let sent = payload["row_count"]
        .as_u64()
        .expect("row_count is a number");
    assert!(
        sent < ROWS,
        "rows were dropped to fit the wire: {sent} of {ROWS}"
    );
    assert_eq!(
        payload["rows"].as_array().expect("rows array").len() as u64,
        sent,
        "row_count matches the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["returned_rows"], sent,
        "the evidence reports the rows actually sent"
    );
    assert_eq!(
        payload["evidence"]["truncated"], true,
        "the evidence reports the narrowing"
    );
    assert!(
        payload["note"]
            .as_str()
            .unwrap_or_default()
            .contains("cut to"),
        "the narrowing is named: {}",
        payload["note"]
    );

    server.close_and_expect_exit(0);
}
