fn session_tool() -> ToolDefinition {
    ToolDefinition {
        name: "session_task_list".into(),
        description: "may write session-scoped metadata only".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::WriteSession,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }
}
