use super::*;
use crate::render::TerminalEvent;

fn requested(name: &str, arguments: serde_json::Value) -> AgentEvent {
    AgentEvent::ToolRequested {
        name: name.to_owned(),
        arguments,
        effect: None,
    }
}

fn completed(name: &str) -> AgentEvent {
    AgentEvent::ToolCompleted {
        name: name.to_owned(),
        summary: "did it".to_owned(),
    }
}

fn denied(name: &str) -> AgentEvent {
    AgentEvent::ToolDenied {
        name: name.to_owned(),
        reason: "refused".to_owned(),
    }
}

fn tally(events: &[AgentEvent]) -> ToolTally {
    let mut tally = ToolTally::default();
    for event in events {
        tally.record(event);
    }
    tally
}

#[cfg(test)]
#[path = "turn_summary_tests/settlement.rs"]
mod settlement;
#[cfg(test)]
#[path = "turn_summary_tests/tool_tally.rs"]
mod tool_tally;
#[cfg(test)]
#[path = "turn_summary_tests/wire_shape.rs"]
mod wire_shape;
