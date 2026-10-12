use super::render_coding_result;
use crate::cli::CodingCommand;
use serde_json::json;

fn review_command() -> CodingCommand {
    CodingCommand::Review {
        id: "job-1".into(),
        candidate: "cand-1".into(),
        approve: true,
    }
}

fn preview() -> serde_json::Value {
    json!({
        "candidate":"cand-1",
        "source_sha256":"a".repeat(64),
        "changes":[
            {"path":"src/lib.rs","kind":"Modified","bytes":120},
            {"path":"src/new.rs","kind":"Added","bytes":40},
            {"path":"old.txt","kind":"Deleted","bytes":0}
        ],
        "diff":{"files":[],"total_files":3,"bounded":true,"format":"before_after"},
        "checks":[
            {"profile":"lint","outcome":"succeeded","successful":true},
            {"profile":"tests","outcome":"succeeded","successful":true}
        ],
        "checks_configured":true,
        "diagnostics_configured":false,
        "diagnostic_delta":{"available":true,"introduced_count":2,"unchanged_count":5,"resolved_count":1},
        "apply_requires_fresh_approval":true
    })
}

#[test]
fn review_lists_changes_checks_and_the_exact_apply_command() {
    let text = render_coding_result(&review_command(), &preview());
    assert!(
        text.contains("Review of candidate cand-1 for job job-1"),
        "{text}"
    );
    assert!(text.contains("3 changed files"), "{text}");
    assert!(text.contains("modified  src/lib.rs (120 bytes)"), "{text}");
    assert!(text.contains("added     src/new.rs (40 bytes)"), "{text}");
    assert!(text.contains("deleted   old.txt"), "{text}");
    assert!(text.contains("lint: succeeded"), "{text}");
    assert!(text.contains("tests: succeeded"), "{text}");
    assert!(
        text.contains("2 introduced, 5 unchanged, 1 resolved"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "/coding apply job-1 cand-1 --source-sha256 {} --approve",
            "a".repeat(64)
        )),
        "the apply command carries the exact reviewed digest: {text}"
    );
}

#[test]
fn review_without_passing_checks_says_apply_will_be_refused() {
    let mut value = preview();
    value["checks"] = json!([]);
    value["checks_configured"] = json!(false);
    let text = render_coding_result(&review_command(), &value);
    assert!(text.contains("No checks are configured"), "{text}");
    assert!(
        text.contains("Apply is refused until successful configured checks exist"),
        "{text}"
    );
}

#[test]
fn a_failed_check_blocks_the_apply_line() {
    let mut value = preview();
    value["checks"][1] = json!({"profile":"tests","outcome":"failed","successful":false});
    let text = render_coding_result(&review_command(), &value);
    assert!(!text.contains("/coding apply"), "{text}");
    assert!(
        text.contains("Apply will be refused until every check passes (not passing: tests)"),
        "{text}"
    );
}

#[test]
fn apply_reports_the_digest_and_leaves_other_commands_as_json() {
    let apply = CodingCommand::Apply {
        id: "job-1".into(),
        candidate: "cand-1".into(),
        source_sha256: "b".repeat(64),
        approve: true,
    };
    let text = render_coding_result(
        &apply,
        &json!({"applied":true,"source_sha256":"b".repeat(64)}),
    );
    assert!(
        text.contains("Applied candidate cand-1 to the original checkout"),
        "{text}"
    );
    assert!(text.contains(&"b".repeat(12)), "{text}");

    let other = CodingCommand::List { after: None };
    let value = json!({"jobs":[]});
    assert_eq!(render_coding_result(&other, &value), value.to_string());
}

#[test]
fn an_unexpected_review_shape_falls_back_to_the_json() {
    let value = json!({"unexpected":true});
    assert_eq!(
        render_coding_result(&review_command(), &value),
        value.to_string()
    );
}

#[test]
fn setup_lists_every_step_and_the_action_for_each_blocked_one() {
    let value = json!({
        "ready": false,
        "installs_nothing": true,
        "steps": [
            {"check":"coding_enabled","status":"ok","action":null},
            {"check":"model_tool_loop","status":"blocked","action":"Run `coding setup --probe` to let saya make one bounded call"}
        ],
        "blocked": []
    });
    let text = render_coding_result(&CodingCommand::Setup { probe: false }, &value);
    assert!(text.contains("Coding setup: 1 step needs action"), "{text}");
    assert!(text.contains("[ok]       coding_enabled"), "{text}");
    assert!(
        text.contains("[blocked]  model_tool_loop: Run `coding setup --probe`"),
        "{text}"
    );
    assert!(
        text.contains("Nothing was installed, enabled or written."),
        "{text}"
    );

    let ready = json!({"ready":true,"installs_nothing":true,"steps":[{"check":"git","status":"ok","action":null}],"blocked":[]});
    let text = render_coding_result(&CodingCommand::Setup { probe: true }, &ready);
    assert!(text.contains("Coding setup: ready"), "{text}");
}

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

#[test]
fn an_empty_check_list_never_prints_the_apply_command() {
    let mut value = preview();
    value["checks"] = json!([]);
    value["checks_configured"] = json!(true);
    let text = render_coding_result(&review_command(), &value);
    assert!(!text.contains("/coding apply"), "{text}");
    assert!(
        text.contains("Apply is refused until successful configured checks exist"),
        "{text}"
    );
}

fn reconcile_command() -> CodingCommand {
    CodingCommand::Reconcile {
        id: "job-1".into(),
        plan_sha256: None,
        approve: false,
    }
}

#[test]
fn a_reconcile_plan_lists_each_effect_and_the_exact_approving_command() {
    let digest = "d".repeat(64);
    let value = json!({
        "job_id":"job-1","last_sequence":7,"plan_sha256":digest,"changes_nothing":true,
        "effects":[
            {"reservation_sequence":4,"settled_uncertain":false,"reserved":[
                {"dimension":"logical_requests","endpoint":null,"price_metadata_sha256":null,"reserved":1}]},
            {"reservation_sequence":6,"settled_uncertain":true,"reserved":[
                {"dimension":"tool_calls","endpoint":null,"price_metadata_sha256":null,"reserved":2}]}
        ]
    });
    let text = render_coding_result(&reconcile_command(), &value);
    assert!(
        text.contains("2 unresolved effects for job job-1"),
        "{text}"
    );
    assert!(
        text.contains("reservation 4  open  logical_requests 1"),
        "{text}"
    );
    assert!(
        text.contains("reservation 6  settled uncertain  tool_calls 2"),
        "{text}"
    );
    assert!(text.contains("nothing is refunded"), "{text}");
    assert!(
        text.contains(&format!(
            "/coding reconcile job-1 --plan-sha256 {digest} --approve"
        )),
        "{text}"
    );

    let empty = json!({"job_id":"job-1","last_sequence":7,"plan_sha256":digest,"effects":[],"changes_nothing":true});
    let text = render_coding_result(&reconcile_command(), &empty);
    assert!(
        text.contains("Nothing is unresolved for job job-1"),
        "{text}"
    );
    assert!(
        !text.contains("--plan-sha256"),
        "no reconcile command for an empty plan: {text}"
    );
}

#[test]
fn a_finished_reconcile_says_nothing_was_refunded_and_points_at_resume() {
    let value = json!({"job_id":"job-1","plan_sha256":"d".repeat(64),"reconciled":[4,6],"refunded":false,
        "next_action":"coding resume job-1 --approve"});
    let text = render_coding_result(&reconcile_command(), &value);
    assert!(
        text.contains("Reconciled 2 effects for job job-1 (reservations 4, 6)"),
        "{text}"
    );
    assert!(text.contains("nothing was refunded"), "{text}");
    assert!(text.contains("/coding resume job-1 --approve"), "{text}");
}
