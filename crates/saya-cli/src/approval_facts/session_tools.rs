//! The session file tools' fact lines: `workspace_write`'s path, bytes, and
//! containment rule; `scratch_sql`'s statement and the scratch's own policy.

use serde_json::Value;

use super::{ApprovalFacts, body};

/// `workspace_write`'s fact lines. The per-write byte bound and the
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
             write is atomic — an existing file is replaced whole, never partially"
                .to_string(),
        );
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    // The scope sentence renders only when a session grant is actually on
    // offer — last, closest to the `[s]` answer it describes.
    if let Some(scope) = super::scope::scope_sentence("workspace_write", grant) {
        lines.push(scope);
    }
    Some(body(
        "workspace_write — writes into this session's workspace".to_string(),
        lines,
    ))
}

/// `scratch_sql`'s fact lines: the statement plus the scratch's own policy —
/// single statement, the row cap, the per-statement timeout, the
/// session-local engine with external access off and no file reads.
pub(super) fn scratch_facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let sql = arguments.get("sql").and_then(Value::as_str)?;
    let sql = crate::agent::tools::collapse_whitespace(sql);
    if sql.is_empty() {
        return None;
    }
    let mut lines = vec![format!("  sql: {sql}")];
    if let Some(scratch) = facts.scratch.as_ref() {
        lines.push(format!(
            "  one statement per call · ≤ {} rows · {}s per-statement timeout",
            scratch.row_cap, scratch.timeout_seconds
        ));
        lines.push(
            "  a session-local DuckDB file in this session's state directory · external \
             access off · no file reads"
                .to_string(),
        );
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    Some(body(
        "scratch_sql — this session's scratch database".to_string(),
        lines,
    ))
}

/// `scratch_import`'s fact lines name the contained source and the import
/// bounds, without reflecting a CSV value into the approval surface.
pub(super) fn scratch_import_facts(
    arguments: &Value,
    facts: &ApprovalFacts,
    session_line: Option<String>,
) -> Option<String> {
    let path = arguments.get("path").and_then(Value::as_str)?;
    let table = arguments.get("table").and_then(Value::as_str)?;
    let mut lines = vec![
        format!(
            "  workspace CSV: {}",
            crate::agent::tools::collapse_whitespace(path)
        ),
        format!(
            "  scratch table: {}",
            crate::agent::tools::collapse_whitespace(table)
        ),
    ];
    lines.push("  ≤ 32 MiB · ≤ 500,000 rows · ≤ 512 columns · fields ≤ 64 KiB".to_string());
    if let Some(scratch) = facts.scratch.as_ref() {
        lines.push(format!(
            "  session-local DuckDB · ≤ {} result rows · {}s scratch SQL timeout · 60s import deadline · external access off",
            scratch.row_cap, scratch.timeout_seconds
        ));
    }
    if let Some(session_line) = session_line {
        lines.push(session_line);
    }
    Some(body(
        "scratch_import — workspace CSV into scratch".to_string(),
        lines,
    ))
}
