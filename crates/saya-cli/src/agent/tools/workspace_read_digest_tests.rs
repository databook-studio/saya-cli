//! E4 RED slice: `workspace_read` returns a whole-file digest, and a large
//! file is edited (grep → edit) without any whole-file model read. These
//! tests are written first and MUST FAIL against the current code: there is
//! no `digest` field on the read result yet, and no digest-capable path that
//! serves a digest without serving the whole file.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_READ_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsdigest-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// Database tools with the sandbox workspace attached and no connections.
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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// 1. `workspace_read` returns a digest of the file state it was measured
/// against, so a caller can name that state in `expected_digest`.
#[tokio::test]
async fn a_read_returns_a_digest_that_matches_the_file_state() {
    let sandbox = Sandbox::new("digest-matches");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("workspace_read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("a contained read must succeed");
    let digest = result
        .get("digest")
        .and_then(serde_json::Value::as_str)
        .expect("the read result carries a digest");
    assert_eq!(digest.len(), 64, "the digest is a sha256 hex string");
    assert_eq!(
        digest,
        sha256_hex(b"hello workspace\n"),
        "the digest names the file state the read measured"
    );
}

/// 2. The guarded edit round-trips: the digest from the read satisfies the
/// edit precondition, and a stale digest refuses with no write.
#[tokio::test]
async fn an_edit_guarded_by_that_digest_succeeds_and_a_stale_one_refuses() {
    let sandbox = Sandbox::new("digest-guard");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let read = tools
        .execute("workspace_read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("a contained read must succeed");
    let digest = read["digest"]
        .as_str()
        .expect("the read result carries a digest")
        .to_owned();
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the digest from the fresh read guards the edit");
    assert_eq!(result["path"], "notes.md");
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "ANCHOR",
                "new_text": "anchor",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("the same digest is now stale and must refuse");
    let text = error.to_string();
    assert!(
        text.contains("changed since measured"),
        "the refusal must name the moved anchor: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"hello ANCHOR world\n",
        "a refused edit leaves the file byte-identical"
    );
}

/// 3. A ~1 MiB file is located (the `grep` tool), read (truncated prefix +
/// digest), and edited — while no model-facing call serves the file whole.
/// The absence proof is on real tool payloads: every file byte the model
/// would receive (grep hit lines + read content) is measured from the actual
/// results and must total far less than the file size, and the read must
/// report `truncated: true`. A workflow that served the file whole fails
/// these assertions even though the edit itself succeeds.
#[tokio::test]
async fn a_large_file_is_edited_without_reading_it_whole() {
    let sandbox = Sandbox::new("large-no-whole-read");
    // ~1 MiB of filler with one unique anchor line near the middle.
    let filler = "filler line 0123456789abcdef\n".repeat(36_000);
    let anchor_line = "UNIQUE_ANCHOR_LINE target here\n";
    let mid = filler.len() / 2;
    let seed = format!("{}{}{}", &filler[..mid], anchor_line, &filler[mid..]);
    let file_size = u64::try_from(seed.len()).expect("seed fits");
    assert!(
        file_size > WORKSPACE_READ_MAX_BYTES * 4,
        "the seed must dwarf the read bound"
    );
    sandbox
        .ws
        .write("big.txt", seed.as_bytes())
        .expect("seed write must succeed");

    let tools = sandbox.tools();
    // Locate the anchor through the model-facing search: the file exceeds
    // the read bound, so only a search horizon that covers edit targets can
    // name the line — a capped prefix read never sees it.
    let located = tools
        .execute("grep", serde_json::json!({"pattern": "UNIQUE_ANCHOR_LINE"}))
        .await
        .expect("grep over the large file must succeed");
    assert_eq!(
        located["files_skipped"].as_u64(),
        Some(0),
        "the large file must be searched, not skipped whole"
    );
    assert_eq!(located["files_scanned"].as_u64(), Some(1));
    let hits = located["matches"].as_array().expect("matches array");
    assert_eq!(hits.len(), 1, "the anchor line is located exactly once");
    let hit_bytes = hits[0]["text"].as_str().expect("hit text").len() as u64;

    // The fresh read that produces the guarding digest: truncated prefix
    // plus a whole-file digest — the digest must not cost a whole-file read
    // at the model layer.
    let read = tools
        .execute("workspace_read", serde_json::json!({"path": "big.txt"}))
        .await
        .expect("a bounded read of the large file must succeed");
    assert_eq!(
        read["truncated"],
        serde_json::Value::Bool(true),
        "the large file is never served whole"
    );
    assert_eq!(read["size"].as_u64(), Some(file_size));
    let content_bytes = read["content"]
        .as_str()
        .expect("content must be a string")
        .len() as u64;
    assert_eq!(
        content_bytes, WORKSPACE_READ_MAX_BYTES,
        "the served prefix is capped at exactly the read bound"
    );
    let digest = read["digest"]
        .as_str()
        .expect("the read result carries a digest")
        .to_owned();
    assert_eq!(digest.len(), 64);

    // The edit commits against the digest without serving the file whole:
    // the result carries only size and digest, never content.
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "big.txt",
                "old_text": "UNIQUE_ANCHOR_LINE target here",
                "new_text": "UNIQUE_ANCHOR_LINE edited here",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the digest-guarded edit of the large file must succeed");
    assert_eq!(result["path"], "big.txt");
    assert_eq!(result["size"].as_u64(), Some(file_size));

    // The absence proof: every file byte the model received across the whole
    // workflow — the bounded hit line plus the capped prefix — totals far
    // less than the file. Had any call served the file whole, this fails.
    let served = hit_bytes + content_bytes;
    assert!(
        served < file_size / 2,
        "the workflow must never serve the file whole: served {served} of {file_size} bytes"
    );
    let after = fs::read(sandbox.ws_root().join("big.txt")).expect("file must exist");
    assert!(
        after
            .windows(anchor_line.len())
            .any(|window| window == b"UNIQUE_ANCHOR_LINE edited here\n"),
        "the anchor line was replaced"
    );
}

/// 4. The digest covers the whole file, not the returned slice: two files
/// sharing a prefix but differing past the read bound hash differently.
#[tokio::test]
async fn the_digest_covers_what_its_doc_says_it_covers() {
    let sandbox = Sandbox::new("digest-scope");
    let prefix = vec![b'p'; WORKSPACE_READ_MAX_BYTES as usize];
    let mut first = prefix.clone();
    first.extend_from_slice(b"ending-one");
    let mut second = prefix;
    second.extend_from_slice(b"ending-two");
    sandbox
        .ws
        .write("first.txt", &first)
        .expect("seed write must succeed");
    sandbox
        .ws
        .write("second.txt", &second)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let first_read = tools
        .execute("workspace_read", serde_json::json!({"path": "first.txt"}))
        .await
        .expect("a bounded read must succeed");
    let second_read = tools
        .execute("workspace_read", serde_json::json!({"path": "second.txt"}))
        .await
        .expect("a bounded read must succeed");
    assert_eq!(first_read["content"], second_read["content"]);
    assert_eq!(first_read["truncated"], serde_json::Value::Bool(true));
    let first_digest = first_read["digest"]
        .as_str()
        .expect("the read result carries a digest");
    let second_digest = second_read["digest"]
        .as_str()
        .expect("the read result carries a digest");
    assert_ne!(
        first_digest, second_digest,
        "files sharing the returned prefix but differing past it must hash differently: \
         the digest covers the whole file, not the slice"
    );
    assert_eq!(first_digest, sha256_hex(&first));
}

/// 5. The read bound still behaves as before: an over-bound read is a
/// truncated success — never a refusal — and now still carries the digest.
#[tokio::test]
async fn an_over_bound_read_still_truncates_as_before() {
    let sandbox = Sandbox::new("over-bound");
    let total = WORKSPACE_READ_MAX_BYTES + 16;
    sandbox
        .ws
        .write("big.bin", &vec![b'a'; total as usize])
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("workspace_read", serde_json::json!({"path": "big.bin"}))
        .await
        .expect("a bounded read must succeed");
    assert_eq!(result["truncated"], serde_json::Value::Bool(true));
    assert_eq!(result["size"].as_u64(), Some(total));
    assert!(
        result["digest"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "even a truncated read carries the whole-file digest"
    );
}

/// The definition advertises the digest: the model can learn from the schema
/// that `digest` names the state an `expected_digest` precondition states.
#[test]
fn the_read_definition_advertises_the_digest() {
    let tools = DatabaseTools::definitions(false, false, false, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "workspace_read")
        .expect("workspace_read is advertised even with the data gate closed");
    assert!(
        tool.description.contains("digest"),
        "the description must name the digest so the model knows it exists: {}",
        tool.description
    );
}
