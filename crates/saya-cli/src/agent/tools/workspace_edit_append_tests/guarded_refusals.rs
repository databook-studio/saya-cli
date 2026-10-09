use super::*;

/// The guard also applies when appending to an existing file: a chunk that
/// introduces the marker onto a file that had none is refused, and nothing
/// is appended.
#[tokio::test]
async fn append_to_an_existing_file_refuses_a_chunk_with_the_marker() {
    let sandbox = Sandbox::new("append-marker");
    let seed = b"line one;";
    sandbox
        .ws
        .write("log.txt", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": seed.len() as u64,
                "chunk": " leaked: [redacted]",
                "expected_digest": observed_digest(&sandbox, "log.txt"),
            }),
        )
        .await
        .expect_err("an appended chunk holding the marker must be refused");
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        seed,
        "a refused append leaves the file byte-identical"
    );
}

/// The redaction-placeholder guard applies to append-creates-a-new-file the
/// same way it applies to `workspace_write`: a brand-new file starts at 0
/// markers, so a first chunk holding the literal marker is refused and no
/// file is created.
#[tokio::test]
async fn append_creating_a_new_file_with_the_marker_is_refused() {
    let sandbox = Sandbox::new("create-marker");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "fresh.txt",
                "offset": 0,
                "chunk": "leaked: [redacted]",
            }),
        )
        .await
        .expect_err("a new file's first chunk holding the marker must be refused");
    assert!(
        !sandbox.ws_root().join("fresh.txt").exists(),
        "a refused append must leave no file behind"
    );
}

/// A chunk over the bound refuses whole — never truncated — and leaves no
/// partial append behind.
#[tokio::test]
async fn an_over_bound_chunk_refuses_whole() {
    let sandbox = Sandbox::new("over-bound");
    sandbox
        .ws
        .write("log.txt", b"seed;")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let oversized = "a".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 5,
                "chunk": oversized,
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
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        b"seed;",
        "a refused append leaves the file byte-identical"
    );
    // The boundary case: exactly the bound is a legal chunk.
    let exact = "a".repeat(WORKSPACE_EDIT_MAX_BYTES);
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 5,
                "chunk": exact,
                "expected_digest": observed_digest(&sandbox, "log.txt"),
            }),
        )
        .await
        .expect("a chunk exactly at the bound must be accepted");
    assert_eq!(result["path"], "log.txt");
    assert_eq!(result["size"], 5 + WORKSPACE_EDIT_MAX_BYTES as u64);
}

/// H1 reproduction (must fail before the fix): an existing but unreadable
/// file appended at offset 0 must refuse and leave the bytes byte-identical.
/// The probe must not classify a permission refusal as absence and take the
/// create branch.
#[cfg(unix)]
#[tokio::test]
async fn append_to_an_unreadable_file_refuses_and_preserves_bytes() {
    use std::os::unix::fs::PermissionsExt as _;
    let sandbox = Sandbox::new("h1-unreadable");
    let target = sandbox.ws_root().join("secret.txt");
    sandbox
        .ws
        .write("secret.txt", b"do-not-destroy;")
        .expect("seed write must succeed");
    let before = fs::read(&target).expect("seed file must exist");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o000)).expect("chmod 000 must apply");
    let tools = sandbox.tools();
    let outcome = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "secret.txt",
                "offset": 0,
                "chunk": "attacker bytes;",
            }),
        )
        .await;
    fs::set_permissions(&target, fs::Permissions::from_mode(0o644))
        .expect("chmod restore must apply");
    assert!(
        outcome.is_err(),
        "appending at offset 0 to an existing but unreadable file must refuse, not create"
    );
    assert_eq!(
        fs::read(&target).expect("file must survive"),
        before,
        "a refused append leaves the file byte-identical"
    );
}
