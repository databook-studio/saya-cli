/// One probe of the boundary scan: send the audit SQL with an id of `length`
/// ASCII characters and the inline `_meta`, and return the reply line with
/// whether it was narrowed. Every reply must fit the bound and be sent, not
/// refused; a narrowed reply is marked truncated with consistent counts; a
/// full reply carries all 8 rows untruncated.
fn probe(server: &mut TestServer, length: usize) -> (String, bool) {
    let line =
        server.call_query_with_meta(json!("i".repeat(length)), Some(audit_meta()), AUDIT_SQL);
    assert!(
        line.len() <= MAX_RESPONSE_BYTES,
        "the reply LINE must stay within the {}-byte wire bound, was {} bytes \
         (id length {length})",
        MAX_RESPONSE_BYTES,
        line.len()
    );
    let reply: Value = serde_json::from_str(&line).expect("the reply line is JSON");
    assert_eq!(
        reply["result"]["isError"], false,
        "every scanned reply is sent, not refused"
    );
    let payload = &reply["result"]["structuredContent"];
    let sent = payload["row_count"]
        .as_u64()
        .expect("row_count is a number");
    assert_eq!(
        payload["rows"].as_array().expect("rows array").len() as u64,
        sent,
        "row_count matches the rows actually sent (id length {length})"
    );
    let narrowed = sent < 8;
    if narrowed {
        assert_eq!(
            payload["truncated"], true,
            "a narrowed reply is marked truncated (id length {length})"
        );
    } else {
        assert_eq!(
            payload["truncated"], false,
            "a full reply carries all 8 rows untruncated (id length {length})"
        );
    }
    (line, narrowed)
}

/// The exact boundary (R29-1): the reply line is linear in the id length —
/// one byte per ASCII id character rides the envelope — so two full-answer
/// probes below the flip determine the line's constant, and the flip is the
/// smallest id length whose measure tips over the bound. The scan samples id
/// lengths ±64 around that flip: every reply fits the bound, at least one
/// lands within 64 bytes of it — the boundary reply is expected to be
/// EXACTLY the bound — and both sides of the flip appear. One server process
/// carries the whole scan, and the probe count is kept small on purpose:
/// replies are megabytes and the unoptimized test profile makes each
/// round-trip take seconds.
#[test]
fn mcp_query_reply_line_boundary_scan_stays_under_the_cap() {
    let fixture = fixture("boundary-scan");
    let mut server = TestServer::spawn(
        &fixture,
        &["mcp", "serve", "--profile", "local", "--allow-data-sharing"],
    );

    server.handshake();

    // Two full-answer probes derive the line's id-length constant; agreeing
    // values confirm the reply line is linear in the id length below the
    // flip. The audit pinned the flip for this SQL and _meta at an id of
    // 135,977, so 100,000 and 101,000 are safely below it.
    let (line_a, narrowed_a) = probe(&mut server, 100_000);
    let (line_b, narrowed_b) = probe(&mut server, 101_000);
    assert!(
        !narrowed_a && !narrowed_b,
        "setup: both derivation probes get the full 8-row answer"
    );
    let constant_a = line_a.len() - 100_000;
    let constant_b = line_b.len() - 101_000;
    assert_eq!(
        constant_a, constant_b,
        "the reply line must be linear in the id length below the flip"
    );
    let flip = MAX_RESPONSE_BYTES - constant_a + 1;

    let mut saw_full = false;
    let mut saw_narrowed = false;
    let mut closest_to_bound = usize::MAX;
    for length in [
        flip.saturating_sub(64),
        flip.saturating_sub(32),
        flip - 1,
        flip,
        flip + 1,
        flip + 64,
    ] {
        let (line, narrowed) = probe(&mut server, length);
        saw_full |= !narrowed;
        saw_narrowed |= narrowed;
        if !narrowed {
            closest_to_bound = closest_to_bound.min(MAX_RESPONSE_BYTES - line.len());
        }
    }
    assert!(
        saw_full && saw_narrowed,
        "the scan must cross the narrowing flip (full: {saw_full}, narrowed: {saw_narrowed})"
    );
    assert!(
        closest_to_bound <= 64,
        "at least one reply must land within 64 bytes of the bound; the \
         closest full reply was {closest_to_bound} bytes below it"
    );

    server.close_and_expect_exit(0);
}
