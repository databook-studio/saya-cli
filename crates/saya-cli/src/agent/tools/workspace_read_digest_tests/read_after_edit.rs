use super::*;

/// A read of a file whose current digest is the one this run's last edit
/// produced still returns the content as usual, and carries a one-line note.
/// The note is a hint, never a refusal: the model asked to read, and it reads.
#[tokio::test]
async fn a_read_after_this_runs_edit_says_the_file_is_unchanged() {
    let sandbox = Sandbox::new("note-after-edit");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let read = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("a contained read must succeed");
    let digest = read["digest"]
        .as_str()
        .expect("the read result carries a digest")
        .to_owned();
    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the digest from the fresh read guards the edit");

    let after = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("a read after this run's edit still returns the content");
    assert_eq!(
        after["note"], "unchanged since your last edit",
        "the read names the state this run produced: {after}"
    );
    assert!(
        after["content"]
            .as_str()
            .is_some_and(|content| content.contains("hello ANCHOR world")),
        "the content is served as usual: {after}"
    );
}

/// The note is about this run's own last edit. A path this run never mutated,
/// or one someone else changed after the edit, carries no note.
#[tokio::test]
async fn a_read_without_this_runs_edit_carries_no_note() {
    let sandbox = Sandbox::new("note-absent");
    sandbox
        .ws
        .write("notes.md", b"hello world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let read = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("a contained read must succeed");
    assert!(
        read.get("note").is_none(),
        "a read of a path this run never wrote carries no note: {read}"
    );

    // A change made outside the tools is not this run's edit.
    sandbox
        .ws
        .write("notes.md", b"changed outside the run\n")
        .expect("the foreign write must succeed");
    let after = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the foreign state is readable");
    assert!(
        after.get("note").is_none(),
        "a foreign change is not this run's edit: {after}"
    );
}
