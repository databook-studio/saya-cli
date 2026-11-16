fn policy_tool(effect: ToolEffect, concurrency: ToolConcurrency) -> ToolDefinition {
    ToolDefinition {
        name: "policy_tool".into(),
        description: "policy matrix tool".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect,
        concurrency,
        completion: None,
    }
}
