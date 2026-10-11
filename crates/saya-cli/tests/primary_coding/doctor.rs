use super::support::Fixture;

#[tokio::test]
async fn coding_doctor_reports_named_checks_from_an_isolated_home_without_leaking_secrets() {
    let sentinel = "saya-doctor-sentinel-secret";
    let endpoint = format!("http://{sentinel}.invalid");
    let fixture = Fixture::with_endpoint("doctor-fresh-home", Some(&endpoint)).await;
    let report = fixture.application.doctor(false).await.unwrap();
    assert_eq!(report["ready"], false);
    assert_eq!(report["enabled"], true);
    assert_eq!(report["private_state_writable"], true);
    assert_eq!(report["check_programs_installed"], true);
    assert!(report["confinement_proven"].is_boolean());
    assert_eq!(report["native_provider_tool_loop"], serde_json::Value::Null);
    assert!(report["git"].is_string());
    let blocked = report["blocked"].as_array().unwrap();
    assert!(blocked.iter().any(|entry| {
        entry
            .as_str()
            .unwrap_or_default()
            .contains("Configure installed MCP and LSP profiles")
    }));
    assert!(blocked.iter().any(|entry| {
        entry
            .as_str()
            .unwrap_or_default()
            .contains("Run coding doctor --probe")
    }));
    let serialized = serde_json::to_string(&report).unwrap();
    assert!(serialized.contains("endpoint_fingerprint"));
    assert!(!serialized.contains(sentinel));
    assert!(!serialized.contains(&endpoint));
}
