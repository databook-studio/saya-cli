use super::sql_format::{collapse_whitespace, format_sql};

/// A short, human-readable detail for a tool call, surfaced in approval prompts
/// and the transcript so the user sees exactly what will run. Returns the SQL
/// for the query tools (annotated with the target connection, or "all databases"
/// for the fan-out tool), or `None` for tools with nothing worth showing.
/// C0 redaction note: the detail rides the same redaction the rest of the
/// output does. `path`/`program` name what ran (a relative workspace path, a
/// bare program name); `content`/argv/stdout/stderr never reach the detail —
/// the failure tail below is a fixed vocabulary word, never tool output, and
/// the request detail carries no argument values beyond the key fact.
pub(crate) fn tool_call_detail(name: &str, arguments: &serde_json::Value) -> Option<String> {
    match name {
        "workspace_write" => {
            let path = arguments
                .get("path")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())?;
            Some(path.to_owned())
        }
        "run_command" => {
            let program = arguments
                .get("program")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty())?;
            Some(program.to_owned())
        }
        "bounded_sql_query"
        | "bounded_sql_query_all"
        | "render_chart"
        | "result_shape"
        | "column_health"
        | "join_check" => {
            let sql = arguments.get("sql").and_then(serde_json::Value::as_str)?;
            let sql = collapse_whitespace(sql);
            if sql.is_empty() {
                return None;
            }
            let connection = arguments
                .get("connection")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty());
            Some(match (name, connection) {
                ("render_chart", connection) => {
                    let chart_type = arguments
                        .get("chart_type")
                        .and_then(serde_json::Value::as_str)
                        .filter(|v| !v.is_empty());
                    match (connection, chart_type) {
                        (Some(conn), Some(ct)) => format!("{sql}  (@{conn}) (chart: {ct})"),
                        (Some(conn), None) => format!("{sql}  (@{conn})"),
                        (None, Some(ct)) => format!("{sql}  (chart: {ct})"),
                        (None, None) => sql,
                    }
                }
                (_, Some(connection)) => format!("{sql}  (@{connection})"),
                ("bounded_sql_query_all", None) => format!("{sql}  (all connected databases)"),
                _ => sql,
            })
        }
        _ => None,
    }
}
/// A SQL query tool call, ready for readable display: the target database label
/// (`None` = the primary connection) and the SQL formatted across lines.
pub(crate) struct SqlCall {
    pub(crate) target: Option<String>,
    pub(crate) sql: String,
}

/// If this tool call is a SQL query tool, returns its target label and the SQL
/// formatted for display. Returns `None` for non-query tools (e.g. schema_discovery)
/// or when there is no non-empty `sql` argument.
pub(crate) fn sql_tool_call(name: &str, arguments: &serde_json::Value) -> Option<SqlCall> {
    match name {
        "bounded_sql_query"
        | "bounded_sql_query_all"
        | "render_chart"
        | "result_shape"
        | "column_health"
        | "join_check" => {
            let raw = arguments.get("sql").and_then(serde_json::Value::as_str)?;
            let sql = format_sql(raw);
            if sql.is_empty() {
                return None;
            }
            let connection = arguments
                .get("connection")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty());
            let target = match (name, connection) {
                (_, Some(connection)) => Some(format!("@{connection}")),
                ("bounded_sql_query_all", None) => Some("all connected databases".to_string()),
                _ => None,
            };
            Some(SqlCall { target, sql })
        }
        _ => None,
    }
}

/// C0 properties 1, 2 (request half), 4, 5: the shared detail seam both
/// adapters render from. Placed beside the seam per the testing standard
/// (unit tests beside the code); the completion half lives in
/// `saya-agent/tests/c0_name_the_call.rs`, beside the summaries it pins.
#[cfg(test)]
mod c0_property_tests {
    use super::tool_call_detail;

    /// Property 1: a `workspace_write` request detail names the file it wrote.
    #[test]
    fn workspace_write_completion_names_the_file_it_wrote() {
        let detail = tool_call_detail(
            "workspace_write",
            &serde_json::json!({"path": "notes.md", "content": "hi"}),
        )
        .expect("workspace_write detail exists");
        assert!(
            detail.contains("notes.md"),
            "workspace_write completion must name the file: got {detail:?}"
        );
    }

    /// Property 2 (request half): a `run_command` request detail names the program.
    #[test]
    fn run_command_request_names_the_program() {
        let detail = tool_call_detail(
            "run_command",
            &serde_json::json!({"program": "pytest", "args": ["-q"]}),
        )
        .expect("run_command detail exists");
        assert!(
            detail.contains("pytest"),
            "run_command request must name the program: got {detail:?}"
        );
    }

    /// Property 4: both adapters render from the same seam so they cannot
    /// drift — the piped `terminal_event` mapper and the TUI `apply_event`
    /// both call `tool_call_detail`, and the shared `ToolRequested` text
    /// carries the path. Pinned here as the seam identity plus the piped
    /// half; the TUI half is pinned beside `apply_event` in
    /// `stream_events.rs`.
    #[test]
    fn both_adapters_render_the_same_fact_for_the_same_call() {
        use crate::stream_render::terminal_event;

        let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
        let piped = terminal_event(saya_agent::AgentEvent::tool_requested(
            "workspace_write",
            arguments,
            Some(saya_agent::ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: saya_agent::LocalStateEffect::WriteWorkspace,
            }),
        ))
        .expect("a ToolRequested renders headlessly");
        match piped {
            crate::TerminalEvent::ToolRequested { detail, .. } => {
                let detail = detail.expect("workspace_write exposes a detail");
                assert!(
                    detail.contains("notes.md"),
                    "the shared seam must name the file: {detail:?}"
                );
                assert_eq!(
                    detail,
                    tool_call_detail(
                        "workspace_write",
                        &serde_json::json!({"path": "notes.md", "content": "hi"}),
                    )
                    .expect("same seam, same fact"),
                    "both adapters read the same seam, so the fact cannot drift"
                );
            }
            other => panic!("a ToolRequested must map to ToolRequested: {other:?}"),
        }
    }

    /// Property 5: a tool this slice does not cover keeps today's text,
    /// byte-exact — `workspace_list` still exposes no detail.
    #[test]
    fn an_uncovered_tool_keeps_todays_text_byte_exact() {
        assert_eq!(
            tool_call_detail("workspace_list", &serde_json::json!({"path": "notes"})),
            None,
            "workspace_list must keep today's None detail"
        );
        assert_eq!(
            tool_call_detail("schema_discovery", &serde_json::json!({})),
            None,
            "schema_discovery must keep today's None detail"
        );
    }

    /// Extra: no argument values beyond the key fact reach the request
    /// detail — file content and argv never ride the line (the request
    /// detail names the program only; argv could carry secrets, and the
    /// approval prompt already shows the full call).
    #[test]
    fn request_detail_never_carries_values_beyond_the_key_fact() {
        let write = tool_call_detail(
            "workspace_write",
            &serde_json::json!({"path": "notes.md", "content": "SECRET-BODY"}),
        )
        .expect("workspace_write detail exists");
        assert!(
            !write.contains("SECRET-BODY"),
            "file content must not reach the line: {write:?}"
        );
        let run = tool_call_detail(
            "run_command",
            &serde_json::json!({"program": "pytest", "args": ["--token", "SECRET-ARG"]}),
        )
        .expect("run_command detail exists");
        assert_eq!(
            run, "pytest",
            "the request detail names the program only: {run:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_chart_tool_call_detail_surfaces_sql() {
        let detail = tool_call_detail(
            "render_chart",
            &serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        )
        .expect("render_chart detail exists");
        assert!(detail.contains("SELECT 1"));
        assert!(detail.contains("(chart: bar)"));
    }

    #[test]
    fn render_chart_sql_tool_call_surfaces_sql() {
        let call = sql_tool_call(
            "render_chart",
            &serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
        )
        .expect("render_chart SqlCall exists");
        assert_eq!(call.target, None);
        assert_eq!(call.sql, "SELECT 1");
    }
}
