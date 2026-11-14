/// Replacing an existing file without `expected_digest` — and with no earlier
/// read of it in this run — changes nothing, and the refusal names the file's
/// current size and digest so the model retries without a separate read.
#[tokio::test]
async fn a_replace_without_a_digest_changes_nothing() {
    let sandbox = Sandbox::new("no-digest");
    let seed = b"hello workspace\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("notes.md")).expect("seed file must exist");
    let real_digest = observed_digest(&sandbox, "notes.md");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "workspace",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect_err("a replace without a digest must refuse");
    let text = error.to_string();
    assert!(
        text.contains("expected_digest"),
        "the refusal must name the required argument: {text}"
    );
    assert!(
        text.contains("read"),
        "the refusal must say where the digest comes from: {text}"
    );
    assert!(
        text.contains("not changed"),
        "the refusal must state the file was not changed: {text}"
    );
    assert!(
        text.contains("current_digest") && text.contains(&real_digest),
        "the refusal must hand the model the current state so it retries without a re-read: {text}"
    );
    assert!(
        text.contains(&seed.len().to_string()),
        "the refusal must name the current size: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        before,
        "a refused edit leaves the file byte-identical"
    );
}

/// A same-size change between the model's observation and the tool call is
/// re-anchored when the anchor still resolves exactly once on the current
/// content: the edit applies, says the file changed, and touches only the
/// intended bytes. Zero or several matches still refuse (see the anchor and
/// moved rows).
#[tokio::test]
async fn a_replace_with_a_stale_same_size_digest_reanchors_a_unique_anchor() {
    let sandbox = Sandbox::new("stale-same-size");
    sandbox
        .ws
        .write("notes.md", b"hello world")
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "notes.md");
    // Same size (11 bytes), different bytes — a size guard alone cannot see
    // this; the anchor on the current content must.
    sandbox
        .ws
        .write("notes.md", b"HELLO WORLD")
        .expect("same-size overwrite must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "WORLD",
                "new_text": "SPACE!",
                "expected_digest": observed,
            }),
        )
        .await
        .expect("a unique anchor on the current content must re-anchor");
    assert!(
        result["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {result}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"HELLO SPACE!",
        "the edit applies to the current content"
    );
}

/// The happy path of the new precondition: a digest observed from a fresh
/// read lets the replace proceed and change exactly the intended bytes.
#[tokio::test]
async fn a_replace_with_the_observed_digest_changes_the_intended_bytes() {
    let sandbox = Sandbox::new("observed-digest");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "notes.md");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
                "expected_digest": observed,
            }),
        )
        .await
        .expect("the observed digest must admit the replace");
    assert_eq!(result["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"hello ANCHOR world\n",
        "the intended bytes change and nothing else"
    );
}
