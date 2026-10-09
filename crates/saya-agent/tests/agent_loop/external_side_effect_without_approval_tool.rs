/// A tool that declares an external side effect but does *not* require
/// approval is a misconfiguration the policy must gate — never auto-run. The
/// two execution paths (sequential, when the call arrives alone, and the
/// concurrent batch, when it arrives among others) must agree: the gated tool
/// is denied in both, never silently executed. This is the divergence guard
/// for S8: if the batch predicate and the sequential path stop consulting the
/// same policy, one of these assertions fails.
fn external_side_effect_without_approval_tool() -> ToolDefinition {
    ToolDefinition {
        name: "open_browser".into(),
        description: "opens something outside the agent".into(),
        read_only: true,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: false,
            local_state: saya_agent::LocalStateEffect::None,
        },
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}
