//! Coordinator scenarios for a helper call that fails after admission, so only
//! the composition of a child's brief can refuse it. The batch is
//! mode-independent: the same script runs under a direct or a delegate-only
//! coordinator, and only the readiness view differs.
use super::render::{answer, tagged_tool, tools};
use serde_json::{Value, json};

pub(super) fn response(actor: &str, stage: usize, mode: u8, _request: &str) -> String {
    match (mode, actor == "parent") {
        (52..=54, _) => rollback(actor, stage, mode),
        _ => answer("unexpected convergence actor finished"),
    }
}

/// A control character passes admission, which only measures the assignment,
/// and is refused when the child's brief is composed.
const UNCOMPOSABLE: &str = "\u{1}";

fn task(agent: &str, composable: bool) -> Value {
    let flaw = if composable { "" } else { UNCOMPOSABLE };
    json!({"agent_id": agent, "task": format!("write for {agent}; owned candidate{flaw}")})
}

/// One `helpers` call per task, emitted in a single turn so the helpers run at
/// the same time. Each prompt carries `write for <agent>` so the transport
/// attributes the child's turns to that agent.
fn batch(tag: &str, tasks: &[Value]) -> String {
    let calls: Vec<(String, &str, Value)> = tasks
        .iter()
        .enumerate()
        .map(|(index, task)| {
            (
                format!("{tag}-{index}"),
                "helpers",
                json!({
                    "description": format!("hand piece {index} to a helper"),
                    "prompt": task["task"],
                    "subagent_type": "general",
                    "agent_id": task["agent_id"],
                }),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, Value)> = calls
        .iter()
        .map(|(id, name, args)| (id.as_str(), *name, args.clone()))
        .collect();
    tools(&borrowed)
}

fn rollback(actor: &str, stage: usize, mode: u8) -> String {
    if actor != "parent" {
        return child_write(stage, actor);
    }
    match (mode, stage) {
        (52, 0) => batch("delegate-bad", &[task("child-bad", false)]),
        // 53: both calls in the batch are composable, so under parallel helpers
        // Both are admitted and run at the same time; each returns its own
        // completed summary. The uncomposable-brief case is mode 52.
        (53, 0) => batch(
            "delegate-bad",
            &[task("child-a", true), task("child-b", true)],
        ),
        (54, 0) => batch(
            "delegate-bad",
            &[task("child-a", true), task("child-b", false)],
        ),
        (54, 1) => batch(
            "delegate-fixed",
            &[task("child-a", true), task("child-b", true)],
        ),
        (54, _) => answer("the corrected batch ran"),
        (_, 1) => tagged_tool("status-after", "read", json!({"path":"tracked.txt"})),
        _ => answer("nothing was delegated"),
    }
}

fn child_write(stage: usize, actor: &str) -> String {
    match stage {
        0 => tagged_tool(
            "write",
            "write",
            json!({"path":if actor == "child-a" { "new-a.txt" } else { "new-b.txt" },"content":format!("native {actor}\n")}),
        ),
        _ => answer("child work complete"),
    }
}
