//! The direct root's task list is appended to the transcript once: the turn
//! the loop assembles after the `checklist` call that changed it. Every later
//! turn whose list is unchanged appends nothing — the model already has the
//! state, from that block and from its own `checklist` result in the
//! transcript.
//!
//! The note is appended after everything already sent and never removed: a
//! prompt cache is a prefix cache, so rewriting an earlier note would turn
//! every later request into a cache miss.

use super::direct_task::run_direct;
use serde_json::Value;

/// The block's own source line — what a request carries when it renders the
/// list, and what an unchanged turn must not carry.
const TASKS_MARKER: &str = "source: session-tasks";

/// Every captured request's messages, in request order.
fn requests(provider: &super::provider::Provider) -> Vec<Vec<Value>> {
    provider
        .bodies
        .lock()
        .unwrap()
        .iter()
        .map(|body| {
            let request: Value = serde_json::from_str(body).expect("a captured request is JSON");
            request["messages"].as_array().cloned().unwrap_or_default()
        })
        .collect()
}

/// The messages of `messages` that carry the task-list block.
fn carrying(messages: &[Value]) -> Vec<&Value> {
    messages
        .iter()
        .filter(|message| {
            message["content"]
                .as_str()
                .is_some_and(|content| content.contains(TASKS_MARKER))
        })
        .collect()
}

#[tokio::test]
async fn the_task_list_is_appended_once_and_never_rewritten() {
    let (_fixture, _application, _job, provider) = run_direct(
        "direct-checklist-delta",
        112,
        vec!["tracked.txt".into()],
        vec![],
    )
    .await;

    // The write landed: the tool's own result is the stored count, and it is
    // in the transcript of every request after the call.
    assert_eq!(
        super::direct_task::tool_result(&provider, "call-checklist"),
        serde_json::json!({"tasks": 2}),
        "the scripted checklist call stored the two-task list"
    );

    let requests = requests(&provider);
    assert_eq!(
        requests.len(),
        5,
        "one checklist call, three reads that change nothing, one answer"
    );
    let carriers: Vec<usize> = requests
        .iter()
        .enumerate()
        .filter(|(_, messages)| !carrying(messages).is_empty())
        .map(|(index, _)| index)
        .collect();
    assert_eq!(
        carriers,
        vec![1, 2, 3, 4],
        "the note is appended after the change and stays in the transcript: {requests:#?}"
    );
    for (index, messages) in requests.iter().enumerate().skip(1) {
        assert_eq!(
            carrying(messages).len(),
            1,
            "request {index} carries the block once, not once per message"
        );
    }
    assert!(
        carrying(&requests[1])[0]["content"]
            .as_str()
            .is_some_and(|content| content.contains("parse records")),
        "the block carries the stored list: {}",
        carrying(&requests[1])[0]
    );

    // The cached prefix is untouched: every request begins, message for
    // message, with the request before it. The note is appended at the end and
    // never moved or removed.
    for (index, pair) in requests.windows(2).enumerate() {
        assert!(
            pair[1].starts_with(&pair[0]),
            "request {} replays request {index}'s messages unchanged: {:?} vs {:?}",
            index + 1,
            pair[1],
            pair[0]
        );
    }
}
