use super::support::Fixture;
use serde_json::Value;

fn step<'a>(report: &'a Value, check: &str) -> &'a Value {
    report["steps"]
        .as_array()
        .unwrap()
        .iter()
        .find(|step| step["check"] == check)
        .unwrap_or_else(|| panic!("no step named {check}: {report}"))
}

#[tokio::test]
async fn fresh_home_coding_setup_separates_optional_from_blocked_and_prints_no_secret() {
    let sentinel = "saya-setup-sentinel-secret";
    let endpoint = format!("http://{sentinel}.invalid");
    let fixture = Fixture::with_endpoint("setup-fresh-home", Some(&endpoint)).await;
    let report = fixture.application.setup(false).await.unwrap();

    // A fresh install with a configured provider is ready: an unrun probe and
    // an unconfigured MCP/LSP profile are optional, not blocked.
    assert_eq!(report["ready"], true);
    assert_eq!(report["installs_nothing"], true);
    assert_eq!(step(&report, "git")["status"], "ok");
    assert_eq!(step(&report, "private_state")["status"], "ok");
    let model = step(&report, "model_tool_loop");
    assert_eq!(model["status"], "optional");
    assert!(
        model["action"]
            .as_str()
            .unwrap()
            .contains("saya job setup --probe"),
        "the optional model step names the one action that clears it: {model}"
    );
    assert!(report["blocked"].as_array().unwrap().is_empty());
    assert!(!report["optional"].as_array().unwrap().is_empty());
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(
        !serialized.contains(sentinel) && !serialized.contains("endpoint"),
        "setup never carries endpoints or secrets: {serialized}"
    );
}
