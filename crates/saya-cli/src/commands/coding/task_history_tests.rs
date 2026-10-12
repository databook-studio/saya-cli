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
        task: "work".into(),
        write_paths: vec!["c.txt".into()],
        dependencies: dependencies.iter().map(|name| id(name)).collect(),
    }
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
