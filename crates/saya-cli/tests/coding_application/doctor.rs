use super::support::Fixture;

#[tokio::test]
async fn coding_doctor_reports_from_an_isolated_home_without_leaking_secrets() {
    let sentinel = "saya-doctor-sentinel-secret";
    let endpoint = format!("http://{sentinel}.invalid");
    let fixture = Fixture::with_endpoint("doctor-fresh-home", Some(&endpoint)).await;
    let report = fixture.application.doctor(false).await.unwrap();
    // A fresh install with a configured provider is ready: an unconfigured
    // MCP/LSP profile and an unrun probe are optional, not blocked.
    assert_eq!(report["ready"], true);
    assert_eq!(report["private_state_writable"], true);
    assert!(report["confinement_proven"].is_boolean());
    assert_eq!(report["native_provider_tool_loop"], serde_json::Value::Null);
    assert!(report["git"].is_string());
    assert!(report["git_compatibility"].is_object());
    assert!(report["git_compatibility"]["version"].is_string());
    assert!(report["git_compatibility"]["version_known"].is_boolean());
    assert!(report["git_compatibility"]["lazy_fetch_protection"].is_string());
    let blocked = report["blocked"].as_array().unwrap();
    assert!(
        blocked.is_empty(),
        "a fresh install has no missing required capability: {blocked:?}"
    );
    let optional = report["optional"].as_array().unwrap();
    assert!(optional.iter().any(|entry| {
        entry
            .as_str()
            .unwrap_or_default()
            .contains("Configure installed MCP and LSP profiles")
    }));
    assert!(optional.iter().any(|entry| {
        entry
            .as_str()
            .unwrap_or_default()
            .contains("Run saya job doctor --probe")
    }));
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(serialized.contains("endpoint_fingerprint"));
    assert!(!serialized.contains(sentinel));
    assert!(!serialized.contains(&endpoint));
}
