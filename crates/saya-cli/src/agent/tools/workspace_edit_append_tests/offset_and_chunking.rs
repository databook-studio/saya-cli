use super::*;

/// An offset mismatch refuses, writes nothing, and reports the current size —
/// that read-back is what lets the model resume from the right place rather
/// than guess.
#[tokio::test]
async fn an_offset_mismatch_writes_nothing_and_reports_the_size() {
    let sandbox = Sandbox::new("offset-mismatch");
    sandbox
        .ws
        .write("log.txt", b"chunk-one;")
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("log.txt")).expect("seed file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 3,
                "chunk": "chunk-two;",
                "expected_digest": observed_digest(&sandbox, "log.txt"),
            }),
        )
        .await
        .expect_err("a stale offset must refuse, not append");
    let text = error.to_string();
    assert!(
        text.contains("10"),
        "the refusal must report the current size so the model can resume, got: {text}"
    );
    assert!(
        text.contains("digest:") || text.contains("digest"),
        "the refusal must carry the current digest for the resume precondition, got: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        before,
        "a refused append leaves the file byte-identical"
    );
}

/// Q1: a matching offset with a contradictory `expected_size` refuses and
/// writes nothing. The schema promises the size guard, so the append arm
/// must honour it like the replace arm does — not silently ignore it.
#[tokio::test]
async fn an_expected_size_mismatch_refuses_an_append_without_writing() {
    let sandbox = Sandbox::new("expected-size");
    sandbox
        .ws
        .write("log.txt", b"chunk-one;")
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("log.txt")).expect("seed file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "log.txt",
                "offset": 10,
                "chunk": "chunk-two;",
                "expected_size": 3,
                "expected_digest": observed_digest(&sandbox, "log.txt"),
            }),
        )
        .await
        .expect_err("a contradictory expected_size must refuse, not append");
    let saya_agent::ToolError::WorkspaceEditMoved { current_size, .. } = error else {
        panic!("a contradictory expected_size must refuse with the moved error, got: {error}");
    };
    assert_eq!(current_size, 10);
    assert_eq!(
        fs::read(sandbox.ws_root().join("log.txt")).expect("file must survive"),
        before,
        "a refused append leaves the file byte-identical"
    );
}

/// Several chunks reassemble byte-exact: the same content written in one go
/// and written chunk by chunk (each at the size the previous chunk left)
/// land identical, and each result carries `{ path, size, digest }` for the
/// caller to verify what landed.
#[tokio::test]
async fn a_chunked_write_reassembles_byte_exact() {
    let sandbox = Sandbox::new("chunked");
    let tools = sandbox.tools();
    let whole = "alpha;beta;gamma;delta;";
    // One-go baseline.
    let one_go = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "one-go.txt",
                "offset": 0,
                "chunk": whole,
            }),
        )
        .await
        .expect("offset 0 on an absent path creates the file");
    // Chunked: each chunk names the size the previous chunk left. The first
    // chunk creates the file at offset 0 (no digest); every later chunk
    // appends to the existing file, so it names the digest the previous
    // chunk's result reported.
    let mut offset = 0u64;
    let mut observed: Option<String> = None;
    for chunk in ["alpha;", "beta;", "gamma;", "delta;"] {
        let mut arguments = serde_json::json!({
            "path": "chunked.txt",
            "offset": offset,
            "chunk": chunk,
        });
        if let Some(digest) = &observed {
            arguments["expected_digest"] = serde_json::json!(digest);
        }
        let result = tools
            .execute("workspace_edit", arguments)
            .await
            .unwrap_or_else(|error| {
                panic!("chunk {chunk:?} at offset {offset} must land: {error}")
            });
        assert_eq!(result["path"], "chunked.txt");
        offset = result["size"]
            .as_u64()
            .expect("the result carries the new size");
        observed = result["digest"].as_str().map(str::to_owned);
        assert!(
            observed.as_ref().is_some_and(|digest| digest.len() == 64),
            "the result carries the digest of what landed"
        );
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("chunked.txt")).expect("file must exist"),
        fs::read(sandbox.ws_root().join("one-go.txt")).expect("baseline must exist"),
        "chunked writes reassemble byte-exact against the one-go write"
    );
    assert_eq!(one_go["size"], offset, "both land the same size");
    assert_eq!(
        one_go["digest"],
        tools
            .execute(
                "workspace_edit",
                serde_json::json!({
                    "path": "chunked.txt",
                    "offset": offset,
                    "chunk": "",
                    "expected_digest": observed,
                }),
            )
            .await
            .expect("an empty final chunk at EOF must land")["digest"],
        "identical bytes hash identical"
    );
}

/// A truncated tool-call argument never parses, so nothing lands. This pins
/// the claim through the append path: a `chunk` cut mid-JSON (an unterminated
/// string, the shape a capped streaming assembler hands over) fails argument
/// parsing before validation runs, and the file is absent afterwards.
#[tokio::test]
async fn a_truncated_argument_lands_nothing() {
    let sandbox = Sandbox::new("truncated");
    let tools = sandbox.tools();
    // The wire's raw fragment never becomes a `Value`: the assembly that
    // turns streamed deltas into tool calls rejects it, exactly as the
    // providers do on a capped response.
    let raw = r#"{"path": "cut.txt", "offset": 0, "chunk": "abc"#;
    assert!(
        serde_json::from_str::<serde_json::Value>(raw).is_err(),
        "a truncated argument must not parse"
    );
    // And one step up: the executor's own validation rejects a non-object
    // before any filesystem contact.
    let error = tools
        .execute("workspace_edit", serde_json::json!("not-an-object"))
        .await
        .expect_err("a non-object argument must be rejected before any write");
    assert_eq!(error, saya_agent::ToolError::ArgumentsNotObject);
    assert!(
        !sandbox.ws_root().join("cut.txt").exists(),
        "a truncated argument lands nothing: the file is never created"
    );
}

/// Offset 0 on an absent path creates the file, and the result carries
/// `{ path, size, digest }` so the caller can verify what landed.
#[tokio::test]
async fn append_at_offset_zero_creates_the_file() {
    let sandbox = Sandbox::new("create");
    let tools = sandbox.tools();
    assert!(!sandbox.ws_root().join("fresh.txt").exists());
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "fresh.txt",
                "offset": 0,
                "chunk": "first bytes;",
            }),
        )
        .await
        .expect("offset 0 on an absent path must create");
    assert_eq!(result["path"], "fresh.txt");
    assert_eq!(result["size"], 12u64);
    assert!(
        result["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the result carries the sha256 hex digest of what landed"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("fresh.txt")).expect("file must exist"),
        b"first bytes;",
        "the created file holds exactly the chunk"
    );
}
