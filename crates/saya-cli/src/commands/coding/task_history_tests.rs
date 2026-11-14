use super::blocked_by;
use crate::commands::coding::CodingTask;
use saya_types::{AgentId, JobState};
use std::collections::BTreeMap;

fn id(value: &str) -> AgentId {
    AgentId::parse(value).unwrap()
}

fn task(dependencies: &[&str]) -> CodingTask {
    CodingTask {
        agent_id: id("child-c"),
        subagent_type: "general".into(),
        task: "work".into(),
        write_paths: vec!["c.txt".into()],
        dependencies: dependencies.iter().map(|name| id(name)).collect(),
    }
}

/// A `tasks.jsonl` line written before `subagent_type` existed loads as
/// `general` — the write-capable shape every helper had — and a recorded
/// `explore` loads as itself.
#[test]
fn a_record_without_a_subagent_type_loads_as_general() {
    let legacy: CodingTask = serde_json::from_str(
        r#"{"agent_id":"child-c","task":"work","write_paths":["c.txt"],"dependencies":[]}"#,
    )
    .unwrap();
    assert_eq!(legacy.subagent_type, "general");
    assert_eq!(legacy.agent_id, id("child-c"));

    let explore: CodingTask =
        serde_json::from_str(r#"{"agent_id":"child-c","task":"look","subagent_type":"explore"}"#)
            .unwrap();
    assert_eq!(explore.subagent_type, "explore");
    assert!(explore.write_paths.is_empty());
    assert!(explore.dependencies.is_empty());
}

#[test]
fn an_unfinished_child_waits_on_dependencies_without_a_terminal_record() {
    let states = BTreeMap::from([
        (id("child-a"), JobState::Completed),
        (id("child-b"), JobState::Running),
    ]);
    let pending = task(&["child-a", "child-b", "child-never-started"]);
    let waiting = blocked_by(&pending, JobState::Queued, &states);
    assert_eq!(waiting, vec![&id("child-b"), &id("child-never-started")]);
}

#[test]
fn a_finished_child_waits_on_nothing() {
    let states = BTreeMap::from([(id("child-b"), JobState::Running)]);
    assert!(blocked_by(&task(&["child-b"]), JobState::Completed, &states).is_empty());
    assert!(blocked_by(&task(&[]), JobState::Running, &states).is_empty());
}
