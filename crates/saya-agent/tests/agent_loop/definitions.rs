fn definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "bounded_sql_query".into(),
            description: "read-only query".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: saya_agent::LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        },
        ToolDefinition {
            name: "bounded_sql_query_all".into(),
            description: "fan-out read-only query".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: true,
                external_side_effect: false,
                requires_approval: true,
                local_state: saya_agent::LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Serial,
            completion: None,
        },
        ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema discovery".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            effect: ToolEffect {
                database_data: false,
                external_side_effect: false,
                requires_approval: false,
                local_state: saya_agent::LocalStateEffect::None,
            },
            concurrency: ToolConcurrency::Concurrent,
            completion: None,
        },
    ]
}
