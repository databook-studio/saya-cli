use super::*;

struct RangeReadCase<'a> {
    path: &'a str,
    bytes: &'a [u8],
    start: u64,
    count: u64,
    content: &'a str,
    total_lines: u64,
    lines_returned: u64,
    truncated: bool,
}

#[tokio::test]
async fn ranged_read_returns_late_lines_and_the_whole_file_digest() {
    let sandbox = Sandbox::new("late-range");
    let lines = (1..=3_000)
        .map(|line| format!("line-{line:04}-{}\n", "x".repeat(90)))
        .collect::<String>();
    let bytes = lines.as_bytes();
    assert!(bytes.len() as u64 > WORKSPACE_READ_MAX_BYTES);
    let expected = sha256_hex(bytes);
    sandbox
        .ws
        .write("large.txt", bytes)
        .expect("seed write must succeed");
    let tools = sandbox.tools();

    let result = tools
        .execute(
            "read",
            serde_json::json!({
                "path": "large.txt",
                "start_line": 900,
                "line_count": 2
            }),
        )
        .await
        .expect("a valid late line range should be readable");

    let expected_content = (900..=901)
        .map(|line| format!("{line:>6}→line-{line:04}-{}\n", "x".repeat(90)))
        .collect::<String>();
    assert_eq!(result["content"], expected_content);
    assert_eq!(result["size"].as_u64(), Some(bytes.len() as u64));
    assert_eq!(result["digest"].as_str(), Some(expected.as_str()));
    assert_eq!(result["truncated"], true);
    assert_eq!(result["selected_range_complete"], true);
}

#[tokio::test]
async fn ranged_read_reports_lf_boundaries_and_preserves_original_line_bytes() {
    let sandbox = Sandbox::new("line-boundaries");
    let cases = [
        RangeReadCase {
            path: "crlf.txt",
            bytes: b"first\r\nsecond\r\n",
            start: 1,
            count: 1,
            content: "     1→first\r\n",
            total_lines: 2,
            lines_returned: 1,
            truncated: true,
        },
        RangeReadCase {
            path: "empty.txt",
            bytes: b"",
            start: 1,
            count: 2,
            content: "",
            total_lines: 0,
            lines_returned: 0,
            truncated: false,
        },
        RangeReadCase {
            path: "tail.txt",
            bytes: b"tail",
            start: 1,
            count: 2,
            content: "     1→tail",
            total_lines: 1,
            lines_returned: 1,
            truncated: false,
        },
        RangeReadCase {
            path: "past-eof.txt",
            bytes: b"one\nlast",
            start: 3,
            count: 2,
            content: "",
            total_lines: 2,
            lines_returned: 0,
            truncated: true,
        },
    ];
    let tools = sandbox.tools();

    for case in cases {
        sandbox
            .ws
            .write(case.path, case.bytes)
            .expect("fixture write must succeed");
        let result = tools
            .execute(
                "read",
                serde_json::json!({
                    "path": case.path,
                    "start_line": case.start,
                    "line_count": case.count
                }),
            )
            .await
            .expect("valid line ranges must be readable");
        assert_eq!(result["content"], case.content, "{}", case.path);
        assert_eq!(
            result["total_lines"].as_u64(),
            Some(case.total_lines),
            "{}",
            case.path
        );
        assert_eq!(
            result["lines_returned"].as_u64(),
            Some(case.lines_returned),
            "{}",
            case.path
        );
        assert_eq!(result["selected_range_complete"], true, "{}", case.path);
        // A range that delivered the whole file carries no `truncated`: the
        // flag is served only when the read was cut.
        assert_eq!(
            result.get("truncated").and_then(serde_json::Value::as_bool),
            case.truncated.then_some(true),
            "{}",
            case.path
        );
    }
}

#[tokio::test]
async fn ranged_read_rejects_line_range_overflow_before_opening_the_path() {
    let sandbox = Sandbox::new("line-overflow");
    let error = sandbox
        .tools()
        .execute(
            "read",
            serde_json::json!({
                "path": "not-created.txt",
                "start_line": u64::MAX,
                "line_count": 2
            }),
        )
        .await
        .expect_err("an overflowing one-based line range must refuse");
    assert_eq!(error, saya_agent::ToolError::UnsupportedProperty);
    assert!(!sandbox.ws.root().join("not-created.txt").exists());
}

#[tokio::test]
async fn ranged_result_fits_after_secret_redaction_without_losing_range_metadata() {
    let sandbox = Sandbox::new("range-shape-redaction");
    // One marker at the head of a long filler run, then the multibyte tail:
    // the byte cut always lands inside the run, so the fail-closed redaction
    // of the marker never reaches across the JSON keys that follow it.
    let line = format!("token={} fin café終\n", "x".repeat(16_000));
    sandbox
        .ws
        .write("long-line.txt", line.as_bytes())
        .expect("fixture write must succeed");
    let result = sandbox
        .tools()
        .execute_with_result_cap(
            "read",
            serde_json::json!({
                "path": "long-line.txt",
                "start_line": 1,
                "line_count": 1
            }),
            2_048,
        )
        .await
        .expect("the model-facing shaped result should fit the assigned cap");
    let shaped = saya_agent::shape_tool_result(&result, 2_048);

    assert!(!shaped.truncated, "the actual provider projection must fit");
    assert!(shaped.redactions > 0);
    assert!(shaped.text.contains("[redacted]"));
    assert!(shaped.text.contains("\"digest\""));
    assert!(shaped.text.contains("\"selected_range_complete\":false"));
    assert_eq!(result["selected_range_complete"], false);
}
