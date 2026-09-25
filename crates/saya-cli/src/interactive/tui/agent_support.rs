use crate::agent::tools::DatabaseTools;
use crate::approval_facts::ApprovalFacts;
use saya_agent::{LocalStateEffect, ToolDefinition, ToolEffect};

pub(crate) fn read_shaped_tool() -> ToolDefinition {
    let tools = DatabaseTools::definitions(true, false, false, false, true);
    tools
        .iter()
        .find(|tool| tool.name == "bounded_sql_query")
        .expect("bounded_sql_query is defined")
        .clone()
}

pub(crate) fn side_effecting_tool() -> ToolDefinition {
    ToolDefinition {
        name: "run_program".into(),
        description: "spawns a process outside the agent".into(),
        read_only: false,
        parameters: serde_json::json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        completion: None,
    }
}

/// A composition that carries what these tests' calls name: runner doors
/// over `bench`/`deploy`, the fetch member, and a bound workspace root —
/// the facts a session composes for those programs (U8: the suggestion
/// gates on the composition, so a test asserting an offer must stage the
/// capability it offers).
pub(crate) fn composed_facts() -> ApprovalFacts {
    ApprovalFacts {
        runner: Some(crate::approval_facts::RunnerFacts {
            runner_programs: vec!["bench".into(), "deploy".into()],
            interpreter_programs: Vec::new(),
            ..crate::approval_facts::RunnerFacts::default()
        }),
        fetch: Some(crate::approval_facts::FetchFacts {
            fetch_body_bytes: 61_440,
            fetch_seconds: 30,
            fetch_redirects: 5,
            download: None,
        }),
        workspace_root: Some(std::path::PathBuf::from("/home/user/proj")),
        ..ApprovalFacts::default()
    }
}
