/// An empty anchor matches everywhere, so it is a validation error — never
/// an edit, never a write.
#[tokio::test]
async fn an_empty_anchor_is_rejected() {
    let sandbox = Sandbox::new("empty");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("an empty anchor must be rejected");
    assert_eq!(
        error,
        saya_agent::ToolError::WorkspaceEditEmptyAnchor {
            path: "notes.md".into(),
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello workspace\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// A replacement over the bound refuses whole — never truncated — and leaves
/// no partial edit behind.
#[tokio::test]
async fn a_replacement_over_the_bound_refuses_whole() {
    let sandbox = Sandbox::new("bound");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": oversized,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("over the bound must be refused whole");
    match error {
        saya_agent::ToolError::WorkspaceEditTooLarge { limit, found, .. } => {
            assert_eq!(limit, WORKSPACE_EDIT_MAX_BYTES);
            assert_eq!(found, WORKSPACE_EDIT_MAX_BYTES + 1);
        }
        other => panic!("expected a typed over-bound refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello anchor world\n",
        "a refused edit leaves the file byte-identical"
    );
    // The boundary case: exactly the bound is a legal replacement.
    let exact = "a".repeat(WORKSPACE_EDIT_MAX_BYTES);
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": exact,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("a replacement exactly at the bound must be accepted");
    assert_eq!(result["path"], "notes.md");
}

/// An over-bound anchor refuses whole too: the range is never resolved, the
/// file never opened for a splice.
#[tokio::test]
async fn an_anchor_over_the_bound_refuses_whole() {
    let sandbox = Sandbox::new("anchor-bound");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": oversized,
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("an over-bound anchor must be refused whole");
    assert!(
        matches!(error, saya_agent::ToolError::WorkspaceEditTooLarge { .. }),
        "expected a typed over-bound refusal, got: {error}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello workspace\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// Errors carry counts, line numbers, sizes and digests — never unbounded
/// file content. A secret-shaped sentinel planted in the file must never
/// appear in any error payload.
#[tokio::test]
async fn an_error_payload_never_carries_file_content() {
    let sandbox = Sandbox::new("sentinel");
    // The sentinel is secret-shaped (`token=...`) so it would trip the
    // transcript redaction if it ever reached an error path; the contract is
    // stronger: it never reaches the payload at all.
    let sentinel = "token=SENTINEL-7f3a9c-must-never-leak";
    let seed = format!("header line\n{sentinel}\nfooter line\n");
    sandbox
        .ws
        .write("notes.md", seed.as_bytes())
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    // Zero matches: the refusal names the anchor argument (model-supplied),
    // never the file.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "no such anchor",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("zero matches must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("SENTINEL-7f3a9c"),
        "a zero-match refusal must not carry file content: {text}"
    );
    // Multiple matches on a *different* anchor: the refusal names counts and
    // lines, never the sentinel line's content.
    sandbox
        .ws
        .write("dup.md", b"dup one\nfiller\ndup one\n")
        .expect("seed write must succeed");
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "dup.md",
                "old_text": "dup one",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "dup.md"),
            }),
        )
        .await
        .expect_err("multiple matches must refuse");
    let text = error.to_string();
    assert!(
        !text.contains("dup one"),
        "an ambiguity refusal reports lines, not excerpts: {text}"
    );
    // The sentinel file itself is untouched.
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed.as_bytes(),
        "a refused edit leaves the file byte-identical"
    );
}
