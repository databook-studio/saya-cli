// `grep`'s presentation arguments as the model states them: `output_mode`,
// `context`, `type`, `multiline` and `head_limit`. Each mode serves its own
// JSON shape, and each answer still reports its coverage.

async fn grep(sandbox: &Sandbox, arguments: serde_json::Value) -> serde_json::Value {
    sandbox
        .tools()
        .execute("grep", arguments)
        .await
        .expect("a contained grep must succeed")
}

#[tokio::test]
async fn output_modes_serve_paths_counts_and_lines_with_their_coverage() {
    let sandbox = Sandbox::new("grep-modes");
    sandbox.ws.write("a.txt", b"needle one\nneedle two\n").unwrap();
    sandbox.ws.write("b.txt", b"needle\n").unwrap();
    sandbox.ws.write("c.txt", b"nothing here\n").unwrap();
    let files = grep(
        &sandbox,
        serde_json::json!({"pattern": "needle", "output_mode": "files_with_matches"}),
    )
    .await;
    assert_eq!(
        files["matches"],
        serde_json::json!(["a.txt", "b.txt"]),
        "files_with_matches serves one bare path per matching file"
    );
    assert_eq!(files["matches_found"], 2);
    assert_eq!(files["total_matches"], 2);
    assert_eq!(files["omitted"], 0);
    assert_eq!(files["coverage_complete"], true);
    assert_eq!(files["truncated"], false);

    let counts = grep(
        &sandbox,
        serde_json::json!({"pattern": "needle", "output_mode": "count"}),
    )
    .await;
    assert_eq!(
        counts["matches"],
        serde_json::json!([{"path": "a.txt", "count": 2}, {"path": "b.txt", "count": 1}])
    );
    assert_eq!(counts["matches_found"], 2);
    assert_eq!(counts["total_matches"], 2);
    assert_eq!(counts["coverage_complete"], true);

    let lines = grep(&sandbox, serde_json::json!({"pattern": "needle"})).await;
    assert_eq!(
        lines["matches"],
        serde_json::json!([
            {"path": "a.txt", "line": 1, "text": "needle one"},
            {"path": "a.txt", "line": 2, "text": "needle two"},
            {"path": "b.txt", "line": 1, "text": "needle"}
        ]),
        "the default content mode is unchanged, and a hit carries no context marker"
    );
    assert_eq!(lines["matches_found"], 3);
    assert_eq!(lines["total_matches"], 3);
    assert_eq!(lines["coverage_complete"], true);
}

#[tokio::test]
async fn context_lines_mark_themselves_and_never_count_as_found() {
    let sandbox = Sandbox::new("grep-context");
    sandbox
        .ws
        .write("edges.txt", b"needle first\nb\nc\nd\ne\nneedle last\n")
        .unwrap();
    let result = grep(&sandbox, serde_json::json!({"pattern": "needle", "context": 2})).await;
    assert_eq!(
        result["matches"],
        serde_json::json!([
            {"path": "edges.txt", "line": 1, "text": "needle first"},
            {"path": "edges.txt", "line": 2, "text": "b", "context": true},
            {"path": "edges.txt", "line": 3, "text": "c", "context": true},
            {"path": "edges.txt", "line": 4, "text": "d", "context": true},
            {"path": "edges.txt", "line": 5, "text": "e", "context": true},
            {"path": "edges.txt", "line": 6, "text": "needle last"}
        ]),
        "two lines each side of a hit at the first and at the last line"
    );
    assert_eq!(result["matches_found"], 2);
    assert_eq!(result["total_matches"], 2);
    assert_eq!(result["omitted"], 0);
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["truncated"], false);
}

#[tokio::test]
async fn head_limit_narrows_the_served_entries_without_lying_about_omitted() {
    let sandbox = Sandbox::new("grep-head-limit");
    sandbox
        .ws
        .write("many.txt", b"needle\nneedle\nneedle\nneedle\nneedle\n")
        .unwrap();
    let result = grep(
        &sandbox,
        serde_json::json!({"pattern": "needle", "head_limit": 2}),
    )
    .await;
    assert_eq!(result["matches"].as_array().unwrap().len(), 2);
    assert_eq!(result["matches_found"], 5);
    assert_eq!(result["total_matches"], 5);
    assert_eq!(result["omitted"], 3);
    assert_eq!(
        result["coverage_complete"], true,
        "the walk still finished, so the counts are exact"
    );
    assert_eq!(result["truncated"], true);
}

#[tokio::test]
async fn a_type_outside_the_bounded_map_is_a_typed_refusal() {
    let sandbox = Sandbox::new("grep-type-unknown");
    sandbox.ws.write("a.rs", b"needle\n").unwrap();
    let error = sandbox
        .tools()
        .execute(
            "grep",
            serde_json::json!({"pattern": "needle", "type": "cobol"}),
        )
        .await
        .expect_err("an unknown type must be refused, not searched");
    let message = error.to_string();
    assert!(
        message.contains("cobol") && message.contains("rust") && message.contains("py"),
        "the refusal names what was asked for and what is supported: {message}"
    );
}

#[tokio::test]
async fn a_type_keeps_only_that_languages_extensions() {
    let sandbox = Sandbox::new("grep-type");
    sandbox.ws.write("a.rs", b"needle\n").unwrap();
    sandbox.ws.write("b.txt", b"needle\n").unwrap();
    sandbox.ws.write("nested/c.rs", b"needle\n").unwrap();
    let result = grep(&sandbox, serde_json::json!({"pattern": "needle", "type": "rust"})).await;
    assert_eq!(
        result["matches"],
        serde_json::json!([
            {"path": "a.rs", "line": 1, "text": "needle"},
            {"path": "nested/c.rs", "line": 1, "text": "needle"}
        ])
    );
    assert_eq!(result["files_scanned"], 2);
    assert_eq!(result["total_matches"], 2);
    assert_eq!(result["coverage_complete"], true);
}

#[tokio::test]
async fn multiline_matches_across_a_line_boundary_and_reports_the_start_line() {
    let sandbox = Sandbox::new("grep-multiline");
    sandbox
        .ws
        .write("block.txt", b"before\nstart\nmiddle\nend\nafter\n")
        .unwrap();
    let result = grep(
        &sandbox,
        serde_json::json!({"pattern": "start\nmiddle\nend", "multiline": true}),
    )
    .await;
    assert_eq!(
        result["matches"],
        serde_json::json!([{
            "path": "block.txt", "line": 2, "text": "start\nmiddle\nend"
        }])
    );
    assert_eq!(result["matches_found"], 1);
    assert_eq!(result["total_matches"], 1);
    assert_eq!(result["coverage_complete"], true);
    let linewise = grep(
        &sandbox,
        serde_json::json!({"pattern": "start\nmiddle\nend"}),
    )
    .await;
    assert_eq!(linewise["matches_found"], 0);
    assert_eq!(linewise["coverage_complete"], true);
}

#[tokio::test]
async fn undefined_arguments_and_combinations_are_refused_before_any_search() {
    let sandbox = Sandbox::new("grep-refusals");
    sandbox.ws.write("a.txt", b"needle\n").unwrap();
    let tools = sandbox.tools();
    for arguments in [
        serde_json::json!({"pattern": "needle", "output_mode": "paths"}),
        serde_json::json!({"pattern": "needle", "context": 11}),
        serde_json::json!({"pattern": "needle", "context": "2"}),
        serde_json::json!({"pattern": "needle", "head_limit": 0}),
        serde_json::json!({"pattern": "needle", "head_limit": 501}),
        serde_json::json!({"pattern": "needle", "multiline": "yes"}),
        serde_json::json!({"pattern": "needle", "type": 7}),
    ] {
        let error = tools
            .execute("grep", arguments.clone())
            .await
            .expect_err("an undefined argument must be refused");
        assert!(
            matches!(
                error,
                ToolError::UnsupportedProperty | ToolError::NoEffect(_)
            ),
            "{arguments} must be a typed refusal, got {error:?}"
        );
    }
    let error = tools
        .execute(
            "grep",
            serde_json::json!({"pattern": "needle", "multiline": true, "context": 1}),
        )
        .await
        .expect_err("context cannot be combined with multiline");
    assert!(
        error.to_string().contains("multiline"),
        "the refusal names the combination: {error}"
    );
}

#[tokio::test]
async fn context_and_head_limit_stay_under_the_result_cap() {
    use saya_agent::{CancellationToken, ToolExecutionContext};
    let sandbox = Sandbox::new("grep-context-cap");
    sandbox
        .ws
        .write("lines.txt", "needle\n".repeat(200).as_bytes())
        .unwrap();
    let result = sandbox
        .tools()
        .execute_with_context(
            "grep",
            serde_json::json!({"pattern": "needle", "context": 10}),
            ToolExecutionContext {
                result_cap: 4096,
                cancellation: CancellationToken::new(),
            },
        )
        .await
        .unwrap();
    assert!(
        serde_json::to_vec(&result).unwrap().len() <= 4096,
        "context lines spend the same result cap every other entry does"
    );
    assert_eq!(result["matches_found"], 200);
    assert_eq!(result["total_matches"], 200);
    assert_eq!(result["coverage_complete"], true);
    assert_eq!(result["truncated"], true);
}
