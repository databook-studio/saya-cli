/// The engine already marked the result truncated (the connector's 16 MiB
/// result cap stopped it mid-scan), and the reply STILL needed a byte cut:
/// the narrowing note must name the cut — the old note condition compared
/// the truncation flags, which were already equal, and the note went
/// missing. `row_count`, the note's numbers, and the evidence must agree.
#[test]
fn mcp_query_byte_cut_note_appears_when_the_engine_already_truncated() {
    let fixture = fixture("note");
    fixture.seed_blobs_count(20);
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!(3), "SELECT id, payload FROM blobs ORDER BY id");

    assert!(
        reply_line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes",
        MAX_RESPONSE_BYTES,
        reply_line.len()
    );
    let reply: Value = serde_json::from_str(&reply_line).expect("the reply line is JSON");
    assert_eq!(reply["result"]["isError"], false);
    let payload = &reply["result"]["structuredContent"];
    assert_eq!(
        payload["truncated"], true,
        "the engine already truncated; the byte cut keeps it truncated"
    );
    let note = payload["note"]
        .as_str()
        .expect("the byte cut must be named even when the engine had already truncated");
    // "the response exceeded the byte bound and was cut to {sent}/{total} rows"
    let cut = note
        .split("cut to ")
        .nth(1)
        .expect("the note quantifies the cut: {note}");
    let (sent, total) = cut
        .split_once('/')
        .expect("the note names sent/total rows: {note}");
    let sent: u64 = sent.trim().parse().expect("sent row count in the note");
    let total: u64 = total
        .trim()
        .trim_end_matches(" rows")
        .parse()
        .expect("total row count in the note");
    assert_eq!(
        payload["row_count"]
            .as_u64()
            .expect("row_count is a number"),
        sent,
        "the note's sent count is the payload's row_count: {note}"
    );
    assert!(total > sent, "rows were dropped for the byte bound: {note}");
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

    server.close_and_expect_exit(0);
}

/// The audit's exact case (R29-1): an id of 135,977 ASCII characters with the
/// inline `_meta` that keeps the `resultType` discriminator on the wire. The
/// budget counts the newline and the discriminator, so the full 8-row answer
/// does not fit by exactly one byte: the reply is narrowed — never refused,
/// never sent fat — and stays internally consistent.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_meta_and_boundary_id() {
    let fixture = fixture("id-meta");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line =
        server.call_query_with_meta(json!("i".repeat(135_977)), Some(audit_meta()), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// The same call on the legacy wire shape (no `_meta`): rmcp strips the
/// `resultType` discriminator after the measure, so the reply line is 24
/// bytes SHORTER than the budget — whatever fits the with-form budget fits
/// here with slack to spare.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_id_on_the_legacy_shape() {
    let fixture = fixture("id-legacy");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("i".repeat(135_977)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}
