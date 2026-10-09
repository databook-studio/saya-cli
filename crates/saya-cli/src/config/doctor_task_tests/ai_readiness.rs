#[test]
fn ai_without_a_key_reference_fails_credentials_for_a_cloud_provider() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert!(
        credentials.detail.contains("unauthenticated"),
        "{credentials:?}"
    );
    let provider = check_named(&readiness, "provider");
    assert_eq!(provider.status, CheckStatus::Ok);
    assert_eq!(provider.detail, "openai, model gpt-4o");
}

#[test]
fn ai_credentials_fail_names_the_redacted_reference_label_not_any_value() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Fail);
    assert!(
        credentials.detail.contains("env:SAYA_TEST_AI_KEY"),
        "{credentials:?}"
    );
}

#[test]
fn ai_credentials_resolve_from_secret_values_without_serializing_the_value() {
    const SENTINEL: &str = "sentinel-ai-key-never-print-this";
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai\"\nmodel = \"gpt-4o\"\napi_key = { env = \"SAYA_TEST_AI_KEY\" }\n",
        "",
        BTreeMap::from([("SAYA_TEST_AI_KEY".to_string(), SENTINEL.to_string())]),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    assert_eq!(
        check_named(&readiness, "credentials").status,
        CheckStatus::Ok
    );
    let serialized = serde_json::to_string(&readiness).unwrap();
    assert!(!serialized.contains(SENTINEL), "{serialized}");
}

#[test]
fn a_local_provider_is_ready_without_credentials() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"qwen2.5-coder:14b\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(readiness.ready);
    let credentials = check_named(&readiness, "credentials");
    assert_eq!(credentials.status, CheckStatus::Ok);
    assert_eq!(credentials.detail, "no credentials required for ollama");
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Ok);
    assert_eq!(endpoint.detail, "configured endpoint: localhost:11434");
}

#[test]
fn an_empty_model_fails_the_provider_check() {
    let runtime = runtime_from("[ai]\nmodel = \"\"\n", "", BTreeMap::new());
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    assert!(!readiness.ready);
    assert_eq!(
        check_named(&readiness, "provider").status,
        CheckStatus::Fail
    );
}

#[test]
fn an_endpointless_gateway_warns_without_failing() {
    // openai_compatible has no default endpoint and none configured: there is
    // no address to report, which is a warning, never a connection attempt.
    let runtime = runtime_from(
        "[ai]\nprovider = \"openai_compatible\"\nmodel = \"m\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert_eq!(endpoint.detail, "configured endpoint: none");
    // A warning never blocks readiness: only credentials decide here.
    assert!(readiness.ready);
}

#[test]
fn an_unparseable_base_url_warns_the_endpoint_check() {
    let runtime = runtime_from(
        "[ai]\nprovider = \"ollama\"\nmodel = \"m\"\nbase_url = \"not a url\"\n",
        "",
        BTreeMap::new(),
    );
    let readiness = readiness(&runtime, DoctorTask::Ai, Path::new("/"));
    let endpoint = check_named(&readiness, "endpoint");
    assert_eq!(endpoint.status, CheckStatus::Warn);
    assert!(
        endpoint.detail.contains("could not be parsed"),
        "{endpoint:?}"
    );
    // The raw base_url is never echoed: it may carry userinfo.
    assert!(!endpoint.detail.contains("not a url"));
}
