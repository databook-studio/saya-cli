//! Review, apply, and setup rendering tests.

use super::*;

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
        "apply_requires_fresh_approval":true
    })
}

#[test]
fn review_lists_changes_and_the_exact_apply_command() {
    let text = render_coding_result(&review_command(), &preview());
    assert!(
        text.contains("Review of candidate cand-1 for job job-1"),
        "{text}"
    );
    assert!(text.contains("3 changed files"), "{text}");
    assert!(text.contains("modified  src/lib.rs (120 bytes)"), "{text}");
    assert!(text.contains("added     src/new.rs (40 bytes)"), "{text}");
    assert!(text.contains("deleted   old.txt"), "{text}");
    assert!(
        text.contains(&format!(
            "saya job apply job-1 cand-1 --source-sha256 {} --approve",
            "a".repeat(64)
        )),
        "the apply command carries the exact reviewed digest: {text}"
    );
    assert!(
        text.contains("Apply re-checks that the candidate and your original tree are unchanged"),
        "the review states the freshness the apply re-checks: {text}"
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
            {"check":"git","status":"ok","action":null},
            {"check":"model_tool_loop","status":"blocked","action":"Run `saya job setup --probe` to let saya make one bounded call"}
        ],
        "blocked": []
    });
    let text = render_coding_result(&CodingCommand::Setup { probe: false }, &value);
    assert!(text.contains("Job setup: 1 step needs action"), "{text}");
    assert!(text.contains("[ok]       git"), "{text}");
    assert!(
        text.contains("[blocked]  model_tool_loop: Run `saya job setup --probe`"),
        "{text}"
    );
    assert!(
        text.contains("Nothing was installed, enabled or written."),
        "{text}"
    );

    let ready = json!({"ready":true,"installs_nothing":true,"steps":[{"check":"git","status":"ok","action":null}],"blocked":[]});
    let text = render_coding_result(&CodingCommand::Setup { probe: true }, &ready);
    assert!(text.contains("Job setup: ready"), "{text}");
}
