//! The live per-call lines a buffered tool request mirrors onto the tail.
//! Pure formatting, split out of `tool_buffer.rs` to keep it under the
//! file-size cap; it performs no mutation and no bookkeeping.

pub(super) fn live_request_lines(name: &str, arguments: &serde_json::Value) -> Vec<String> {
    if let Some(call) = crate::agent::tools::sql_tool_call(name, arguments) {
        let header = match &call.target {
            Some(t) => format!("SQL · {t}"),
            None => "SQL".to_string(),
        };
        let body = call
            .sql
            .lines()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        return vec![format!("{header}\n{body}")];
    }
    vec![
        match crate::agent::tools::tool_call_detail(name, arguments) {
            Some(detail) => format!("→ {name}: {detail}"),
            None => format!("→ {name}"),
        },
    ]
}
