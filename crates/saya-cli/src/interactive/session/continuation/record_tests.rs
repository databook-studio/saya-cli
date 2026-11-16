//! Tests for the deterministic continuation-fact collector and builder.

use super::{SessionObservations, facts::build_continuation_facts};
use crate::SlashCommand;
use crate::interactive::session::state::SessionState;
use saya_agent::{AgentEvent, AgentOutput, TokenUsage, ToolMetadata};
use saya_store::RedactedTurn;
use saya_types::{CONTINUATION_VERSION, CheckOutcome, SessionContinuationV1};

/// A settled live output with the given answer, tool metadata, and events.
fn output(answer: &str, tool_metadata: Vec<ToolMetadata>, events: Vec<AgentEvent>) -> AgentOutput {
    AgentOutput {
        answer: answer.into(),
        events,
        used_bounded_sql_query: false,
        tool_metadata,
        usage: TokenUsage::default(),
        learning_usage: None,
        truncated: false,
        answer_sql: None,
    }
}

fn tool(name: &str, status: &str, arguments: &str) -> ToolMetadata {
    ToolMetadata {
        name: name.into(),
        status: status.into(),
        arguments: arguments.into(),
        result_shape: None,
    }
}

fn turn(user: &str, assistant: &str) -> RedactedTurn {
    RedactedTurn {
        user: user.into(),
        assistant: assistant.into(),
        database_derived: false,
        tools: Vec::new(),
    }
}

fn requested_run_command(arguments: serde_json::Value) -> AgentEvent {
    AgentEvent::ToolRequested {
        name: "run_command".into(),
        arguments,
        effect: None,
    }
}

fn completed_run_command(summary: impl Into<String>) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: "run_command".into(),
        summary: summary.into(),
    }
}

const NOW: i64 = 1_700_000_000_000;

include!("record_tests/file_observations.rs");

include!("record_tests/command_checks.rs");

include!("record_tests/builder_and_clear.rs");
