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
