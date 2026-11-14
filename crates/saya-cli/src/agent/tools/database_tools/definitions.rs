use saya_agent::{LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect, ToolError};

use super::DatabaseTools;

impl DatabaseTools {
    /// Returns available database tool definitions. Contract read tools are
    /// appended only when a state store is present **and** database context is
    /// allowed; when the privacy gate forbids database context they are hidden
    /// rather than advertised as always-empty, matching the read-tool
    /// precedent. `permit_candidate_writes` gates nothing here: no tool
    /// declares `WriteCandidate` — proposals are extracted post-turn from the
    /// bounded turn record — so the flag is accepted only to keep the call
    /// sites unchanged. `permit_workspace_writes` gates `write` the
    /// hidden-not-advertised way: a write tool the model can see but never use
    /// wastes context and invites retries, so it is omitted until the run was
    /// constructed with workspace writes permitted.
    pub(crate) fn definitions(
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
        permit_workspace_writes: bool,
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
            concurrency: ToolConcurrency::Serial,
            completion: None,
        }];
        // The ask is not a database tool: it needs no connection, so it is
        // appended here only because this list is what a chat surface
        // advertises (its definition lives in the shared `clarification`
        // module, where the coding surfaces reach it too).
        tools.push(crate::agent::tools::clarification::definition());
        super::super::workspace::append_definitions(&mut tools, permit_workspace_writes);
        append_query_tools(&mut tools, connection_prop.clone(), allow_query_data);
        append_answer_tools(&mut tools, allow_query_data);
        // Contract tools are a sibling concern (see `knowledge`); they are
        // appended here so the agent receives one flat definition list, matching
        // how this function is assembled for the database tools.
        tools.extend(crate::agent::tools::knowledge::contract_tool_definitions(
            allow_query_data,
            has_state_store,
        ));
        // The notebook edit goes last: it is the one workspace write added
        // after this surface's tool order was fixed, and a name appended at
        // the end leaves every earlier tool's position — and so the request's
        // cached prefix — exactly as it was.
        super::super::workspace::append_notebook_edit(&mut tools, permit_workspace_writes);
        let _ = permit_candidate_writes;
        tools
    }
}

include!("definitions/queries.rs");
include!("definitions/answer.rs");

pub(super) fn validate_arguments(
    name: &str,
    arguments: &serde_json::Value,
) -> Result<(), ToolError> {
    if super::super::workspace::is_tool_name(name) {
        return super::super::workspace::validate_arguments(name, arguments);
    }
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    let (allowed, requires_sql) = match name {
        "schema_discovery" => (&["connection"][..], false),
        "request_clarification" => (&["question", "options"][..], false),
        "bounded_sql_query" => (&["connection", "sql"][..], true),
        "bounded_sql_query_all" => (&["sql"][..], true),
        "result_shape" => (&["connection", "sql"][..], true),
        "column_health" => (&["connection", "sql"][..], true),
        "join_check" => (&["connection", "sql"][..], true),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// SQL operations share turn-scoped cancellation, observation, and capture
    /// state. Their advertised definitions must therefore stay serial even
    /// though the SQL safety layer admits read-only statements.
    #[test]
    fn query_definitions_stay_serial() {
        let definitions = DatabaseTools::definitions(true, false, false, false);
        for name in [
            "schema_discovery",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
        ] {
            let definition = definitions
                .iter()
                .find(|definition| definition.name == name)
                .unwrap_or_else(|| panic!("{name} is advertised"));
            assert_eq!(definition.concurrency, ToolConcurrency::Serial, "{name}");
        }
    }
}
