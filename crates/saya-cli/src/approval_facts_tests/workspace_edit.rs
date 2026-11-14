use super::*;

/// The consent surface states the write contract the executor enforces:
/// create-if-absent, and a replace only behind the digest the model last
/// observed — never a "replaced whole" claim over an existing file.
#[test]
fn a_workspace_write_ask_states_the_digest_contract_not_a_whole_replace() {
    let facts = session_facts();
    let tool = session_tool("write");
    let arguments = serde_json::json!({"path": "notes/summary.md", "content": "hello world"});
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        !prompt.contains("replaced whole"),
        "an existing file is not replaced whole — the ask must not say so: {prompt}"
    );
    assert!(
        prompt.contains("create-if-absent"),
        "the ask names the create-if-absent contract: {prompt}"
    );
}

/// `edit` states the path and the edit form's byte sizes — never
/// the anchored bodies or the digests (the prompt states sizes only).
#[test]
fn workspace_edit_facts_state_sizes_never_bodies() {
    let facts = session_facts();
    let replace = serde_json::json!({
        "path": "notes/summary.md",
        "old_text": "ANCHOR-OLD-9f3",
        "new_text": "ANCHOR-NEW-7b2",
        "expected_digest": "sha256:cafe0123456789",
    });
    let rendered = call_facts("edit", &replace, None, &facts, None, None)
        .expect("an anchored replace has approval facts");
    assert!(
        rendered.contains("edit — edits a file in this session's workspace"),
        "the header names the tool: {rendered}"
    );
    assert!(
        rendered.contains("path: notes/summary.md"),
        "the path is stated: {rendered}"
    );
    assert!(
        rendered.contains("replace one anchored string"),
        "the form names the replace shape: {rendered}"
    );
    assert!(
        rendered.contains("old_text 14 bytes") && rendered.contains("new_text 14 bytes"),
        "the sizes are stated, never the bodies: {rendered}"
    );
    for secret in ["ANCHOR-OLD-9f3", "ANCHOR-NEW-7b2", "sha256:cafe0123456789"] {
        assert!(
            !rendered.contains(secret),
            "no body or digest text: {rendered}"
        );
    }
    let append = serde_json::json!({
        "path": "notes/summary.md",
        "offset": 42u64,
        "chunk": "CHUNK-BODY-3c8",
    });
    let rendered = call_facts("edit", &append, None, &facts, None, None)
        .expect("an append has approval facts");
    assert!(
        rendered.contains("append 14 bytes at offset 42"),
        "the form names the append shape with size and offset: {rendered}"
    );
    assert!(
        !rendered.contains("CHUNK-BODY-3c8"),
        "the chunk body never prints: {rendered}"
    );
}

/// A `replace_all` call is approved as what it is: the operator is told the
/// one anchor is replaced everywhere, not at one site.
#[test]
fn workspace_edit_facts_name_the_replace_all_scope() {
    let facts = session_facts();
    let arguments = serde_json::json!({
        "path": "notes/summary.md",
        "old_text": "ANCHOR-OLD-9f3",
        "new_text": "ANCHOR-NEW-7b2",
        "replace_all": true,
        "expected_digest": "sha256:cafe0123456789",
    });
    let rendered = call_facts("edit", &arguments, None, &facts, None, None)
        .expect("a replace_all call has approval facts");
    assert!(
        rendered.contains("replace every occurrence of one anchored string"),
        "the ask states the whole-file scope: {rendered}"
    );
    assert!(
        !rendered.contains("ANCHOR-OLD-9f3") && !rendered.contains("ANCHOR-NEW-7b2"),
        "the scope line states sizes only, never bodies: {rendered}"
    );
}

/// The arguments are model-controlled, so the facts are total: a missing,
/// null, non-string, or multi-byte value never panics and never prints a
/// placeholder — the block degrades to what the call demonstrates.
#[test]
fn workspace_edit_facts_are_total_over_degenerate_arguments() {
    let facts = session_facts();
    for arguments in [
        serde_json::json!({}),
        serde_json::json!({"path": 7}),
        serde_json::json!({"path": null}),
        serde_json::json!({"old_text": 5}),
    ] {
        assert_eq!(
            call_facts("edit", &arguments, None, &facts, None, None),
            None,
            "a call that demonstrates no path shows no facts: {arguments}"
        );
    }
    for arguments in [
        serde_json::json!({"path": "notes.md", "old_text": 5}),
        serde_json::json!({"path": "notes.md", "old_text": "a", "new_text": null}),
        serde_json::json!({"path": "notes.md", "offset": 7}),
        serde_json::json!({"path": "notes.md", "chunk": "abc"}),
        serde_json::json!({"path": "notes.md"}),
    ] {
        let rendered = call_facts("edit", &arguments, None, &facts, None, None)
            .expect("a path names the block");
        assert!(
            rendered.contains("path: notes.md"),
            "the path line stands: {rendered}"
        );
        assert!(
            !rendered.contains("replace one anchored string") && !rendered.contains("append "),
            "a degenerate form omits the line, never a placeholder: {rendered}"
        );
    }
    let unicode = serde_json::json!({
        "path": "not\r\nés/ \n notes.md",
        "old_text": "☃️",
        "new_text": "\u{1F600}",
        "expected_digest": "d",
    });
    let rendered = call_facts("edit", &unicode, None, &facts, None, None)
        .expect("multi-byte values are counted as bytes, never panicked on");
    assert!(
        rendered.contains("path: not és/ notes.md"),
        "the collapsed path: {rendered}"
    );
    assert!(
        rendered.contains("old_text 6 bytes"),
        "the snowman plus variation selector is six bytes: {rendered}"
    );
    assert!(
        rendered.contains("new_text 4 bytes"),
        "the emoji is four bytes: {rendered}"
    );
    let big = "x".repeat(1024 * 1024);
    let huge = serde_json::json!({
        "path": "notes.md",
        "old_text": "a",
        "new_text": big,
        "expected_digest": "d",
    });
    let rendered = call_facts("edit", &huge, None, &facts, None, None)
        .expect("a 1 MiB new_text is stated as a size");
    assert!(
        rendered.contains("new_text 1048576 bytes"),
        "the size is stated: {rendered}"
    );
    assert!(
        !rendered.contains("xxxx"),
        "the body never prints: {rendered}"
    );
}

/// A `edit` ask states the path and sizes and offers the same
/// `[s]` scope sentence as the write tool — one grant vocabulary for both
/// guarded file tools.
#[test]
fn a_workspace_edit_ask_states_the_path_and_the_same_write_scope() {
    let facts = session_facts();
    let tool = session_tool("edit");
    let arguments = serde_json::json!({
        "path": "notes/summary.md",
        "old_text": "ANCHOR-OLD-9f3",
        "new_text": "ANCHOR-NEW-7b2",
        "expected_digest": "sha256:cafe0123456789",
    });
    let grant = grant_token(&tool.name, &arguments, None, &facts);
    assert_eq!(
        grant.as_deref(),
        Some("workspace-write"),
        "one token for both file tools"
    );
    let prompt = approval_prompt(&tool, &arguments, grant.as_deref(), &facts, None, None);
    assert!(
        prompt.contains("path: notes/summary.md"),
        "the path is stated: {prompt}"
    );
    assert!(
        prompt.contains("old_text 14 bytes") && prompt.contains("new_text 14 bytes"),
        "sizes only: {prompt}"
    );
    assert!(
        prompt.contains("covers every workspace write") && prompt.contains("not this path alone"),
        "the [s] scope sentence: {prompt}"
    );
    assert!(
        prompt.contains("[s] allow workspace-write for this session"),
        "the third answer names the token: {prompt}"
    );
    for secret in ["ANCHOR-OLD-9f3", "ANCHOR-NEW-7b2", "sha256:cafe0123456789"] {
        assert!(!prompt.contains(secret), "no body or digest: {prompt}");
    }
}
