use super::*;

/// Append and replace share one write path, asserted structurally, not by
/// comment: a stale offset against a moved file refuses with the same typed
/// shape the replace half's precondition race produces — the harness's own
/// size check re-surfaced with the current size — because both commit
/// through `commit_splice`/`Workspace::patch_range`. A second write path
/// would refuse differently (or succeed); this pins that it cannot.
#[tokio::test]
async fn append_and_replace_share_one_write_path() {
    let sandbox = Sandbox::new("one-path");
    sandbox
        .ws
        .write("shared.txt", b"anchor here\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    // The append half's everyday mismatch: offset names a size the file no
    // longer has.
    let append_error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "shared.txt",
                "offset": 3,
                "chunk": "tail;",
                "expected_digest": observed_digest(&sandbox, "shared.txt"),
            }),
        )
        .await
        .expect_err("a stale offset must refuse");
    let saya_agent::ToolError::WorkspaceAppendOffset {
        expected_offset,
        current_size,
        current_digest,
        ..
    } = append_error
    else {
        panic!("a stale offset must refuse with the typed offset error, got: {append_error}");
    };
    assert_eq!(expected_offset, 3);
    assert_eq!(current_size, 12);
    assert_eq!(current_digest.len(), 64);
    // The replace half's equivalent refusal: a stale digest whose anchor
    // matches nothing on the current content.
    let replace_error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "shared.txt",
                "old_text": "missing",
                "new_text": "MISSING",
                "expected_size": 3,
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("a stale digest with no matching anchor must refuse");
    let saya_agent::ToolError::WorkspaceEditMoved { current_size, .. } = replace_error else {
        panic!("a stale precondition must refuse with the moved error, got: {replace_error}");
    };
    assert_eq!(current_size, 12, "both halves report the same current size");
    // The append half's chunk bound IS the replace half's bound: both are
    // the harness patch layer's replacement bound, by construction not
    // coincidence. It is deliberately not the whole-file write bound,
    // which a single `write` call carries.
    assert_eq!(
        WORKSPACE_EDIT_MAX_BYTES,
        saya_harness::workspace::patch::PATCH_REPLACEMENT_MAX_BYTES,
        "one bound for every anchored or appended byte the tool accepts"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("shared.txt")).expect("file must survive"),
        b"anchor here\n",
        "both refusals leave the file byte-identical"
    );
}

/// An append to an existing file without `expected_digest` refuses and
/// changes nothing: the typed message names the read-first remedy
/// (`expected_digest`, `read`) and carries the file's current size and
/// digest so the model retries without a separate read. The check
/// precedes the offset check — a wrong offset without a digest gets the
/// same read-first refusal, not an offset payload.
#[tokio::test]
async fn an_append_to_an_existing_file_without_a_digest_changes_nothing() {
    let sandbox = Sandbox::new("append-no-digest");
    let seed = b"chunk-one;";
    sandbox
        .ws
        .write("log.txt", seed)
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("log.txt")).expect("seed file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": seed.len() as u64,
                "chunk": "chunk-two;",
            }),
        )
        .await
        .expect_err("appending to an existing file without a digest must refuse");
    let message = error.to_string();
    assert!(
        message.contains("expected_digest") && message.contains("read"),
        "the refusal must name the read-first remedy: {message}"
    );
    let real = observed_digest(&sandbox, "log.txt");
    assert!(
        message.contains("current_digest") && message.contains(&real),
        "the refusal must hand back the file's current digest so the model retries \
         without a re-read (D6): {message}"
    );
    // The read-first check precedes the offset check: a wrong offset with
    // no digest gets the same message, not an offset refusal.
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 0,
                "chunk": "chunk-two;",
            }),
        )
        .await
        .expect_err("a wrong offset without a digest still refuses read-first");
    assert!(
        matches!(error, saya_agent::ToolError::WorkspaceDigestRequired { .. }),
        "the wrong-offset call refuses read-first, not with an offset payload: {error}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        before,
        "a refused append leaves the file byte-identical"
    );
}

/// A stale same-size digest refuses the append: the model observed one
/// state, the file changed (same size) before the append, and the digest
/// precondition refuses as the typed moved error — the file is never
/// extended from a stale anchor.
#[tokio::test]
async fn an_append_with_a_stale_same_size_digest_is_refused() {
    let sandbox = Sandbox::new("append-stale-digest");
    sandbox
        .ws
        .write("log.txt", b"hello world")
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "log.txt");
    // Same-size (11 bytes) overwrite between the observation and the append.
    sandbox
        .ws
        .write("log.txt", b"HELLO WORLD")
        .expect("overwrite must write");
    let before = fs::read(sandbox.ws_root().join("log.txt")).expect("file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 11u64,
                "chunk": "tail;",
                "expected_digest": observed,
            }),
        )
        .await
        .expect_err("a stale same-size digest must refuse the append");
    let saya_agent::ToolError::WorkspaceEditMoved { current_size, .. } = error else {
        panic!("a stale digest must refuse with the typed moved error, got: {error}");
    };
    assert_eq!(current_size, 11);
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        before,
        "a refused append leaves the file byte-identical"
    );
}

/// The digest observed from a fresh read admits the append: the file is
/// extended by exactly the chunk, and the result reports the new size and
/// digest so the next turn resumes from what landed.
#[tokio::test]
async fn an_append_with_the_observed_digest_extends_the_file() {
    let sandbox = Sandbox::new("append-with-digest");
    let seed = b"chunk-one;";
    sandbox
        .ws
        .write("log.txt", seed)
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "log.txt");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": seed.len() as u64,
                "chunk": "chunk-two;",
                "expected_digest": observed,
            }),
        )
        .await
        .expect("the observed digest must admit the append");
    assert_eq!(result["path"], "log.txt");
    assert_eq!(result["size"], (seed.len() + "chunk-two;".len()) as u64);
    assert!(
        result["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the result carries the digest of what landed"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must exist"),
        b"chunk-one;chunk-two;",
        "the append extends the file by exactly the chunk"
    );
}
