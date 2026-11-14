//! Managed-worktree and child-agent view rendering tests.

use super::*;

#[test]
fn worktrees_list_each_retained_checkout_with_its_lifecycle() {
    let value = json!({"checkouts":[
        {"checkout_id":"cand-1","lifecycle":"Retained"},
        {"checkout_id":"cand-2","lifecycle":"Active"}
    ],"next_cursor":null});
    let text = render_coding_result(&CodingCommand::Worktrees { id: "job-1".into() }, &value);
    assert!(text.contains("2 managed checkouts for job job-1"), "{text}");
    assert!(text.contains("cand-1  Retained"), "{text}");
    assert!(text.contains("cand-2  Active"), "{text}");
    let more = json!({"checkouts":[],"next_cursor":"x"});
    let text = render_coding_result(&CodingCommand::Worktrees { id: "job-1".into() }, &more);
    assert!(text.contains("more checkouts exist"), "{text}");
}

#[test]
fn show_lists_the_job_state_and_each_child_agent() {
    let value = json!({"job_id":"job-1","state":"running","children":[
        {"agent_id":"child-a","state":"running","invocation_id":"inv-1"},
        {"agent_id":"child-b","state":"queued","invocation_id":"inv-2"}
    ],"next_cursor":null});
    let text = render_coding_result(&CodingCommand::Show { id: "job-1".into() }, &value);
    assert!(
        text.contains("Job job-1: running, 2 child agents"),
        "{text}"
    );
    assert!(text.contains("child-a  running"), "{text}");
    assert!(text.contains("child-b  queued"), "{text}");
    let empty = json!({"job_id":"job-1","state":"completed","children":[],"next_cursor":null});
    let text = render_coding_result(&CodingCommand::Show { id: "job-1".into() }, &empty);
    assert!(
        text.contains("Job job-1: completed, no child agents"),
        "{text}"
    );
}

#[test]
fn show_names_owned_paths_and_what_a_child_is_waiting_on() {
    let value = json!({"job_id":"job-1","state":"running","children":[
        {"agent_id":"child-a","state":"running","write_paths":["a.txt","docs"],"blocked_by":[]},
        {"agent_id":"child-b","state":"queued","write_paths":["b.txt"],"blocked_by":["child-a"]}
    ],"next_cursor":null});
    let text = render_coding_result(&CodingCommand::Show { id: "job-1".into() }, &value);
    assert!(
        text.contains("child-a  running  owns a.txt, docs"),
        "{text}"
    );
    assert!(
        text.contains("child-b  queued  owns b.txt  waiting on child-a"),
        "{text}"
    );
}
