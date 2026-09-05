use super::sql_format::{collapse_whitespace, format_sql};

/// A short, human-readable detail for a tool call, surfaced in approval prompts
/// and the transcript so the user sees exactly what will run. Returns the SQL
/// for the query tools (annotated with the target connection, or "all databases"
/// for the fan-out tool), or `None` for tools with nothing worth showing.
pub(crate) fn tool_call_detail(name: &str, arguments: &serde_json::Value) -> Option<String> {
    match name {
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
