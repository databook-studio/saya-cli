fn workspace_tool() -> ToolDefinition {
    ToolDefinition {
        name: "workspace_write".into(),
        description: "may write a file in the run workspace".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteWorkspace,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}
