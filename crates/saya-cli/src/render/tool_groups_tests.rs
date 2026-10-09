//! C1 properties: the shared grouper + shaper over `AgentEvent` streams.
//!
//! Each named test pins one slice property; the trailing extras cover what
//! the list misses and say why.

use super::{group_tool_events, is_failure_summary, shape_group};
use saya_agent::{AgentEvent, LocalStateEffect, ToolEffect};

fn write_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    })
}

fn read_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: true,
        external_side_effect: false,
        requires_approval: false,
        local_state: LocalStateEffect::Read,
    })
}

fn run_effect() -> Option<ToolEffect> {
    Some(ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: false,
        local_state: LocalStateEffect::WriteWorkspace,
    })
}

fn requested(name: &str, arguments: serde_json::Value, effect: Option<ToolEffect>) -> AgentEvent {
    AgentEvent::tool_requested(name, arguments, effect)
}

fn completed(name: &str, summary: &str) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: name.into(),
        summary: summary.into(),
    }
}

#[path = "tool_groups_tests/failure_status.rs"]
mod failure_status;
#[path = "tool_groups_tests/group_boundaries.rs"]
mod group_boundaries;
#[path = "tool_groups_tests/grouping.rs"]
mod grouping;
