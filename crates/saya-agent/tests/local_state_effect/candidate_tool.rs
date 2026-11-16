fn candidate_tool() -> ToolDefinition {
    ToolDefinition {
        name: "remember_candidate".into(),
        description: "may persist a candidate claim".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteCandidate,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}
