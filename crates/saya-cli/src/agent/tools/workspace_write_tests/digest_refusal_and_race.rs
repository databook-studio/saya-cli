/// A same-size overwrite between the model's read and its write is caught by
/// the digest: the stale digest refuses and the file keeps the other
/// writer's bytes — never last-writer-wins.
#[tokio::test]
async fn write_with_a_stale_same_size_digest_is_refused() {
    let sandbox = Sandbox::new("stale-digest");
    let tools = sandbox.tools();
    tools
        .execute(
            "write",
            serde_json::json!({"path": "stale.txt", "content": "hello world"}),
        )
        .await
        .expect("the creating write must succeed");
    let observed = tools
        .execute("read", serde_json::json!({"path": "stale.txt"}))
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
            "write",
            serde_json::json!({
                "path": "stale.txt",
                "content": "clobber",
                "expected_digest": observed["digest"],
            }),
        )
        .await
        .expect_err("a stale digest must refuse the replace");
    let message = error.to_string();
    assert!(
        message.contains("changed") && message.contains("current_digest"),
        "the refusal must carry the state the model retries from: {message}"
    );
    assert!(
        !message.contains("hello world") && !message.contains("hell0 world"),
        "the refusal must not carry file content: {message}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("stale.txt")).expect("file must survive"),
        b"hell0 world",
        "the other writer's bytes must survive a refused stale replace"
    );
}

/// After an external change the read witness refuses as stale, and the
/// refusal carries the file's current digest so the model retries with it
/// instead of spending a read turn first.
#[tokio::test]
async fn a_stale_write_refusal_carries_the_current_digest() {
    let sandbox = Sandbox::new("stale-witness-digest");
    let tools = sandbox.tools();
    tools
        .execute(
            "write",
            serde_json::json!({"path": "stale.txt", "content": "hello world"}),
        )
        .await
        .expect("the creating write must succeed");
    tools
        .execute("read", serde_json::json!({"path": "stale.txt"}))
        .await
        .expect("the file must read back");
    // A same-size change the model never saw: the witness it holds is stale.
    sandbox
        .ws
        .write("stale.txt", b"hell0 world")
        .expect("the external same-size overwrite must land");
    let current = sandbox
        .ws
        .read("stale.txt", 0)
        .expect("the current state must read")
        .digest;

    let error = tools
        .execute(
            "write",
            serde_json::json!({"path": "stale.txt", "content": "clobber"}),
        )
        .await
        .expect_err("a stale witness must refuse the replace");
    let text = error.to_string();
    assert!(
        text.contains("current_digest") && text.contains(&current),
        "the refusal must hand back the current digest for the retry: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("stale.txt")).expect("the file must survive"),
        b"hell0 world",
        "the other writer's bytes must survive a refused stale replace"
    );
}

/// A no-digest write over a file this run never read refuses with the
/// file's current size and digest, so the model retries with the state it
/// was just handed rather than spending a read turn.
#[tokio::test]
async fn a_missing_digest_write_refusal_names_the_current_state() {
    let sandbox = Sandbox::new("missing-digest-state");
    let tools = sandbox.tools();
    // Seeded outside the toolset: the run has no read witness for this path.
    sandbox
        .ws
        .write("notes.md", b"hello world")
        .expect("the seed write must succeed");
    let current = sandbox
        .ws
        .read("notes.md", 0)
        .expect("the current state must read")
        .digest;

    let error = tools
        .execute(
            "write",
            serde_json::json!({"path": "notes.md", "content": "clobber"}),
        )
        .await
        .expect_err("a no-digest write over an existing file must refuse");
    let text = error.to_string();
    assert!(
        text.contains("current_digest") && text.contains(&current),
        "the refusal must name the current size and digest: {text}"
    );
    assert!(
        text.contains("11"),
        "the refusal must name the current size: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("the file must survive"),
        b"hello world"
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
            "write",
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
            "write",
            serde_json::json!({"path": "race.txt", "content": "loser bytes"}),
        )
        .await
        .expect_err("a no-digest write over an existing file must refuse");
    assert!(matches!(error, ToolError::WorkspaceDigestRequired { .. }));
    assert_eq!(
        fs::read(sandbox.ws_root().join("race.txt")).expect("file must survive"),
        b"winner bytes",
        "the concurrent creator's bytes must survive"
    );
}
