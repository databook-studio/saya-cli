/// A same-size overwrite between the model's read and its write is caught by
/// the digest: the stale digest refuses and the file keeps the other
/// writer's bytes — never last-writer-wins.
#[tokio::test]
async fn write_with_a_stale_same_size_digest_is_refused() {
    let sandbox = Sandbox::new("stale-digest");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "stale.txt", "content": "hello world"}),
        )
        .await
        .expect("the creating write must succeed");
    let observed = tools
        .execute("workspace_read", serde_json::json!({"path": "stale.txt"}))
        .await
        .expect("the file must read back");
    // A non-cooperating writer lands a same-size change a length check
    // cannot see — exactly what the digest precondition exists for.
    sandbox
        .ws
        .write("stale.txt", b"hell0 world")
        .expect("the external same-size overwrite must land");
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "stale.txt",
                "content": "clobber",
                "expected_digest": observed["digest"],
            }),
        )
        .await
        .expect_err("a stale digest must refuse the replace");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("changed") && message.contains("workspace_read"),
                "the refusal must send the model back to a fresh read: {message}"
            );
            assert!(
                !message.contains("hello world") && !message.contains("hell0 world"),
                "the refusal must not carry file content: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("stale.txt")).expect("file must survive"),
        b"hell0 world",
        "the other writer's bytes must survive a refused stale replace"
    );
}

/// A digest-carrying write to an absent path is refused, never a create:
/// `expected_digest` says "replace", and there is nothing to replace.
#[tokio::test]
async fn write_with_a_digest_for_a_missing_file_is_refused_not_created() {
    let sandbox = Sandbox::new("digest-missing");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "notes/missing.md",
                "content": "never lands",
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("a digest-carrying write to a missing file must refuse");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("does not exist") && message.contains("expected_digest"),
                "the refusal must point at the digest-vs-create confusion: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert!(
        !sandbox.ws_root().join("notes").exists(),
        "a refused digest write must create nothing, not even the parent"
    );
}

/// The creation race: another writer wins the path before the tool's
/// no-digest call commits — the tool refuses and the winner's bytes survive,
/// never clobbered. The executor cannot interleave two of its own calls, so
/// the race's winner is planted through the harness's own create-if-absent
/// (`Workspace::create_new`, the same commit gate the tool's create holds);
/// what the test asserts is the tool's side of the race: refuse, never
/// overwrite the winner.
#[tokio::test]
async fn a_creation_race_never_clobbers_the_winner() {
    let sandbox = Sandbox::new("create-race");
    let tools = sandbox.tools();
    sandbox
        .ws
        .create_new("race.txt", b"winner bytes")
        .expect("the concurrent creator must win the race");
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "race.txt", "content": "loser bytes"}),
        )
        .await
        .expect_err("a no-digest write over an existing file must refuse");
    assert!(matches!(error, ToolError::WorkspaceWrite(_)));
    assert_eq!(
        fs::read(sandbox.ws_root().join("race.txt")).expect("file must survive"),
        b"winner bytes",
        "the concurrent creator's bytes must survive"
    );
}
