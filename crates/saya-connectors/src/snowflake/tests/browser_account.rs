use super::browser_support::*;
use super::*;

/// Snowflake maps a SAML assertion by the base account name, while the host
/// carries the region and any privatelink suffix. Sending the full host as
/// `ACCOUNT_NAME` makes the IdP lookup fail on exactly the accounts that need
/// it, and a plain identifier hides that because there the base name and the
/// host are the same string.
#[tokio::test]
async fn account_name_is_the_base_account_not_the_regional_host() {
    let _serial = browser_test_guard().await;
    let (origin, seen) = sso_fixture_server().await;
    // Built from the regional identifier a privatelink deployment is reached
    // on, so the base account has to be derived rather than assumed.
    let mut item = SnowflakeConnector::new(
        "acct.ap-southeast-2.privatelink".into(),
        "user".into(),
        Auth::ExternalBrowser(ExternalBrowser {
            enabled: true,
            token: Arc::new(Mutex::new(None)),
        }),
        Context {
            warehouse: None,
            database: None,
            schema: None,
            role: None,
        },
        ConnectorOptions {
            query_timeout_seconds: 2,
            max_connections: 1,
            ..Default::default()
        },
    )
    .unwrap();
    item.origin = origin;
    item.browser_opener = record_browser_opener;
    item.sso_timeout = Duration::from_secs(2);

    let _ = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await;

    let requests = seen.lock().await;
    let authenticator = requests
        .iter()
        .find(|item| item.starts_with("POST /session/authenticator-request"))
        .expect("an authenticator request");
    let body: Value = serde_json::from_str(request_body(authenticator)).unwrap();
    assert_eq!(
        body["data"]["ACCOUNT_NAME"], "acct",
        "the IdP matches the bare account, not the host it was reached on"
    );
}
