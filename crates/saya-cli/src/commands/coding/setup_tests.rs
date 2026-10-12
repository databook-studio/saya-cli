use super::checklist;
use serde_json::json;

fn report(enabled: bool, probed: serde_json::Value) -> serde_json::Value {
    json!({
        "ready": enabled && probed == json!(true),
        "enabled": enabled,
        "git": "git version 2.45.0",
        "private_state_writable": true,
        "confinement_proven": true,
        "check_programs_installed": true,
        "native_provider_tool_loop": probed,
        "blocked": []
    })
}

fn status_of(list: &serde_json::Value, check: &str) -> (String, serde_json::Value) {
    let step = list["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["check"] == check)
        .unwrap();
    (
        step["status"].as_str().unwrap().into(),
        step["action"].clone(),
    )
}

#[test]
fn a_ready_report_has_no_blocked_step() {
    let list = checklist(&report(true, json!(true)));
    assert_eq!(list["ready"], true);
    assert!(
        list["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "ok" && step["action"].is_null())
    );
}

#[test]
fn a_disabled_feature_and_an_unprobed_model_each_name_their_own_action() {
    let list = checklist(&report(false, json!(null)));
    let (status, action) = status_of(&list, "coding_enabled");
    assert_eq!(status, "blocked");
    assert!(
        action
            .as_str()
            .unwrap()
            .contains("trusted user configuration")
    );
    let (status, action) = status_of(&list, "model_tool_loop");
    assert_eq!(status, "blocked");
    assert!(action.as_str().unwrap().contains("coding setup --probe"));
}

#[test]
fn a_probed_model_that_failed_asks_for_a_tool_capable_model() {
    let list = checklist(&report(true, json!(false)));
    let (status, action) = status_of(&list, "model_tool_loop");
    assert_eq!(status, "blocked");
    assert!(action.as_str().unwrap().contains("supports tool calls"));
    assert_eq!(list["ready"], false);
}

#[test]
fn every_failed_fact_blocks_exactly_its_own_step() {
    // (report key, step name): flipping one fact must block that step only.
    for (key, check) in [
        ("private_state_writable", "private_state"),
        ("confinement_proven", "command_confinement"),
        ("check_programs_installed", "check_programs"),
    ] {
        let mut broken = report(true, json!(true));
        broken[key] = json!(false);
        let list = checklist(&broken);
        for step in list["steps"].as_array().unwrap() {
            let expected = if step["check"] == check {
                "blocked"
            } else {
                "ok"
            };
            assert_eq!(step["status"], expected, "{key} -> {check}: {list}");
        }
    }
}

#[test]
fn a_git_that_failed_its_version_check_is_a_blocked_step() {
    let mut broken = report(true, json!(true));
    broken["git"] = json!(null);
    let list = checklist(&broken);
    let (status, action) = status_of(&list, "git");
    assert_eq!(status, "blocked");
    assert!(action.as_str().unwrap().contains("Install Git"));
    for step in list["steps"].as_array().unwrap() {
        if step["check"] != "git" {
            assert_eq!(step["status"], "ok", "only git is blocked: {list}");
        }
    }
}
