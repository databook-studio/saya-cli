use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect, ToolError};

use super::DatabaseTools;

impl DatabaseTools {
    /// Returns available database tool definitions. Contract read tools are
    /// appended only when a state store is present **and** database context is
    /// allowed; when the privacy gate forbids database context they are hidden
    /// rather than advertised as always-empty. `contract_propose`
    /// is appended only when candidate writes are permitted **and** a store is
    /// present **and** the gate is open — hidden, not advertised and
    /// denied, matching the read-tool precedent.
    pub(crate) fn definitions(
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
    ) -> Vec<ToolDefinition> {
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
                local_state: LocalStateEffect::None,
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
                    local_state: LocalStateEffect::None,
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
                    local_state: LocalStateEffect::None,
                },
            });
            tools.push(ToolDefinition {
                name: "result_shape".into(),
                description: "Run one bounded read-only SQL query and return its SHAPE only — \
                    the row count, whether the row cap was hit, and the column names with a \
                    type label — and never any row values. Use this instead of \
                    bounded_sql_query when you only need to know whether a query worked, \
                    roughly how many rows it returned, and what columns came back; it costs \
                    far less context than fetching the rows. Read `truncated` first: when it \
                    is true, `row_count` is a floor and not the real total, so a capped count \
                    read as the true count leads to a false conclusion."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: true,
                    local_state: LocalStateEffect::None,
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
                    local_state: LocalStateEffect::None,
                },
            });
            tools.push(ToolDefinition {
                name: "designate_answer".into(),
                description: "Designate the SQL query that answers the user's question — the \
                    statement that produced the answer, never an exploratory probe you ran to \
                    learn the schema or test a guess. Call this exactly once, in your final \
                    message, alongside your prose answer, with the SQL that produced it. This \
                    does not run a query — it records which of the queries you ran is the \
                    answering one. Omit it when no single query answers the question."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "sql": { "type": "string" }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                effect: ToolEffect {
                    database_data: false,
                    external_side_effect: false,
                    requires_approval: false,
                    local_state: LocalStateEffect::None,
                },
            });
        }
        // Contract tools are a sibling concern (see `contract_tools`); they are
        // appended here so the agent receives one flat definition list, matching
        // how this function is assembled for the database tools.
        tools.extend(
            crate::agent::tools::contract_tools::contract_tool_definitions(
                allow_query_data,
                has_state_store,
            ),
        );
        let _ = permit_candidate_writes;
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
        "result_shape" => (&["connection", "sql"][..], true),
        "render_chart" => (
            &["connection", "sql", "chart_type", "x", "y", "title"][..],
            true,
        ),
        "designate_answer" => (&["sql"][..], true),
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
