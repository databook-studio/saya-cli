fn append_query_tools(
    tools: &mut Vec<ToolDefinition>,
    connection_prop: serde_json::Value,
    allow_query_data: bool,
) {
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
        });
        tools.push(ToolDefinition {
            name: "column_health".into(),
            description: "Run one bounded read-only SQL query and return per-column health \
                    statistics — null count, null percentage, distinct value count, and numeric \
                    zero count — and never any cell value. Use this to run your expression \
                    through the safety path BEFORE trusting it: a 100% null rate after a DATE() \
                    or CAST reveals that the function silently coerced every row (e.g. SQLite's \
                    DATE() returns NULL for '1/1/2021 12:01:36 AM', collapsing every group into \
                    one and turning a per-day average into a lifetime total). Read `truncated` \
                    first: when it is true, the stats are over a capped sample, not the full \
                    result, so a 100% null rate on a sample is still a strong signal but the \
                    counts are floors."
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
        });
        tools.push(ToolDefinition {
            name: "join_check".into(),
            description: "Check whether a JOIN in your query multiplies or drops rows before \
                    you trust a SUM, AVG, or COUNT over it. Builds two COUNT(*) statements — one \
                    over the full join and one over the base table alone — and compares them. \
                    Returns {applicable, joined_rows, base_rows, fanned_out, dropped_rows}. When \
                    the join is on a non-unique key, joined_rows exceeds base_rows and \
                    fanned_out is true, meaning every aggregate over the base table is inflated. \
                    When joined_rows is less than base_rows, the join dropped rows and \
                    dropped_rows is true — just as corrupting. When no sound probe can be built \
                    (no join, no distortable aggregate, subquery in FROM, etc.), returns \
                    applicable: false with a reason — never a guess. Call this WHILE building a \
                    join query, not only after the fact."
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
        });
    }
}
