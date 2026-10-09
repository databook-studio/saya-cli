#[tokio::test]
async fn grep_reports_literal_hits_with_one_based_line_numbers() {
    let sandbox = Sandbox::new("grep-content");
    sandbox
        .ws
        .write("log.txt", b"one\ntwo\nneedle\nfour")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(0));
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["path"], "log.txt");
    assert_eq!(
        hits[0]["line"].as_u64(),
        Some(3),
        "line numbers are 1-based"
    );
    assert_eq!(hits[0]["text"], "needle");
    assert_eq!(hits[0]["truncated"], serde_json::Value::Bool(false));
}

#[tokio::test]
async fn grep_honours_the_case_insensitive_flag() {
    let sandbox = Sandbox::new("grep-case");
    sandbox
        .ws
        .write("log.txt", b"Hello World\nsecond")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "hello world", "case_insensitive": false}),
        )
        .await
        .expect("a contained grep must succeed");
    assert_eq!(result["matches"].as_array().expect("matches").len(), 0);
    let result = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "hello world", "case_insensitive": true}),
        )
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["line"].as_u64(), Some(1));
}

/// A file bigger than the per-file search horizon is skipped whole, never
/// half-searched: a hit list over a prefix would read as full coverage. A
/// non-UTF-8 file is skipped rather than served as mojibake. Both land in
/// `files_skipped`, so a miss is never mistaken for proof of absence.
#[tokio::test]
async fn grep_skips_oversized_and_non_utf8_files_and_counts_the_skips() {
    let sandbox = Sandbox::new("grep-skip");
    let mut oversized = vec![b'a'; WORKSPACE_GREP_MAX_FILE_BYTES as usize];
    oversized.extend_from_slice(b"needle");
    fs::write(sandbox.ws.root().join("big.txt"), &oversized)
        .expect("oversized fixture must be written directly");
    sandbox
        .ws
        .write("bin.dat", b"\xff\xfe\xfd")
        .expect("contained write must succeed");
    sandbox
        .ws
        .write("small.txt", b"needle here")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1, "only the fully readable file is searched");
    assert_eq!(hits[0]["path"], "small.txt");
    assert_eq!(result["files_scanned"].as_u64(), Some(1));
    assert_eq!(result["files_skipped"].as_u64(), Some(2));
}

/// One enormous line cannot blow the context: the reported text is capped at
/// exactly the line bound and the cap is reported on the hit.
#[tokio::test]
async fn grep_bounds_the_reported_line_itself() {
    let sandbox = Sandbox::new("grep-line-bound");
    let mut line = b"needle".to_vec();
    line.extend(std::iter::repeat_n(b'a', 20_000));
    sandbox
        .ws
        .write("wide.txt", &line)
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect("a contained grep must succeed");
    let hits = result["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["line"].as_u64(), Some(1));
    assert_eq!(
        hits[0]["text"].as_str().expect("text").len(),
        WORKSPACE_GREP_MAX_LINE_BYTES
    );
    assert_eq!(hits[0]["truncated"], serde_json::Value::Bool(true));
}

#[tokio::test]
async fn grep_reports_a_match_count_overrun_as_a_typed_error() {
    let sandbox = Sandbox::new("grep-match-bound");
    sandbox.plant(
        (0..WORKSPACE_GREP_MAX_MATCHES + 1).map(|index| format!("f{index}.txt")),
        b"needle",
    );
    let tools = sandbox.tools();
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect_err("an over-bound match list must be refused, not truncated");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("bound exceeded"),
                "the refusal must name the bounds failure: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

#[tokio::test]
async fn grep_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle"}))
        .await
        .expect_err("no workspace, no grep");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
}

#[tokio::test]
async fn grep_rejects_a_non_string_pattern_and_a_non_boolean_flag() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("grep", serde_json::json!({"pattern": 7}))
        .await
        .expect_err("a non-string pattern must be rejected at validation");
    assert_eq!(error, ToolError::PatternNotString);
    let error = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "needle", "case_insensitive": "yes"}),
        )
        .await
        .expect_err("a non-boolean flag must be rejected at validation");
    assert_eq!(error, ToolError::CaseInsensitiveNotBool);
    let error = tools
        .execute("grep", serde_json::json!({"pattern": "needle", "extra": 1}))
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
