//! Reconcile-plan and reconcile-finish rendering tests.

use super::*;

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
