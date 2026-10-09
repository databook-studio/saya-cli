use super::*;

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
