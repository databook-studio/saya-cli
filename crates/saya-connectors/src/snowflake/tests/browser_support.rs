use super::*;

pub(super) fn test_browser_opener(_: &str) -> Result<(), ()> {
    Ok(())
}

pub(super) static OPENED_URL: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();

pub(super) fn record_browser_opener(url: &str) -> Result<(), ()> {
    *OPENED_URL
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap() = Some(url.into());
    Ok(())
}

pub(super) fn failing_browser_opener(_: &str) -> Result<(), ()> {
    Err(())
}

/// `browser_opener` is a bare `fn` pointer, so tests that want to inspect the
/// opened URL must capture it through the process-global `OPENED_URL`. Tests
/// that install `record_browser_opener` therefore share that global and must run
/// serially; this guard enforces mutual exclusion and resets the captured URL so
/// one test never observes another's value. An async-aware mutex is used because
/// the guard is held across the SSO flow's `.await` points.
static BROWSER_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub(super) async fn browser_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = BROWSER_TEST_LOCK.lock().await;
    *OPENED_URL
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    guard
}

pub(super) fn external_connector() -> SnowflakeConnector {
    connector(Auth::ExternalBrowser(ExternalBrowser {
        enabled: true,
        token: Arc::new(Mutex::new(None)),
    }))
}

pub(super) fn auth_response(url: &str) -> Reply {
    Reply::json(json!({"success":true,"data":{"ssoUrl":url,"proofKey":"proof-marker"}}))
}

pub(super) async fn sso_fixture_server() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let capture = seen.clone();
    tokio::spawn(async move {
        let mut auth_count = 0;
        let mut query_count = 0;
        for _ in 0..7 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            capture.lock().await.push(request.clone());
            let response = if request.starts_with("POST /session/authenticator-request") {
                auth_count += 1;
                let body: Value = serde_json::from_str(request_body(&request)).unwrap();
                let port = body["data"]["BROWSER_MODE_REDIRECT_PORT"].as_u64().unwrap() as u16;
                let token = format!("callback-{auth_count}%2Breserved%26value");
                tokio::spawn(async move {
                    sleep(Duration::from_millis(5)).await;
                    let mut callback = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                    let body = format!("token={token}");
                    let header = format!(
                        "POST /callback HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n",
                        body.len()
                    );
                    callback.write_all(header.as_bytes()).await.unwrap();
                    sleep(Duration::from_millis(5)).await;
                    callback.write_all(body.as_bytes()).await.unwrap();
                });
                json!({"success":true,"data":{"ssoUrl":"https://idp.example.test/login?state=redacted","proofKey":"proof-key"}})
            } else if request.starts_with("POST /session/v1/login-request") {
                json!({"success":true,"data":{"token":format!("session-{auth_count}")}})
            } else {
                query_count += 1;
                if query_count == 1 {
                    json!({"success":false,"data":{"code":"390104"}})
                } else {
                    json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[7]]}})
                }
            };
            let body = response.to_string();
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{address}"), seen)
}

pub(super) async fn sso_exchange_failure_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let request = read_request(&mut socket).await;
        let body: Value = serde_json::from_str(request_body(&request)).unwrap();
        let port = body["data"]["BROWSER_MODE_REDIRECT_PORT"].as_u64().unwrap() as u16;
        let response = auth_response("https://idp.example.test/login?state=exchange-secret");
        let body = String::from_utf8(response.body).unwrap();
        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        let mut callback = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        let callback_body = "token=callback-secret";
        callback.write_all(format!("POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{callback_body}", callback_body.len()).as_bytes()).await.unwrap();
        let (mut exchange, _) = listener.accept().await.unwrap();
        let _ = read_request(&mut exchange).await;
        let body = "{\"message\":\"session-secret\"}";
        exchange.write_all(format!("HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nContent-Type: application/json\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
    });
    format!("http://{address}")
}
