use super::*;

/// The redaction contract holds for the append half: results and errors
/// carry sizes, offsets and digests — never file content. A secret-shaped
/// sentinel planted in the file must never appear in any payload.
#[tokio::test]
async fn an_append_error_payload_never_carries_file_content() {
    let sandbox = Sandbox::new("append-sentinel");
    let sentinel = "token=SENTINEL-9d2b4e-must-never-leak";
    let seed = format!("header line\n{sentinel}\nfooter line\n");
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
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
            "workspace_edit",
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
        "an append result must not carry file content: {rendered}"
    );
    assert!(
        !rendered.contains(&seed),
        "an append result carries size+digest, never the bytes: {rendered}"
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
            "workspace_edit",
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
    assert_eq!(error, saya_agent::ToolError::UnsupportedProperty);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "offset": 0}),
        )
        .await
        .expect_err("half an append must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ChunkNotString);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "offset": "zero", "chunk": "c"}),
        )
        .await
        .expect_err("a non-integer offset must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::OffsetNotUint);
    let error = tools
        .execute(
            "workspace_edit",
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
