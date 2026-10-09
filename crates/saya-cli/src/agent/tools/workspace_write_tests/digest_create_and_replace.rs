/// A no-digest write to an absent path creates the file, and the result now
/// carries `size` and the whole-file `digest` — the value a later guarded
/// replace or edit chains from.
#[tokio::test]
async fn write_creates_a_new_file_and_returns_size_and_digest() {
    let sandbox = Sandbox::new("create-new");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes/new.md", "content": "fresh bytes"}),
        )
        .await
        .expect("a no-digest write to an absent path must create the file");
    assert_eq!(result["size"], 11);
    let digest = result["digest"].as_str().expect("digest must be a string");
    assert_eq!(digest.len(), 64, "the digest must be sha256 hex");
    assert!(
        digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase()),
        "the digest must be lowercase hex: {digest}"
    );
    let observed = tools
        .execute(
            "workspace_read",
            serde_json::json!({"path": "notes/new.md"}),
        )
        .await
        .expect("the file must read back");
    assert_eq!(
        result["digest"], observed["digest"],
        "the result's digest must be the file's real digest"
    );
    assert_eq!(result["size"], observed["size"]);
}

/// The defining refusal: writing over an existing file without a digest
/// changes nothing, and the refusal tells the model how to proceed —
/// `expected_digest` from a fresh `workspace_read` — without leaking the
/// file's current digest or content.
#[tokio::test]
async fn write_over_an_existing_file_without_a_digest_changes_nothing() {
    let sandbox = Sandbox::new("no-digest-replace");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "original secret bytes"}),
        )
        .await
        .expect("the creating write must succeed");
    let observed = tools
        .execute("workspace_read", serde_json::json!({"path": "report.md"}))
        .await
        .expect("the file must read back");
    let current_digest = observed["digest"].as_str().expect("a digest").to_owned();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "clobbered"}),
        )
        .await
        .expect_err("replacing an existing file without a digest must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("expected_digest") && message.contains("workspace_read"),
                "the refusal must tell the model how to replace: {message}"
            );
            assert!(
                !message.contains(&current_digest),
                "the refusal must not leak the file's current digest: {message}"
            );
            assert!(
                !message.contains("original secret bytes"),
                "the refusal must not leak the file's content: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.md")).expect("file must survive"),
        b"original secret bytes",
        "the file must be byte-identical after the refused replace"
    );
}

/// With the digest the model observed the file is replaced, and the result
/// names the new state so the model can chain the next guarded edit.
#[tokio::test]
async fn write_with_the_observed_digest_replaces_the_file() {
    let sandbox = Sandbox::new("guarded-replace");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "report.md", "content": "first version"}),
        )
        .await
        .expect("the creating write must succeed");
    let observed = tools
        .execute("workspace_read", serde_json::json!({"path": "report.md"}))
        .await
        .expect("the file must read back");
    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "report.md",
                "content": "second version",
                "expected_digest": observed["digest"],
            }),
        )
        .await
        .expect("a digest-guarded replace of an unchanged file must succeed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("report.md")).expect("file must exist"),
        b"second version",
    );
    let reread = tools
        .execute("workspace_read", serde_json::json!({"path": "report.md"}))
        .await
        .expect("the file must read back");
    assert_eq!(
        result["digest"], reread["digest"],
        "the result's digest must be what landed"
    );
    assert_eq!(result["size"], reread["size"]);
}
