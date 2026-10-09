/// The audit's exact case (R061-3, D9): a 200,000-character ASCII request id
/// rides the reply envelope, so the built result the old fixed-reserve
/// measure admitted whole produced a 16,841,216-byte reply line — 64,000
/// bytes over the bound. The budget must come from the actual envelope.
#[test]
fn mcp_query_reply_line_holds_with_the_audit_large_ascii_request_id() {
    let fixture = fixture("id-ascii");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("i".repeat(200_000)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// A request id full of `"` and `\`: the inbound line already carries it
/// escaped (two bytes per character), and rmcp re-serializes the same
/// escaping into the reply envelope — the id's wire size is doubled. The
/// reply line must still fit.
#[test]
fn mcp_query_reply_line_holds_with_an_escaping_request_id() {
    let fixture = fixture("id-escaping");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let id = format!("{}{}", "\"".repeat(100_000), "\\".repeat(100_000));
    let reply_line = server.call_query(json!(id), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}

/// A non-ASCII request id (100,000 `é`, two UTF-8 bytes each): serde_json
/// carries it through the envelope as raw UTF-8, no `\u` escaping, so the
/// id contributes the same wire bytes as the ASCII case — and the reply
/// line must still fit.
#[test]
fn mcp_query_reply_line_holds_with_a_non_ascii_request_id() {
    let fixture = fixture("id-nonascii");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();
    let reply_line = server.call_query(json!("é".repeat(100_000)), AUDIT_SQL);
    assert_reply_line_fits_and_stays_consistent(&reply_line);
    server.close_and_expect_exit(0);
}
