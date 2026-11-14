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
    assert!(
        hits[0].get("truncated").is_none(),
        "a hit whose text fits carries no `truncated`: {}",
        hits[0]
    );
}

/// A literal search that finds nothing but whose pattern holds regex
/// metacharacters says so, so the model does not read "absent" for a regex it
/// forgot to ask for. The same pattern with `regex: true` matches, and a
/// literal miss with no metacharacters stays silent.
#[tokio::test]
async fn a_zero_match_literal_with_regex_metacharacters_suggests_regex() {
    let sandbox = Sandbox::new("grep-regex-hint");
    sandbox
        .ws
        .write("code.rs", b"fn foo() {}\n")
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let literal = tools
        .execute("grep", serde_json::json!({"pattern": "\\bfoo\\b"}))
        .await
        .expect("a contained grep must succeed");
    assert_eq!(literal["matches"].as_array().expect("matches").len(), 0);
    assert!(
        literal["note"]
            .as_str()
            .is_some_and(|note| note.contains("regex: true")),
        "the result names the fix: {literal}"
    );

    let regex = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "\\bfoo\\b", "regex": true}),
        )
        .await
        .expect("a contained regex grep must succeed");
    assert_eq!(regex["matches"].as_array().expect("matches").len(), 1);
    assert!(regex.get("note").is_none(), "a match carries no note");

    let plain = tools
        .execute("grep", serde_json::json!({"pattern": "absent"}))
        .await
        .expect("a contained grep must succeed");
    assert_eq!(plain["matches"].as_array().expect("matches").len(), 0);
    assert!(
        plain.get("note").is_none(),
        "no note without metacharacters: {plain}"
    );
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

/// A match flood is served short — the first `max` hits plus how many were
/// left out — never refused, and the walk still completes so the total is
/// exact rather than a guess.
#[tokio::test]
async fn grep_truncates_a_match_flood_with_an_omitted_count() {
    let sandbox = Sandbox::new("grep-match-bound");
    let planted = WORKSPACE_GREP_MAX_MATCHES + 200;
    sandbox.plant(
        (0..planted).map(|index| format!("f{index:03}.txt")),
        b"needle",
    );
    let result = sandbox
        .tools()
        .execute("grep", serde_json::json!({"pattern":"needle"}))
        .await
        .unwrap();
    assert_eq!(
        result["matches"].as_array().unwrap().len(),
        WORKSPACE_GREP_MAX_MATCHES
    );
    assert_eq!(result["truncated"], true);
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["matches_found"], planted as u64);
    assert_eq!(result["total_matches"], planted as u64);
    assert_eq!(result["omitted"], 200);
}

/// An absolute path under the root scopes the search like its relative
/// spelling; one outside the root stays refused.
#[tokio::test]
async fn grep_accepts_an_absolute_path_under_the_root_and_refuses_one_outside() {
    let sandbox = Sandbox::new("grep-absolute");
    sandbox
        .ws
        .write("src/lib.rs", b"needle\n")
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let under = sandbox.ws.root().join("src");
    let result = tools
        .execute(
            "grep",
            serde_json::json!({
                "pattern": "needle",
                "path": under.to_str().expect("utf-8 path"),
            }),
        )
        .await
        .expect("an absolute path under the root must be searched");
    assert_eq!(result["matches"][0]["path"], "src/lib.rs");

    let outside = sandbox
        .ws
        .root()
        .parent()
        .expect("the sandbox root has a parent")
        .join("outside");
    fs::create_dir_all(&outside).expect("outside directory must create");
    fs::write(outside.join("x.txt"), b"needle").expect("plant must succeed");
    let error = tools
        .execute(
            "grep",
            serde_json::json!({
                "pattern": "needle",
                "path": outside.to_str().expect("utf-8 path"),
            }),
        )
        .await
        .expect_err("an absolute path outside the root must be refused");
    assert!(
        matches!(error, ToolError::NoEffect(_)),
        "the refusal keeps its existing shape: {error}"
    );
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
