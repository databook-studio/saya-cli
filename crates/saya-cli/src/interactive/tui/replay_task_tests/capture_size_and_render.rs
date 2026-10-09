/// An oversize replay honours the same capture budget the direct-/sql path
/// uses, through the same test-sized walk: over budget the result is not
/// captured (any previous capture is cleared), one visible system line names
/// the way out, and — the query having succeeded — the replay is still the
/// latest selectable query.
#[test]
fn oversize_replay_is_not_captured_and_says_so() {
    let big = QueryResult {
        columns: vec!["text".to_string()],
        rows: vec![serde_json::json!(["x".repeat(200)])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT text FROM t".to_string(),
    };
    // The existing test-sized budget seam (`capture::accounted_bytes_within`,
    // the walk `capture_within` takes): a small budget refuses the big result
    // and admits the small one.
    let accounted = super::super::capture::accounted_bytes_within(&big, 64);
    assert!(
        accounted.is_none(),
        "precondition: the big result refuses the budget"
    );
    assert!(
        super::super::capture::accounted_bytes_within(&sample_result(), 64).is_some(),
        "precondition: the small result stays within it"
    );
    let replay = Replay {
        result: big,
        evidence: saved_evidence(),
        sql: "SELECT text FROM t".to_string(),
        connection: "local".to_string(),
    };
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured = Some(CapturedResult {
        result: sample_result(),
        evidence: saved_evidence(),
    });
    complete(
        done(0, "the replay output", Some(replay)),
        accounted,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    // The query succeeded: the replay is still selectable.
    let last_query = last_query
        .as_ref()
        .expect("the oversize replay is still selectable");
    assert_eq!(last_query.sql, "SELECT text FROM t");
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    // Over budget: not captured, the previous capture cleared, one visible line.
    assert!(captured.is_none(), "an oversize replay is not captured");
    let note = last_block(&transcript, BlockKind::System).expect("the refusal is visible");
    assert!(note.contains("Result not captured"), "{note}");
    assert!(note.contains("larger than 32 MiB"), "{note}");
    assert!(
        note.contains("/export --refresh will re-run it"),
        "the refusal names the way out: {note}"
    );
}

/// A successful completion renders the typed replay as one Table block (the
/// captured CLI text is dropped — its tabs would be stripped), captures the
/// replay (source saved investigation, scope full), and sets the latest
/// selectable query to the replay's SQL and connection.
#[test]
fn replay_completion_sets_capture_and_last_query() {
    let mut app = idle_app();
    let mut state = SessionState::new("test", None, "model");
    let store = session_store(temp_dir("capture").as_path());
    let (tx, rx) = mpsc::channel();
    tx.send(done(0, "table + evidence text", Some(replay())))
        .expect("pre-delivered completion");
    put_running_replay(&mut app, rx, "recent-orders");

    tick_workers(&mut app, &store, &mut state);

    assert!(app.replay_task.is_none() && !app.is_busy());
    assert!(
        last_block(&app.transcript, BlockKind::System).is_none(),
        "the captured CLI text is not pushed: {:?}",
        app.transcript.blocks()
    );
    let block =
        last_block(&app.transcript, BlockKind::Table).expect("the replay renders as a table block");
    assert!(
        block.contains("│ one │"),
        "the result table renders: {block}"
    );
    assert!(
        block.contains("saved investigation: local"),
        "the evidence line rides the table block: {block}"
    );
    let captured = app
        .captured
        .as_ref()
        .expect("a successful replay is captured");
    assert!(
        matches!(
            captured.evidence.source,
            EvidenceSource::SavedInvestigation { .. }
        ),
        "the capture is a saved-investigation result: {:?}",
        captured.evidence.source
    );
    assert_eq!(
        captured.evidence.scope,
        ResultScope::Full,
        "a replay is a full read"
    );
    assert_eq!(captured.result.row_count, 1);
    let last_query = app.last_query.as_ref().expect("the replay is selectable");
    assert_eq!(last_query.sql, "SELECT 1 AS one");
    assert_eq!(
        last_query.connection.as_deref(),
        Some("local"),
        "the selectable query names the connection the replay ran on"
    );
    assert_eq!(
        app.request.started, None,
        "the bar's status fields are released"
    );
    assert_eq!(app.request.activity, None);
}

/// A successful replay renders its rows exactly like a direct /sql result:
/// one Table block — the box table, the scope line, the evidence line — and
/// never the captured CLI text, whose tab-separated rows a System block
/// strips into concatenated words (`regionjoined_rows…`).
#[test]
fn successful_replay_renders_a_table_block_with_evidence() {
    let result = QueryResult {
        columns: vec![
            "region".to_string(),
            "joined_rows".to_string(),
            "distinct_orders".to_string(),
        ],
        rows: vec![serde_json::json!(["north", 8, 359])],
        row_count: 1,
        truncated: false,
        executed_sql: "SELECT region, joined_rows, distinct_orders FROM orders".to_string(),
    };
    let evidence = saved_evidence_for(&result);
    let accounted = accounted_bytes(&result);
    let replay = Replay {
        result,
        evidence,
        sql: "SELECT region, joined_rows, distinct_orders FROM orders".to_string(),
        connection: "local".to_string(),
    };
    // The captured CLI text: tab-separated rows and the evidence line — the
    // non-TTY render whose tabs a System block strips.
    let captured_text = "region\tjoined_rows\tdistinct_orders\n\
                         north\t8\t359\n\
                         saved investigation: local · 1 rows · exec xabc-1 · full result";
    let mut transcript = Transcript::new();
    let mut last_query: Option<LastQuery> = None;
    let mut captured: Option<CapturedResult> = None;
    complete(
        done(0, captured_text, Some(replay)),
        accounted,
        &mut transcript,
        &mut last_query,
        &mut captured,
    );
    let block = transcript
        .blocks()
        .last()
        .expect("the completion pushes one block");
    assert_eq!(
        block.kind,
        BlockKind::Table,
        "the rows render as a table block: {:?}",
        transcript.blocks()
    );
    let text = block.text.as_str();
    // Columns separated by the table's separator — never the concatenated
    // words the tab-stripped captured text produced.
    assert!(
        text.contains("│ region │ joined_rows │ distinct_orders │"),
        "header cells are separated by the table's separator: {text}"
    );
    assert!(
        !text.contains("regionjoined_rows"),
        "no tab-stripped concatenation: {text}"
    );
    assert!(
        text.contains("│ north") && text.contains(" 8 │") && !text.contains("north8"),
        "the row renders boxed, not tab-stripped: {text}"
    );
    // The scope line, built exactly as the /sql path builds it.
    assert!(
        text.contains("from local · SELECT region, joined_rows, distinct_orders FROM orders"),
        "the scope line names the connection and SQL: {text}"
    );
    // The evidence line under the table.
    assert!(
        text.contains("saved investigation: local") && text.contains("full result"),
        "the evidence line rides the table block: {text}"
    );
    // The captured CLI text is dropped entirely: no tab ever reaches a block.
    assert!(
        transcript.blocks().iter().all(|b| !b.text.contains('\t')),
        "the captured tab-separated text is never pushed: {:?}",
        transcript.blocks()
    );
    assert_eq!(
        transcript.blocks().len(),
        1,
        "the table block replaces the captured text: {:?}",
        transcript.blocks()
    );
    let last_query = last_query.as_ref().expect("the replay is selectable");
    assert_eq!(
        last_query.sql,
        "SELECT region, joined_rows, distinct_orders FROM orders"
    );
    assert_eq!(last_query.connection.as_deref(), Some("local"));
    let captured = captured
        .as_ref()
        .expect("within budget, the replay is captured");
    assert_eq!(captured.result.row_count, 1);
}
