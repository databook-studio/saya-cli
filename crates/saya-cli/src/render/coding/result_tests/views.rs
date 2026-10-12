//! Retained-conflict, managed-worktree, and child-agent view rendering tests.

use super::*;

fn conflicts_command() -> CodingCommand {
    CodingCommand::Conflicts {
        id: "job-1".into(),
        candidate: "cand-1".into(),
    }
}

#[test]
fn a_retained_conflict_names_both_ways_out_and_grants_nothing() {
    let sha = "c".repeat(64);
    let value = json!({
        "job_id":"job-1","candidate_checkout_id":"cand-1","status":"retained",
        "authority":"none","fresh_revalidation_required":true,
        "manual_edit":{"review_command":"coding review-existing job-1 cand-1 --approve"},
        "provenance":{"record_sha256":sha},
        "conflict":{}
    });
    let text = render_coding_result(&conflicts_command(), &value);
    assert!(
        text.contains("Conflict retained for candidate cand-1 (job job-1)"),
        "{text}"
    );
    assert!(text.contains("grants no authority"), "{text}");
    assert!(
        text.contains(&format!(
            "/coding resolve-conflict job-1 cand-1 --choice keep-first --record-sha256 {sha} --approve"
        )),
        "{text}"
    );
    assert!(text.contains("keep-second"), "{text}");
    assert!(
        text.contains("coding review-existing job-1 cand-1 --approve"),
        "the hand-edit route starts from the owner's review command: {text}"
    );
}

#[test]
fn a_conflict_that_is_not_retained_says_so() {
    let value = json!({"job_id":"job-1","candidate_checkout_id":"cand-1","status":"not_retained","conflict":null});
    let text = render_coding_result(&conflicts_command(), &value);
    assert!(
        text.contains("No conflict is retained for candidate cand-1"),
        "{text}"
    );
}

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
