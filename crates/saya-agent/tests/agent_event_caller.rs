//! `caller` on the tool events: which helper made the call.
//!
//! `02` §3.6 draws a helper's work as one block per helper, and the block is
//! grouped by the optional `caller` on the tool events, so an event in the
//! parent's stream has to say which helper's loop emitted it. A root event has
//! no caller, and that absence has to stay invisible: the first three tests
//! compare each event's bytes against the shape it had before the field
//! existed, which is what a pipe reader already parses.

use saya_agent::{AgentEvent, ToolEffect};
use serde::Serialize;
use serde_json::{Value, json};

/// [`AgentEvent`]'s three tool variants as they serialised before `caller`
/// existed: the same tag, the same field names, the same declaration order, and
/// the same two conditional fields. Serialising one of these beside the event
/// it mirrors is the byte-identity pin — the events must not gain a key, and a
/// `caller: None` must not surface as `null`.
#[derive(Serialize)]
#[serde(tag = "type")]
enum BeforeTheField {
    #[serde(rename = "tool_requested")]
    Requested {
        name: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        arguments: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effect: Option<ToolEffect>,
    },
    #[serde(rename = "tool_completed")]
    Completed { name: String, summary: String },
    #[serde(rename = "tool_denied")]
    Denied {
        name: String,
        reason: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        arguments: Value,
    },
}

/// The event's own wire bytes, as a root's sink would write them.
fn wire(event: &AgentEvent) -> String {
    serde_json::to_string(event).expect("an agent event serialises")
}

fn effect() -> ToolEffect {
    ToolEffect {
        database_data: false,
        external_side_effect: false,
        requires_approval: false,
        local_state: saya_agent::LocalStateEffect::None,
    }
}

/// A root's request keeps every byte it had: no `caller`, and the declared
/// effect still travels in its own place.
#[test]
fn agent_event_root_tool_requested_is_byte_identical() {
    let event = AgentEvent::ToolRequested {
        name: "read".into(),
        arguments: json!({"path": "src/lib.rs"}),
        effect: Some(effect()),
        caller: None,
    };
    let before = BeforeTheField::Requested {
        name: "read".into(),
        arguments: json!({"path": "src/lib.rs"}),
        effect: Some(effect()),
    };
    assert_eq!(
        wire(&event),
        serde_json::to_string(&before).expect("the old shape serialises"),
        "a root request must not gain a caller key"
    );
}

/// A root's completion keeps every byte it had.
#[test]
fn agent_event_root_tool_completed_is_byte_identical() {
    let event = AgentEvent::ToolCompleted {
        name: "read".into(),
        summary: "read 12 lines".into(),
        caller: None,
    };
    let before = BeforeTheField::Completed {
        name: "read".into(),
        summary: "read 12 lines".into(),
    };
    assert_eq!(
        wire(&event),
        serde_json::to_string(&before).expect("the old shape serialises"),
        "a root completion must not gain a caller key"
    );
    assert_eq!(
        wire(&event),
        r#"{"type":"tool_completed","name":"read","summary":"read 12 lines"}"#,
        "and the line itself is exactly the one a pipe already parses"
    );
}

/// A root's denial keeps every byte it had, arguments included.
#[test]
fn agent_event_root_tool_denied_is_byte_identical() {
    let event = AgentEvent::ToolDenied {
        name: "shell".into(),
        reason: "the workspace refuses it".into(),
        arguments: json!({"command": "rm -rf"}),
        caller: None,
    };
    let before = BeforeTheField::Denied {
        name: "shell".into(),
        reason: "the workspace refuses it".into(),
        arguments: json!({"command": "rm -rf"}),
    };
    assert_eq!(
        wire(&event),
        serde_json::to_string(&before).expect("the old shape serialises"),
        "a root denial must not gain a caller key"
    );
}

/// A helper's forwarded event names the helper, and the name survives a trip
/// through the wire in the field's own place — last, beside the payload.
#[test]
fn agent_event_carries_a_helpers_id_and_round_trips() {
    let forwarded = [
        AgentEvent::ToolRequested {
            name: "read".into(),
            arguments: json!({"path": "src/lib.rs"}),
            effect: None,
            caller: Some("helper-1".into()),
        },
        AgentEvent::ToolCompleted {
            name: "read".into(),
            summary: "read 12 lines".into(),
            caller: Some("helper-1".into()),
        },
        AgentEvent::ToolDenied {
            name: "shell".into(),
            reason: "the helper may not run commands".into(),
            arguments: Value::Null,
            caller: Some("helper-1".into()),
        },
    ];
    for event in forwarded {
        let text = wire(&event);
        let parsed: Value = serde_json::from_str(&text).expect("the line is JSON");
        assert_eq!(
            parsed["caller"], "helper-1",
            "the helper's event names it: {text}"
        );
        let back: AgentEvent = serde_json::from_str(&text).expect("the line reads back");
        assert_eq!(back, event, "the helper's event round-trips: {text}");
    }
}

/// A line written before the field existed reads back as the root's own event,
/// so an unreplayed or old stream is not mistaken for a helper's.
#[test]
fn agent_event_without_a_caller_reads_back_as_the_root() {
    let legacy = r#"{"type":"tool_denied","name":"shell","reason":"refused"}"#;
    let event: AgentEvent = serde_json::from_str(legacy).expect("the old line still reads");
    assert_eq!(
        event,
        AgentEvent::ToolDenied {
            name: "shell".into(),
            reason: "refused".into(),
            arguments: Value::Null,
            caller: None,
        },
        "an absent caller is the root, not an empty helper"
    );
}
