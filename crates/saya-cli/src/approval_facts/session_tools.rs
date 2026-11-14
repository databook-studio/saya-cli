//! The session file tools' fact lines: `write`'s path, bytes, and
//! containment rule; `edit`'s path, edit form, and containment rule.

use serde_json::Value;

use super::{ApprovalFacts, body};

/// `write`'s fact lines. The per-write byte bound and the
/// containment rule are the tool's own enforcement; the path and byte count
/// are the call's.
pub(super) fn workspace_write_facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
    grant: Option<&str>,
) -> Option<String> {
    let path = arguments.get("path").and_then(Value::as_str)?;
    let mut lines = vec![format!(
        "  path: {}",
        crate::agent::tools::collapse_whitespace(path)
    )];
    if let Some(content) = arguments.get("content").and_then(Value::as_str) {
        let bound = crate::agent::tools::WORKSPACE_WRITE_MAX_BYTES;
        lines.push(format!(
            "  bytes: {} · per-write bound: {bound} bytes (over the bound the call refuses \
             whole)",
            content.len()
        ));
    }
    if facts.workspace_root.is_some() {
        lines.push(
            "  containment: absolute paths, `..` escapes, and symlinks are refused · the \
             write is atomic and create-if-absent; replacing an existing file needs the \
             digest of what the model last read (a changed or unread file is refused, \
             never overwritten)"
                .to_string(),
        );
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    // The scope sentence renders only when a session grant is actually on
    // offer — last, closest to the `[s]` answer it describes.
    if let Some(scope) = super::scope::scope_sentence("write", grant) {
        lines.push(scope);
    }
    Some(body(
        "write — writes into this session's workspace".to_string(),
        lines,
    ))
}

/// `edit`'s fact lines: the path, the edit form's byte sizes (an
/// anchored replace or a bounded append), and the containment rule — never
/// the anchored bodies or the digests (the prompt states sizes only).
pub(super) fn workspace_edit_facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
    grant: Option<&str>,
) -> Option<String> {
    let path = arguments.get("path").and_then(Value::as_str)?;
    let mut lines = vec![format!(
        "  path: {}",
        crate::agent::tools::collapse_whitespace(path)
    )];
    if let Some(form) = edit_form_line(arguments) {
        lines.push(form);
    }
    if facts.workspace_root.is_some() {
        lines.push(
            "  containment: absolute paths, `..` escapes, and symlinks are refused · the \
             edit commits atomically behind a fresh-read check — a file changed since \
             the read refuses, never splices"
                .to_string(),
        );
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    // The scope sentence renders only when a session grant is actually on
    // offer — last, closest to the `[s]` answer it describes.
    if let Some(scope) = super::scope::scope_sentence("edit", grant) {
        lines.push(scope);
    }
    Some(body(
        "edit — edits a file in this session's workspace".to_string(),
        lines,
    ))
}

/// The edit form's one line, from the call's own shape: `offset` or `chunk`
/// present is the append form, anything else the anchored replace. A value
/// the line needs that is absent or not its type omits the line — the
/// arguments are model-controlled, so every shape must be total. The bodies
/// and the `expected_digest` are never read for display: only sizes.
fn edit_form_line(arguments: &Value) -> Option<String> {
    if arguments.get("offset").is_some() || arguments.get("chunk").is_some() {
        let offset = arguments.get("offset").and_then(Value::as_u64)?;
        let chunk = arguments.get("chunk").and_then(Value::as_str)?;
        return Some(format!("  append {} bytes at offset {offset}", chunk.len()));
    }
    let old_text = arguments.get("old_text").and_then(Value::as_str)?;
    let new_text = arguments.get("new_text").and_then(Value::as_str)?;
    let scope = match arguments.get("replace_all").and_then(Value::as_bool) {
        Some(true) => "every occurrence of one anchored string",
        _ => "one anchored string",
    };
    Some(format!(
        "  replace {scope} · old_text {} bytes → new_text {} bytes",
        old_text.len(),
        new_text.len()
    ))
}
