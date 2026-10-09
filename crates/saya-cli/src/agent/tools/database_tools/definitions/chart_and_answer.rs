fn append_chart_and_answer_tools(
    tools: &mut Vec<ToolDefinition>,
    connection_prop: serde_json::Value,
    allow_query_data: bool,
    permit_external_effects: bool,
    permit_chart_save: bool,
) {
    if allow_query_data {
        // `render_chart` declares `external_side_effect: true` — it writes
        // a file and opens the user's browser — so it is pushed only where
        // the surface can actually allow an external effect. Anywhere
        // else the enforcement denies every call, so the definition stays
        // hidden, not advertised-and-denied.
        if permit_external_effects {
            tools.push(ToolDefinition {
                    name: "render_chart".into(),
                    description: "Visualize the results of a SQL query as an interactive chart the user can open \
                    in their browser. Call this whenever the user asks to chart, plot, graph, or visualize \
                    data. Provide the SQL to run and choose the chart_type that best fits the data: `bar` for \
                    comparing categories, `line` or `area` for trends over an ordered/time axis, `pie` or \
                    `doughnut` for a category's share of a total, `scatter` for the relationship between two \
                    numeric columns. Optionally name the x (label) column, the y (value) column(s), and a \
                    title. The chart opens from a private temporary copy. When `save_to` is supplied, the same \
                    HTML is also saved in the workspace and that relative path is returned. \
                    Saving over an existing file requires the `expected_digest` you observed by \
                    reading that file first; without a digest an existing file is refused."
                        .into(),
                    read_only: false,
                parameters: {
                    let mut properties = serde_json::json!({
                        "connection": connection_prop.clone(),
                        "sql": { "type": "string" },
                        "chart_type": { "type": "string", "enum": ["bar","line","area","pie","doughnut","scatter"] },
                        "x": { "type": "string" },
                        "y": { "type": "array", "items": { "type": "string" } },
                        "title": { "type": "string" }
                    });
                    if permit_chart_save {
                        properties["save_to"] = serde_json::json!({
                            "type": "string",
                            "description": "Optional workspace-relative destination for the chart HTML."
                        });
                        properties["expected_digest"] = serde_json::json!({
                            "type": "string",
                            "description": "Optional, only with save_to. Omit it to create the \
                                file only: an existing file at the destination is refused. To \
                                replace a file, read it first with workspace_read and pass the \
                                digest you observed; never invent a digest."
                        });
                    }
                    serde_json::json!({
                    "type": "object",
                    "properties": properties,
                    "required": ["sql", "chart_type"],
                    "additionalProperties": false
                    })
                },
                    effect: ToolEffect {
                        database_data: false,
                        external_side_effect: true,
                        requires_approval: true,
                        local_state: if permit_chart_save {
                            LocalStateEffect::WriteWorkspace
                        } else {
                            LocalStateEffect::None
                        },
                    },
                    concurrency: ToolConcurrency::Serial,
                    completion: Some(if permit_chart_save {
                        "chart opened from a private temporary copy".into()
                    } else {
                        "chart written and opened".into()
                    }),
                });
        }
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
        });
    }
}
