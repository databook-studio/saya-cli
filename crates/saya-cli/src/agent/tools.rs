use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolExecutor};
use saya_store::SqliteStateStore;

#[cfg(test)]
use crate::connection::ConnectionEntry;
use crate::connection::ConnectionRegistry;
#[cfg(test)]
use saya_connectors::DatabaseConnector;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    registry: ConnectionRegistry,
    max_rows: usize,
    allow_query_data: bool,
    state_db: Option<SqliteStateStore>,
}

impl DatabaseTools {
    /// Creates database tools with a single optional primary connection for testing.
    #[cfg(test)]
    pub(crate) fn new(
        connector: Option<Box<dyn DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        let mut registry = ConnectionRegistry::new("primary");
        if let Some(c) = connector {
            let dialect = c.dialect();
            registry.insert(
                "primary",
                ConnectionEntry {
                    connector: c,
                    dialect,
                    profile_id: None,
                },
            );
        }
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
        }
    }

    /// Creates database tools configured with a connection registry.
    pub(crate) fn with_registry(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
        }
    }

    /// Runs `sql` against every connected database independently, collecting a
    /// per-database `result` or `error` so a dialect mismatch on one database
    /// never sinks the rest. A single approval covers the whole fan-out.
    async fn query_all(&self, sql: &str) -> Result<serde_json::Value, String> {
        let entries = self.registry.entries();
        if entries.is_empty() {
            return Err("no database profile is selected".into());
        }
        let mut databases = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            let outcome = super::state_tools::query(
                entry.connector.as_ref(),
                sql,
                self.max_rows,
                self.state_db.as_ref(),
                entry.profile_id.as_deref(),
            )
            .await;
            let mut record = serde_json::Map::new();
            record.insert(
                "connection".into(),
                serde_json::Value::String(name.to_string()),
            );
            record.insert(
                "dialect".into(),
                serde_json::Value::String(entry.dialect.as_str().to_string()),
            );
            match outcome {
                Ok(result) => {
                    record.insert("result".into(), result);
                }
                Err(error) => {
                    record.insert("error".into(), serde_json::Value::String(error));
                }
            }
            databases.push(serde_json::Value::Object(record));
        }
        Ok(serde_json::json!({ "databases": databases }))
    }

    /// Returns available database tool definitions.
    pub(crate) fn definitions(allow_query_data: bool) -> Vec<ToolDefinition> {
        let connection_prop = serde_json::json!({
            "type": "string",
            "description": "Optional. Name of the database connection to target; defaults to the primary. Available connections and their dialects are listed in the system context."
        });

        let mut tools = vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "Inspect the selected database schema without changing data.".into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "connection": connection_prop
                },
                "additionalProperties": false
            }),
            requires_approval: false,
        }];
        if allow_query_data {
            tools.push(ToolDefinition {
                name: "bounded_sql_query".into(),
                description: "Run one bounded read-only SQL query against the selected database."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop,
                        "sql": {
                            "type": "string"
                        }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                requires_approval: true,
            });
            tools.push(ToolDefinition {
                name: "bounded_sql_query_all".into(),
                description: "Run one bounded read-only SQL query against EVERY connected \
                    database at once and return the per-database results. Use this when the \
                    same question should be answered across all connected databases; each \
                    database runs independently, so a failure on one (e.g. a dialect \
                    mismatch) is reported alongside the successes rather than aborting the \
                    rest. Do not pass a `connection` argument."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sql": {
                            "type": "string"
                        }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                requires_approval: true,
            });
        }
        tools
    }
}

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

/// Formats a SQL string for readable multi-line display. Purely cosmetic and
/// NEVER used to build a query that executes: it collapses runs of whitespace to
/// single spaces, then inserts a line break before each major clause keyword
/// (case-insensitive, whole word), preserving the original casing of the text.
pub(crate) fn format_sql(sql: &str) -> String {
    // Multi-word keywords must be checked before their single-word prefixes.
    const KEYWORDS: &[&str] = &[
        "LEFT JOIN",
        "RIGHT JOIN",
        "INNER JOIN",
        "OUTER JOIN",
        "FULL JOIN",
        "CROSS JOIN",
        "GROUP BY",
        "ORDER BY",
        "UNION ALL",
        "FROM",
        "WHERE",
        "HAVING",
        "LIMIT",
        "OFFSET",
        "JOIN",
        "UNION",
        "VALUES",
        "RETURNING",
    ];
    // 1) collapse whitespace
    let collapsed = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    // 2) walk tokens by char; before a keyword match at a word boundary
    //    (not at position 0), start a new line.
    let bytes = collapsed.as_bytes();
    let mut out = String::with_capacity(collapsed.len() + 16);
    let mut i = 0usize;
    while i < collapsed.len() {
        // Only consider a break at a word start: i == 0 handled (no break), or prev char is space.
        let at_word_start = i == 0 || bytes[i - 1] == b' ';
        let mut matched: Option<usize> = None; // length of matched keyword
        if at_word_start && i != 0 {
            for kw in KEYWORDS {
                let end = i + kw.len();
                if end <= collapsed.len()
                    && collapsed.is_char_boundary(end)
                    && collapsed[i..end].eq_ignore_ascii_case(kw)
                    && (end == collapsed.len() || bytes[end] == b' ')
                {
                    matched = Some(kw.len());
                    break;
                }
            }
        }
        if matched.is_some() {
            // Trim the single space we already emitted before this keyword, then newline.
            if out.ends_with(' ') {
                out.pop();
            }
            out.push('\n');
        }
        let ch = collapsed[i..].chars().next().unwrap_or('\0');
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Collapses runs of whitespace (including newlines) into single spaces so
/// multi-line model SQL renders as one tidy line across every surface.
fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[async_trait]
impl ToolExecutor for DatabaseTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if matches!(name, "bounded_sql_query" | "bounded_sql_query_all") && !self.allow_query_data {
            return Err("data sharing is disabled for this cloud provider".into());
        }
        if name == "bounded_sql_query_all" {
            let sql = arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .ok_or("invalid query arguments")?;
            return self.query_all(sql).await;
        }
        let connection = arguments.get("connection").and_then(|v| v.as_str());
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                super::state_tools::schema(
                    entry.connector.as_ref(),
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            "bounded_sql_query" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("invalid query arguments")?;
                super::state_tools::query(
                    entry.connector.as_ref(),
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            _ => Err("unsupported read-only tool".into()),
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
mod format_tests {
    use super::*;

    #[test]
    fn format_sql_breaks_before_keywords_and_preserves_casing() {
        let raw = "select 1 from t where a=1 limit 10";
        let formatted = format_sql(raw);
        assert_eq!(formatted, "select 1\nfrom t\nwhere a=1\nlimit 10");
    }

    #[test]
    fn format_sql_collapses_whitespace_first() {
        let raw = "SELECT   1\n  FROM   t\n\n  WHERE   a = 1";
        let formatted = format_sql(raw);
        assert_eq!(formatted, "SELECT 1\nFROM t\nWHERE a = 1");
    }

    #[test]
    fn format_sql_handles_multibyte_utf8() {
        let raw = "select 'café' from t";
        let formatted = format_sql(raw);
        assert_eq!(formatted, "select 'café'\nfrom t");
    }

    #[test]
    fn sql_tool_call_bounded_sql_query_default() {
        let args = serde_json::json!({ "sql": "select 1" });
        let call = sql_tool_call("bounded_sql_query", &args).expect("should return SqlCall");
        assert_eq!(call.target, None);
        assert_eq!(call.sql, "select 1");
    }

    #[test]
    fn sql_tool_call_bounded_sql_query_with_connection() {
        let args = serde_json::json!({ "sql": "select 1", "connection": "wh" });
        let call = sql_tool_call("bounded_sql_query", &args).expect("should return SqlCall");
        assert_eq!(call.target, Some("@wh".to_string()));
        assert_eq!(call.sql, "select 1");
    }

    #[test]
    fn sql_tool_call_bounded_sql_query_all_no_connection() {
        let args = serde_json::json!({ "sql": "select 1" });
        let call = sql_tool_call("bounded_sql_query_all", &args).expect("should return SqlCall");
        assert_eq!(call.target, Some("all connected databases".to_string()));
        assert_eq!(call.sql, "select 1");
    }

    #[test]
    fn sql_tool_call_schema_discovery_returns_none() {
        let args = serde_json::json!({ "connection": "wh" });
        assert!(sql_tool_call("schema_discovery", &args).is_none());
    }
}
