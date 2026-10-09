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
    /// sites unchanged. `permit_workspace_writes` gates `workspace_write` the
    /// hidden-not-advertised way: a write tool the model can see but never use
    /// wastes context and invites retries, so it is omitted until the run was
    /// constructed with workspace writes permitted. `permit_external_effects`
    /// gates `render_chart` the same way: the chart tool declares
    /// `external_side_effect: true`, so read-only, never, and plan enforcement
    /// deny every call to it — advertising it there would hand the model a
    /// tool and then refuse every call.
    pub(crate) fn definitions(
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
        permit_workspace_writes: bool,
        permit_external_effects: bool,
    ) -> Vec<ToolDefinition> {
        Self::definitions_with_chart_save(
            allow_query_data,
            has_state_store,
            permit_candidate_writes,
            permit_workspace_writes,
            permit_external_effects,
            false,
        )
    }

    pub(crate) fn definitions_with_chart_save(
        allow_query_data: bool,
        has_state_store: bool,
        permit_candidate_writes: bool,
        permit_workspace_writes: bool,
        permit_external_effects: bool,
        permit_chart_save: bool,
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
        append_clarification_tools(&mut tools);
        super::super::workspace::append_definitions(&mut tools, permit_workspace_writes);
        append_query_tools(&mut tools, connection_prop.clone(), allow_query_data);
        append_chart_and_answer_tools(
            &mut tools,
            connection_prop.clone(),
            allow_query_data,
            permit_external_effects,
            permit_chart_save,
        );
        // Contract tools are a sibling concern (see `knowledge`); they are
        // appended here so the agent receives one flat definition list, matching
        // how this function is assembled for the database tools.
        tools.extend(crate::agent::tools::knowledge::contract_tool_definitions(
            allow_query_data,
            has_state_store,
        ));
        let _ = permit_candidate_writes;
        tools
    }
}

include!("definitions/clarification.rs");
include!("definitions/queries.rs");
include!("definitions/chart_and_answer.rs");

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
        "render_chart" => (
            &[
                "connection",
                "sql",
                "chart_type",
                "x",
                "y",
                "title",
                "save_to",
                "expected_digest",
            ][..],
            true,
        ),
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
    if name == "render_chart"
        && object
            .get("save_to")
            .is_some_and(|value| !value.is_string())
    {
        return Err(ToolError::PathNotString);
    }
    // The chart save's digest precondition has its own typing (`chart_save`):
    // a present non-null digest must be a string, and a digest without a
    // string `save_to` is a refusal — there is no destination to guard.
    if name == "render_chart" {
        super::chart_save::validate_digest_arguments(object)?;
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
        let definitions = DatabaseTools::definitions(true, false, false, false, true);
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
