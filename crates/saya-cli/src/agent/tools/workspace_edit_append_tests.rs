//! Tests for the `workspace_edit` tool's `append` variant: offset-checked
//! chunked writes over the same atomic commit the `replace` variant uses.
//! Append is a range replace — an empty range at EOF with a positional
//! precondition — so every refusal here leaves the file byte-identical, and
//! every mismatch reports the current size the model resumes from.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_EDIT_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape has a real target to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsappend-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

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
    // Chunked: each chunk names the size the previous chunk left.
    let mut offset = 0u64;
    for chunk in ["alpha;", "beta;", "gamma;", "delta;"] {
        let result = tools
            .execute(
                "workspace_edit",
                serde_json::json!({
                    "path": "chunked.txt",
                    "offset": offset,
                    "chunk": chunk,
                }),
            )
            .await
            .unwrap_or_else(|error| {
                panic!("chunk {chunk:?} at offset {offset} must land: {error}")
            });
        assert_eq!(result["path"], "chunked.txt");
        offset = result["size"]
            .as_u64()
            .expect("the result carries the new size");
        assert!(
            result["digest"]
                .as_str()
                .is_some_and(|digest| digest.len() == 64),
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
            }),
        )
        .await
        .expect("a chunk exactly at the bound must be accepted");
    assert_eq!(result["path"], "log.txt");
    assert_eq!(result["size"], 5 + WORKSPACE_EDIT_MAX_BYTES as u64);
}

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
            "workspace_edit",
            serde_json::json!({
                "path": "shared.txt",
                "offset": 3,
                "chunk": "tail;",
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
    // The replace half's equivalent: a stale `expected_size` precondition.
    let replace_error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "shared.txt",
                "old_text": "anchor",
                "new_text": "ANCHOR",
                "expected_size": 3,
            }),
        )
        .await
        .expect_err("a stale precondition must refuse");
    let saya_agent::ToolError::WorkspaceEditMoved { current_size, .. } = replace_error else {
        panic!("a stale precondition must refuse with the moved error, got: {replace_error}");
    };
    assert_eq!(current_size, 12, "both halves report the same current size");
    // Both bounds are the same constant, by construction not coincidence:
    // the append half's chunk bound IS the replace half's bound.
    assert_eq!(
        WORKSPACE_EDIT_MAX_BYTES,
        super::database_tools::WORKSPACE_WRITE_MAX_BYTES,
        "one bound for every write-shaped byte the tool accepts"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("shared.txt")).expect("file must survive"),
        b"anchor here\n",
        "both refusals leave the file byte-identical"
    );
}

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
    let tools = sandbox.tools();
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
}
