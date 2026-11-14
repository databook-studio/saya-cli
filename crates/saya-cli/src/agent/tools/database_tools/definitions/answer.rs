fn append_answer_tools(tools: &mut Vec<ToolDefinition>, allow_query_data: bool) {
    if allow_query_data {
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
