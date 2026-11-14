use super::*;

#[tokio::test]
async fn keypair_connect_performs_a_real_statement_request() {
    let (origin, seen) = server(vec![Reply::json(v2_done(&[1], 0))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    item.connect().await.unwrap();
    assert!(seen.lock().await[0].contains("SELECT 1"));
}

#[tokio::test]
async fn external_browser_is_disabled_by_default() {
    let item = connector(Auth::ExternalBrowser(ExternalBrowser {
        enabled: false,
        token: Arc::new(Mutex::new(None)),
    }));
    assert!(
        item.connect()
            .await
            .unwrap_err()
            .to_string()
            .contains("interactive mode")
    );
}
