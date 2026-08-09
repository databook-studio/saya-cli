use futures_util::stream::{FuturesUnordered, StreamExt};
use saya_agent::ToolDefinition;
use saya_store::SqliteStateStore;
use std::time::Duration;

use crate::connection::ConnectionRegistry;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    registry: ConnectionRegistry,
    max_rows: usize,
    allow_query_data: bool,
    state_db: Option<SqliteStateStore>,
    max_concurrent_fan_out_queries: usize,
    fan_out_query_timeout: Duration,
}

impl DatabaseTools {
    const MAX_CONCURRENT_FAN_OUT_QUERIES: usize = 4;
    const FAN_OUT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

    /// Creates database tools with a single optional primary connection for testing.
    #[cfg(test)]
    pub(crate) fn new(
        connector: Option<Box<dyn saya_connectors::DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        use crate::connection::ConnectionEntry;

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
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
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
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(super) fn with_registry_and_fan_out_limits(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        max_concurrent_fan_out_queries: usize,
        fan_out_query_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
            max_concurrent_fan_out_queries: max_concurrent_fan_out_queries.max(1),
            fan_out_query_timeout,
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
        let mut entries = entries.into_iter().enumerate();
        let mut pending = FuturesUnordered::new();
        for _ in 0..self.max_concurrent_fan_out_queries {
            if let Some((index, (name, entry))) = entries.next() {
                pending.push(self.query_one(index, name, entry, sql));
            }
        }

        let mut databases = Vec::new();
        while let Some(database) = pending.next().await {
            databases.push(database);
            if let Some((index, (name, entry))) = entries.next() {
                pending.push(self.query_one(index, name, entry, sql));
            }
        }
        databases.sort_by_key(|(index, _, _, _)| *index);

        let databases = databases
            .into_iter()
            .map(|(_, name, dialect, outcome)| {
                let mut record = serde_json::Map::new();
                record.insert("connection".into(), serde_json::Value::String(name));
                record.insert("dialect".into(), serde_json::Value::String(dialect));
                match outcome {
                    Ok(result) => {
                        record.insert("result".into(), result);
                    }
                    Err(error) => {
                        record.insert("error".into(), serde_json::Value::String(error));
                    }
                }
                serde_json::Value::Object(record)
            })
            .collect::<Vec<_>>();
        Ok(serde_json::json!({ "databases": databases }))
    }

    async fn query_one(
        &self,
        index: usize,
        name: &str,
        entry: &crate::connection::ConnectionEntry,
        sql: &str,
    ) -> (usize, String, String, Result<serde_json::Value, String>) {
        let outcome = tokio::time::timeout(
            self.fan_out_query_timeout,
            super::super::state_tools::query(
                entry.connector.as_ref(),
                sql,
                self.max_rows,
                self.state_db.as_ref(),
                entry.profile_id.as_deref(),
            ),
        )
        .await
        .unwrap_or_else(|_| Err("read-only query timed out".into()));
        (
            index,
            name.to_string(),
            entry.dialect.as_str().to_string(),
            outcome,
        )
    }

    /// Runs `sql` read-only at the full row cap, renders an interactive chart file with the
    /// requested type, opens it, and returns the file path (never the rows).
    async fn render_chart(
        &self,
        arguments: &serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let sql = arguments
            .get("sql")
            .and_then(serde_json::Value::as_str)
            .ok_or("invalid query arguments")?;
        let connection = arguments.get("connection").and_then(|v| v.as_str());
        let entry = self.registry.resolve(connection)?;
        let result = entry
            .connector
            .execute(saya_types::QueryRequest::new(sql, self.max_rows))
            .await
            .map_err(|_| "read-only query failed".to_string())?;

        let mut spec = crate::chart::suggest_spec(&result);
        if let Some(kind) = arguments
            .get("chart_type")
            .and_then(serde_json::Value::as_str)
            .and_then(crate::chart::ChartKind::parse)
        {
            spec.kind = kind;
        }
        if let Some(x) = arguments.get("x").and_then(serde_json::Value::as_str) {
            spec.x = Some(x.to_string());
        }
        if let Some(y) = arguments.get("y").and_then(|v| v.as_array()) {
            let cols: Vec<String> = y
                .iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            if !cols.is_empty() {
                spec.y = cols;
            }
        }
        if let Some(title) = arguments.get("title").and_then(serde_json::Value::as_str) {
            spec.title = Some(title.to_string());
        }

        let html = crate::chart::render_html(&result, &spec)?;
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("saya-chart-{unique}.html"));
        crate::chart::write_html(&html, &path)?;
        let _ = crate::chart::open_file(&path);
        Ok(serde_json::json!({
            "path": path.display().to_string(),
            "note": "Interactive chart written and opened in the browser."
        }))
    }

    /// Dispatches a read-only agent tool call to its selected connection.
    pub(super) async fn execute_read_only(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        validate_arguments(name, &arguments)?;
        if matches!(
            name,
            "bounded_sql_query" | "bounded_sql_query_all" | "render_chart"
        ) && !self.allow_query_data
        {
            return Err("data sharing is disabled for this cloud provider".into());
        }
        if name == "bounded_sql_query_all" {
            let sql = arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .ok_or("invalid query arguments")?;
            return self.query_all(sql).await;
        }
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str);
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                super::super::state_tools::schema(
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
                super::super::state_tools::query(
                    entry.connector.as_ref(),
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            "render_chart" => self.render_chart(&arguments).await,
            _ => Err("unsupported read-only tool".into()),
        }
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
                        "connection": connection_prop.clone(),
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
            tools.push(ToolDefinition {
                name: "render_chart".into(),
                description: "Visualize the results of a SQL query as an interactive chart the user can open \
                    in their browser. Call this whenever the user asks to chart, plot, graph, or visualize \
                    data. Provide the SQL to run and choose the chart_type that best fits the data: `bar` for \
                    comparing categories, `line` or `area` for trends over an ordered/time axis, `pie` or \
                    `doughnut` for a category's share of a total, `scatter` for the relationship between two \
                    numeric columns. Optionally name the x (label) column, the y (value) column(s), and a \
                    title. The chart is written to a file and opened; only the file path is returned."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" },
                        "chart_type": { "type": "string", "enum": ["bar","line","area","pie","doughnut","scatter"] },
                        "x": { "type": "string" },
                        "y": { "type": "array", "items": { "type": "string" } },
                        "title": { "type": "string" }
                    },
                    "required": ["sql", "chart_type"],
                    "additionalProperties": false
                }),
                requires_approval: false,
            });
        }
        tools
    }
}

fn validate_arguments(name: &str, arguments: &serde_json::Value) -> Result<(), String> {
    let object = arguments
        .as_object()
        .ok_or("invalid tool arguments: expected an object")?;
    let (allowed, requires_sql) = match name {
        "schema_discovery" => (&["connection"][..], false),
        "bounded_sql_query" => (&["connection", "sql"][..], true),
        "bounded_sql_query_all" => (&["sql"][..], true),
        "render_chart" => (
            &["connection", "sql", "chart_type", "x", "y", "title"][..],
            true,
        ),
        _ => return Err("unsupported read-only tool".into()),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err("invalid tool arguments: unsupported property".into());
    }
    if object
        .get("connection")
        .is_some_and(|connection| !connection.is_string())
    {
        return Err("invalid tool arguments: connection must be a string".into());
    }
    if requires_sql && !object.get("sql").is_some_and(serde_json::Value::is_string) {
        return Err("invalid tool arguments: sql must be a string".into());
    }
    Ok(())
}
