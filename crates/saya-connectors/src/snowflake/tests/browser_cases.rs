use super::browser_support::*;
use super::*;

#[tokio::test]
async fn external_browser_rejects_bad_urls_provider_failures_and_opener_failures() {
    let _serial = browser_test_guard().await;
    for url in [
        "http://idp.example.test/login",
        "https://user:pass@idp.example.test/login",
        "not-a-url",
    ] {
        let (origin, _) = server(vec![auth_response(url)]).await;
        let mut item = external_connector();
        item.origin = origin;
        item.browser_opener = record_browser_opener;
        let error = item.connect().await.unwrap_err().to_string();
        assert!(!error.contains("idp.example"));
        assert!(!error.contains("proof-marker"));
    }
    let marker = "provider-body-marker";
    for response in [
        Reply::status("500 Internal Server Error", json!({"message":marker})),
        Reply {
            status: "200 OK",
            body: marker.as_bytes().to_vec(),
            headers: vec![],
            delay: Duration::ZERO,
        },
        Reply::json(json!({"success":false,"message":marker})),
    ] {
        let (origin, _) = server(vec![response]).await;
        let mut item = external_connector();
        item.origin = origin;
        let error = item.connect().await.unwrap_err().to_string();
        assert!(!error.contains(marker));
    }
    let (origin, _) = server(vec![auth_response(
        "https://idp.example.test/login?state=secret",
    )])
    .await;
    let mut item = external_connector();
    item.origin = origin;
    item.browser_opener = failing_browser_opener;
    let error = item.connect().await.unwrap_err().to_string();
    assert!(!error.to_string().contains("state=secret"));
}

#[tokio::test]
async fn external_browser_timeout_is_bounded_and_secret_free() {
    let _serial = browser_test_guard().await;
    let marker = "callback-secret-marker";
    let (origin, _) = server(vec![auth_response(
        "https://idp.example.test/login?state=secret",
    )])
    .await;
    let mut item = external_connector();
    item.origin = origin;
    item.browser_opener = record_browser_opener;
    item.sso_timeout = Duration::from_millis(30);
    let error = item.connect().await.unwrap_err().to_string();
    assert!(!error.contains(marker));
    assert!(!error.contains("state=secret"));
}
