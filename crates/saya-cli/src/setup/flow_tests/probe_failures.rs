// ---------------------------------------------------------------- probes ----

#[tokio::test]
async fn setup_timeout_names_failed_probe() {
    let result = database_with(std::time::Duration::from_millis(30), || {
        std::future::pending()
    })
    .await;
    assert!(!result.ok, "a never-resolving connection fails");
    assert!(
        result.message.contains("database"),
        "the message names the database probe: {}",
        result.message
    );
    assert!(
        result.message.contains("timed out after 0.0s"),
        "the message names the timeout: {}",
        result.message
    );
}

#[tokio::test]
async fn provider_probe_timeout_is_named() {
    let result = provider_with(std::time::Duration::from_millis(30), || {
        std::future::pending::<Result<(), String>>()
    })
    .await;
    assert!(!result.ok);
    assert!(
        result.message.contains("provider probe timed out"),
        "{}",
        result.message
    );
}

#[tokio::test]
async fn probe_failure_keeps_the_underlying_message() {
    let result = database_with(std::time::Duration::from_millis(30), || async {
        Err("auth failed: bad password".to_string())
    })
    .await;
    assert!(!result.ok);
    assert!(
        result.message.contains("auth failed: bad password"),
        "the error message is always included: {}",
        result.message
    );
}

/// The real probe path with a secret that cannot be resolved: the factory
/// refuses at build time, offline, and the failure names the cause.
#[tokio::test]
async fn database_probe_reports_an_unresolvable_secret() {
    let profile = DatabaseProfile::Snowflake {
        account: "org-account".into(),
        user: "jane".into(),
        auth_type: SnowflakeAuth::Keypair,
        private_key: Some(SecretRef::Env {
            env: "SAYA_SETUP_TEST_MISSING_KEY".into(),
        }),
        password: None,
        passphrase: None,
        warehouse: None,
        database: None,
        schema: None,
        role: None,
    };
    let resolver = saya_config::MapSecretResolver::new(BTreeMap::new());
    let result = database(&profile, &resolver).await;
    assert!(!result.ok, "an unresolvable secret fails the probe");
    assert!(
        result.message.contains("could not be resolved"),
        "the cause is named: {}",
        result.message
    );
}

#[test]
fn provider_probe_sends_no_database_data() {
    let request = ping_request("test-model");
    assert_eq!(request.messages.len(), 1, "exactly one message");
    assert_eq!(
        request.messages[0].role, "user",
        "a user message, not system"
    );
    assert_eq!(request.messages[0].content, "ping", "the word ping, only");
    assert!(request.tools.is_empty(), "no tool definitions attached");
    assert!(
        !request
            .messages
            .iter()
            .any(|message| message.role == "system"),
        "no system message, so no schema can ride along"
    );
    assert_eq!(request.model, "test-model");
}

#[test]
fn connection_errors_classify_best_effort() {
    let classified = classify("password authentication failed for user");
    assert!(classified.contains("authentication failed"), "{classified}");
    let classified = classify("connection timed out");
    assert!(classified.contains("timed out"), "{classified}");
    let classified = classify("SSL error: certificate verify failed");
    assert!(classified.contains("tls"), "{classified}");
    let classified = classify("database \"nope\" does not exist");
    assert!(classified.contains("not found"), "{classified}");
    let classified = classify("something odd happened");
    assert!(classified.contains("could not connect"), "{classified}");
    assert!(
        classified.contains("something odd happened"),
        "the original error is always included: {classified}"
    );
}
