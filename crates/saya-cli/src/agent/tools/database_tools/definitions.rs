use saya_agent::{ToolDefinition, ToolEffect, ToolError};

use super::DatabaseTools;

impl DatabaseTools {
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
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
            },
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
                effect: ToolEffect {
                    database_data: true,
                    external_side_effect: false,
                    requires_approval: true,
                },
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
                effect: ToolEffect {
                    database_data: true,
                    external_side_effect: false,
                    requires_approval: true,
                },
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
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: true,
                    requires_approval: true,
                },
            });
        }
        tools
    }
}

pub(super) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    let (allowed, requires_sql) = match name {
        "schema_discovery" => (&["connection"][..], false),
        "bounded_sql_query" => (&["connection", "sql"][..], true),
        "bounded_sql_query_all" => (&["sql"][..], true),
        "render_chart" => (
            &["connection", "sql", "chart_type", "x", "y", "title"][..],
            true,
        ),
        _ => return Err(ToolError::UnsupportedTool),
    };
    if object.keys().any(|key| !allowed.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    if object
        .get("connection")
        .is_some_and(|connection| !connection.is_string())
    {
        return Err(ToolError::ConnectionNotString);
    }
    if requires_sql && !object.get("sql").is_some_and(serde_json::Value::is_string) {
        return Err(ToolError::SqlNotString);
    }
    Ok(())
}
