use super::browser_support::*;
use super::*;

#[tokio::test]
async fn external_browser_exchanges_fragmented_callback_caches_and_reauthenticates() {
    let _serial = browser_test_guard().await;
    let (origin, seen) = sso_fixture_server().await;
    let mut item = connector(Auth::ExternalBrowser(ExternalBrowser {
        enabled: true,
        token: Arc::new(Mutex::new(None)),
    }));
    item.origin = origin;
    item.browser_opener = record_browser_opener;
    item.sso_timeout = Duration::from_secs(2);
    item.context = Context {
        warehouse: Some("WH".into()),
        database: Some("DB".into()),
        schema: Some("SCH".into()),
        role: Some("ROLE".into()),
    };
    let first = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    let second = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    assert_eq!(first.rows, rows(&[7]));
    assert_eq!(second.rows, rows(&[7]));
    let requests = seen.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|item| item.starts_with("POST /session/authenticator-request"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|item| item.starts_with("POST /session/v1/login-request"))
            .count(),
        2
    );
    let exchange: Value = serde_json::from_str(request_body(&requests[1])).unwrap();
    assert_eq!(exchange["data"]["ACCOUNT_NAME"], "acct");
    assert_eq!(exchange["data"]["LOGIN_NAME"], "user");
    assert_eq!(exchange["data"]["TOKEN"], "callback-1+reserved&value");
    assert_eq!(exchange["data"]["PROOF_KEY"], "proof-key");
    assert_eq!(exchange["data"]["WAREHOUSE_NAME"], "WH");
    assert!(requests[1].contains("warehouse=WH"));
    assert!(requests[1].contains("databaseName=DB"));
    assert!(requests[1].contains("schemaName=SCH"));
    assert!(requests[1].contains("roleName=ROLE"));
    assert!(requests[2].contains("Snowflake Token=\"session-1\""));
    assert!(requests[5].contains("Snowflake Token=\"session-2\""));
    let opened = OPENED_URL.get().unwrap().lock().unwrap().clone().unwrap();
    assert_eq!(opened, "https://idp.example.test/login?state=redacted");
}

#[tokio::test]
async fn external_browser_login_exchange_failure_is_generic_and_redacted() {
    let origin = sso_exchange_failure_server().await;
    let mut item = external_connector();
    item.origin = origin;
    item.browser_opener = test_browser_opener;
    item.sso_timeout = Duration::from_secs(1);
    let error = item.connect().await.unwrap_err().to_string();
    for marker in [
        "exchange-secret",
        "callback-secret",
        "session-secret",
        "proof-marker",
    ] {
        assert!(!error.contains(marker));
    }
}
