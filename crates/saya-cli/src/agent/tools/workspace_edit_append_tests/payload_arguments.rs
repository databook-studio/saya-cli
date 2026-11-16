use super::*;

/// Refusals carry sizes, offsets and digests — never file content. A success
/// result carries only the bounded changed excerpt, so a secret-shaped
/// sentinel outside that window never appears in any payload.
#[tokio::test]
async fn an_append_refusal_never_carries_file_content_and_the_result_is_bounded() {
    let sandbox = Sandbox::new("append-sentinel");
    let sentinel = "token=SENTINEL-9d2b4e-must-never-leak";
    let mut seed = format!("{sentinel}\n");
    for line in 0..10 {
        seed.push_str(&format!("filler {line}\n"));
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
                "offset": 1,
                "chunk": "tail;",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a stale offset must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("SENTINEL-9d2b4e"),
        "an offset-mismatch refusal must not carry file content: {text}"
    );
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "offset": seed.len(),
                "chunk": "tail;",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the correct offset must append");
    let rendered = result.to_string();
    assert!(
        !rendered.contains("SENTINEL-9d2b4e"),
        "the excerpt is bounded to the change, not the whole file: {rendered}"
    );
    let excerpt = result["changed_excerpt"]
        .as_str()
        .expect("the append result carries the changed excerpt");
    assert!(
        excerpt.contains("tail;"),
        "the appended region is shown: {excerpt}"
    );
    assert!(
        excerpt.len() <= 4 * 1024,
        "the excerpt is bounded, never the bytes: {} bytes",
        excerpt.len()
    );
}

/// Mixed halves are a validation error, never a guess: `old_text` with
/// `chunk`, or half of one variant, is rejected before any filesystem
/// contact, and a mistyped `offset`/`chunk` names its own error.
#[tokio::test]
async fn append_rejects_mixed_and_mistyped_arguments() {
    let sandbox = Sandbox::new("append-mixed");
    sandbox
        .ws
        .write("notes.md", b"unchanged")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let before = fs::read(sandbox.ws_root().join("notes.md")).expect("seed file must exist");
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "offset": 0,
                "chunk": "c",
            }),
        )
        .await
        .expect_err("mixed halves must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::EditShapeConflict);
    let error = tools
        .execute("edit", serde_json::json!({"path": "notes.md", "offset": 0}))
        .await
        .expect_err("half an append must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ChunkNotString);
    let error = tools
        .execute(
            "edit",
            serde_json::json!({"path": "notes.md", "offset": "zero", "chunk": "c"}),
        )
        .await
        .expect_err("a non-integer offset must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::OffsetNotUint);
    let error = tools
        .execute(
            "edit",
            serde_json::json!({"path": "notes.md", "offset": 0, "chunk": 7}),
        )
        .await
        .expect_err("a non-string chunk must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ChunkNotString);
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        before,
        "missing and mixed variants are rejected before mutation"
    );
}
