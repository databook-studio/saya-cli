use super::*;

/// The destination-semantics line states replacement only when the call
/// itself names a valid-looking digest: with one present, the prompt says
/// the destination may be replaced only if its current content matches the
/// digest the caller observed — and the create-only line is absent.
#[test]
fn the_download_prompt_states_replacement_only_with_a_valid_digest() {
    let facts = session_facts();
    let arguments = serde_json::json!({
        "url": "https://example.com/model.bin",
        "destination": "artifacts/model.bin",
        "expected_digest": "a".repeat(64),
    });
    let rendered = call_facts("http_download", &arguments, None, &facts, None, None)
        .expect("http_download has approval facts");
    assert!(
        rendered.contains(
            "replaces the destination only if its current content matches the digest you observed"
        ),
        "the replacement line with a digest: {rendered}"
    );
    assert!(
        !rendered.contains("creates the destination only"),
        "no create-only line next to a digest: {rendered}"
    );
}

/// Without a digest argument the prompt states the create-only form: the
/// destination is created only, and an existing file is refused.
#[test]
fn the_download_prompt_states_create_only_without_a_digest() {
    let facts = session_facts();
    let arguments = serde_json::json!({
        "url": "https://example.com/model.bin",
        "destination": "artifacts/model.bin",
    });
    let rendered = call_facts("http_download", &arguments, None, &facts, None, None)
        .expect("http_download has approval facts");
    assert!(
        rendered.contains("creates the destination only; an existing file is refused"),
        "the create-only line without a digest: {rendered}"
    );
    assert!(
        !rendered.contains("replaces the destination"),
        "no replacement line without a digest: {rendered}"
    );
}

/// A present-but-unusable digest (the call will refuse before transport)
/// demonstrates neither replacement nor creation, so the prompt states
/// neither — a prompt may not claim a bound the code does not apply.
#[test]
fn the_download_prompt_states_no_destination_semantics_for_a_malformed_digest() {
    let facts = session_facts();
    let arguments = serde_json::json!({
        "url": "https://example.com/model.bin",
        "destination": "artifacts/model.bin",
        "expected_digest": "not-a-digest",
    });
    let rendered = call_facts("http_download", &arguments, None, &facts, None, None)
        .expect("http_download has approval facts");
    assert!(
        !rendered.contains("creates the destination only"),
        "no create-only claim for a call that refuses: {rendered}"
    );
    assert!(
        !rendered.contains("replaces the destination"),
        "no replacement claim for a call that refuses: {rendered}"
    );
}

/// The session lane's `http_download` definition advertises the optional
/// `expected_digest` argument with the same name the guarded file tools
/// use, the required list unchanged, and the description carrying the
/// create-only / observed-digest guidance.
#[test]
fn the_session_http_download_definition_advertises_the_optional_digest() {
    let tool = session_tool("http_download");
    let properties = tool.parameters["properties"]
        .as_object()
        .expect("the schema has properties");
    let declared = properties
        .get("expected_digest")
        .expect("expected_digest is declared");
    assert_eq!(declared["type"], "string", "the digest is a string");
    let description = declared["description"].as_str().expect("described");
    assert!(
        description.contains("never invent or guess"),
        "the description forbids invented digests: {description}"
    );
    assert!(
        description.contains("read it first"),
        "the description requires observing the digest from a read: {description}"
    );
    assert_eq!(
        tool.parameters["required"],
        serde_json::json!(["url", "destination"]),
        "the required list is unchanged: the digest stays optional"
    );
    assert_eq!(
        tool.parameters["additionalProperties"],
        serde_json::json!(false),
        "the schema stays closed"
    );
}
