use super::checklist;
use serde_json::json;

fn report(probed: serde_json::Value) -> serde_json::Value {
    json!({
        // An unrun probe is optional; only a probe that actually failed makes
        // the install unready.
        "ready": probed != json!(false),
        "git": "git version 2.45.0",
        "private_state_writable": true,
        "confinement_proven": true,
        "native_provider_tool_loop": probed,
        "blocked": [],
        "optional": []
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
    let list = checklist(&report(json!(true)));
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
fn an_unprobed_model_is_optional_and_names_its_own_action() {
    let list = checklist(&report(json!(null)));
    let (status, action) = status_of(&list, "model_tool_loop");
    assert_eq!(status, "optional");
    assert!(action.as_str().unwrap().contains("saya job setup --probe"));
    assert_eq!(list["ready"], true, "an unrun probe does not block");
}

#[test]
fn a_probed_model_that_failed_asks_for_a_tool_capable_model() {
    let list = checklist(&report(json!(false)));
    let (status, action) = status_of(&list, "model_tool_loop");
    assert_eq!(status, "blocked");
    assert!(action.as_str().unwrap().contains("supports tool calls"));
    assert_eq!(list["ready"], false);
}

#[test]
fn every_failed_fact_blocks_exactly_its_own_step() {
    // (report key, step name): flipping one fact must block that step only.
    // Host command confinement is a macOS capability; on Linux the same
    // failed fact leaves the step optional rather than blocking the install.
    for (key, check) in [
        ("private_state_writable", "private_state"),
        ("confinement_proven", "command_confinement"),
    ] {
        let mut broken = report(json!(true));
        broken[key] = json!(false);
        let list = checklist(&broken);
        let failed_status = if key == "confinement_proven" && !cfg!(target_os = "macos") {
            "optional"
        } else {
            "blocked"
        };
        for step in list["steps"].as_array().unwrap() {
            let expected = if step["check"] == check {
                failed_status
            } else {
                "ok"
            };
            assert_eq!(step["status"], expected, "{key} -> {check}: {list}");
        }
    }
}

#[test]
fn a_git_that_failed_its_version_check_is_a_blocked_step() {
    let mut broken = report(json!(true));
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
