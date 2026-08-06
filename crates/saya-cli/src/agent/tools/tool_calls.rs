use super::sql_format::{collapse_whitespace, format_sql};

/// A short, human-readable detail for a tool call, surfaced in approval prompts
/// and the transcript so the user sees exactly what will run. Returns the SQL
/// for the query tools (annotated with the target connection, or "all databases"
/// for the fan-out tool), or `None` for tools with nothing worth showing.
pub(crate) fn tool_call_detail(name: &str, arguments: &serde_json::Value) -> Option<String> {
    match name {
        "bounded_sql_query" | "bounded_sql_query_all" => {
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
        "bounded_sql_query" | "bounded_sql_query_all" => {
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
