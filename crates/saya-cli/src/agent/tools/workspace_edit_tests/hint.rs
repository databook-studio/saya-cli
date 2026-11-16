/// A zero-match edit names the closest region of the current file, with its
/// line numbers and text, so the model can correct its anchor without a
/// fresh read.
#[tokio::test]
async fn a_zero_match_edit_names_the_nearest_region() {
    let sandbox = Sandbox::new("hint-nearest");
    sandbox
        .ws
        .write("notes.md", b"one\nlet total = add(a, b);\nthree\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "let total = add(a, c);",
                "new_text": "let total = add(a, b);",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a missing anchor must refuse");
    let text = error.to_string();
    assert!(text.contains("nearest lines 2-2:"), "{text}");
    assert!(
        text.contains("let total = add(a, b);"),
        "the candidate line is shown: {text}"
    );
    let hint = text
        .rfind("notes.md")
        .map(|at| &text[at + "notes.md".len()..])
        .expect("the error names the path");
    assert!(
        hint.len() <= 2048,
        "the hint stays within its byte budget: {}",
        hint.len()
    );
}

/// The hint is bounded: at most three regions, and none at all when the
/// anchor itself is over the similarity bound.
#[tokio::test]
async fn a_hint_serves_at_most_three_regions_and_skips_a_large_anchor() {
    let sandbox = Sandbox::new("hint-bounds");
    let mut seed = String::new();
    for index in 0..20 {
        seed.push_str(&format!("let total = add(a, b{index});\n"));
    }
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "let total = add(a, c);",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a missing anchor must refuse");
    let text = error.to_string();
    assert_eq!(
        text.matches("nearest lines").count(),
        3,
        "at most three candidate regions: {text}"
    );

    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "z".repeat(4097),
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a missing anchor must refuse");
    assert!(
        !error.to_string().contains("nearest lines"),
        "an anchor over the similarity bound gets no hint: {error}"
    );
}

/// Similarity work is skipped entirely for a file over the hint's file bound,
/// even when every line shares tokens with the anchor.
#[tokio::test]
async fn a_hint_is_skipped_for_a_file_over_its_bound() {
    let sandbox = Sandbox::new("hint-file-bound");
    let seed = "filler line here\n".repeat(70_000);
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "filler line elsewhere",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a missing anchor must refuse");
    assert!(
        !error.to_string().contains("nearest lines"),
        "a file over the hint bound gets no hint: {error}"
    );
}
