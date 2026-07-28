use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use flate2::{Compression, write::GzEncoder};
use rsa::{
    RsaPrivateKey,
    pkcs8::{EncodePrivateKey, LineEnding},
};
use serde_json::{Value, json};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Mutex,
    time::sleep,
};

use super::{Auth, Context, ExternalBrowser, Keypair, SnowflakeConnector, Userpass};
use crate::{ConnectorOptions, DatabaseConnector};

#[derive(Clone)]
struct Reply {
    status: &'static str,
    body: Vec<u8>,
    headers: Vec<(&'static str, &'static str)>,
    delay: Duration,
}

impl Reply {
    fn json(value: Value) -> Self {
        Self {
            status: "200 OK",
            body: value.to_string().into_bytes(),
            headers: vec![],
            delay: Duration::ZERO,
        }
    }
    fn status(status: &'static str, value: Value) -> Self {
        Self {
            status,
            body: value.to_string().into_bytes(),
            headers: vec![],
            delay: Duration::ZERO,
        }
    }
    fn gzip(body: &str) -> Self {
        let mut writer = GzEncoder::new(Vec::new(), Compression::default());
        std::io::Write::write_all(&mut writer, body.as_bytes()).unwrap();
        Self {
            status: "200 OK",
            body: writer.finish().unwrap(),
            headers: vec![("content-encoding", "gzip")],
            delay: Duration::ZERO,
        }
    }
}

async fn server(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(vec![]));
    let capture = seen.clone();
    tokio::spawn(async move {
        for reply in replies {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            capture.lock().await.push(request);
            sleep(reply.delay).await;
            let mut headers = format!(
                "HTTP/1.1 {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n",
                reply.status,
                reply.body.len()
            );
            for (name, value) in reply.headers {
                headers.push_str(&format!("{name}: {value}\r\n"));
            }
            socket
                .write_all(format!("{headers}\r\n").as_bytes())
                .await
                .unwrap();
            socket.write_all(&reply.body).await.unwrap();
        }
    });
    (format!("http://{address}"), seen)
}

async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        let Some(end) = bytes.windows(4).position(|item| item == b"\r\n\r\n") else {
            continue;
        };
        let header = String::from_utf8_lossy(&bytes[..end + 4]);
        let length = header
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length: ")
                    .or_else(|| line.strip_prefix("Content-Length: "))
            })
            .and_then(|item| item.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn keypair() -> Keypair {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    Keypair {
        private_key: key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string(),
        passphrase: None,
    }
}

fn connector(auth: Auth) -> SnowflakeConnector {
    SnowflakeConnector::new(
        "acct".into(),
        "user".into(),
        auth,
        Context {
            warehouse: None,
            database: None,
            schema: None,
            role: None,
        },
        ConnectorOptions {
            query_timeout_seconds: 2,
            max_connections: 1,
        },
    )
    .unwrap()
}

fn handle() -> &'static str {
    "00000000-0000-4000-8000-000000000001"
}
fn rows(values: &[i64]) -> Vec<Value> {
    values.iter().map(|item| json!([item])).collect()
}
fn request_body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}
fn v2_done(values: &[i64], partitions: usize) -> Value {
    json!({"statementHandle":handle(), "data":rows(values), "resultSetMetaData":{"rowType":[{"name":"N"}], "partitionInfo": vec![json!({}); partitions]}})
}
fn userpass() -> Auth {
    Auth::Userpass(Userpass {
        password: "password-sentinel".into(),
        token: Arc::new(Mutex::new(None)),
    })
}

#[tokio::test]
async fn callback_accepts_empty_preconnect_and_fragmented_form_post() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        let _ = TcpStream::connect(address).await.unwrap();
        sleep(Duration::from_millis(10)).await;
        let mut stream = TcpStream::connect(address).await.unwrap();
        stream
            .write_all(b"POST /callback HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 21\r\n\r\ntoken=")
            .await
            .unwrap();
        sleep(Duration::from_millis(10)).await;
        stream.write_all(b"a%2Bb%26c%3D%3F").await.unwrap();
    });
    let token = super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "a+b&c=?");
}

#[tokio::test]
async fn callback_rejects_wrong_method_content_type_and_malformed_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        for request in [
            "PUT /callback HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "POST /callback HTTP/1.1\r\nHost: localhost\r\nContent-Type: text/plain\r\nContent-Length: 8\r\n\r\ntoken=no",
            "BROKEN\r\n\r\n",
            "GET /callback?token=final%2Btoken HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            sleep(Duration::from_millis(5)).await;
        }
    });
    let token = super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "final+token");
}

#[tokio::test]
async fn callback_rejects_duplicate_fields_oversized_input_and_times_out() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let sender = tokio::spawn(async move {
        for request in [
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 8\r\nContent-Length: 8\r\n\r\ntoken=no",
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 8\r\n\r\ntoken=no",
            "GET /callback?token=one&token=two HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 5\r\n\r\ntoken=extra",
            &format!(
                "GET /callback?token=large HTTP/1.1\r\nX-Large: {}\r\n\r\n",
                "x".repeat(17 * 1024)
            ),
            "POST /callback HTTP/1.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: 9000\r\n\r\n",
            "GET /callback?token=final HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ] {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            sleep(Duration::from_millis(5)).await;
        }
    });
    let token = super::sso_callback::capture_token(&listener, Duration::from_secs(1))
        .await
        .unwrap();
    sender.await.unwrap();
    assert_eq!(token, "final");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    assert!(
        super::sso_callback::capture_token(&listener, Duration::from_millis(20))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn v2_protocol_polls_partitions_and_preserves_wire_contract() {
    let pending = Reply::status(
        "202 Accepted",
        json!({"statementHandle":handle(), "statementStatusUrl":"/api/v2/statements/status"}),
    );
    let done = Reply::json(v2_done(&[1], 2));
    let part = Reply::gzip(
        &json!({"data":[[2]], "resultSetMetaData":{"rowType":[{"name":"N"}]}}).to_string(),
    );
    let (origin, seen) = server(vec![pending, done, part]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 2))
        .await
        .unwrap();
    assert_eq!(output.columns, vec!["N"]);
    assert_eq!(output.rows, rows(&[1, 2]));
    assert!(!output.truncated);
    let traffic = seen.lock().await;
    assert_eq!(traffic.len(), 3);
    assert!(traffic[0].starts_with("POST /api/v2/statements HTTP/1.1"));
    assert!(
        traffic[0].contains("user-agent: saya-cli/0.1")
            || traffic[0].contains("User-Agent: saya-cli/0.1")
    );
    assert!(
        traffic[0]
            .to_ascii_lowercase()
            .contains("accept: application/json")
    );
    assert!(
        traffic[0].contains("authorization: Bearer ")
            || traffic[0].contains("Authorization: Bearer ")
    );
    assert!(
        traffic[0]
            .to_ascii_lowercase()
            .contains("x-snowflake-authorization-token-type: keypair_jwt")
    );
    assert!(traffic[0].contains("LIMIT 3"));
    assert!(traffic[1].starts_with("GET /api/v2/statements/status HTTP/1.1"));
    assert!(traffic[2].starts_with(&format!(
        "GET /api/v2/statements/{}?partition=1 HTTP/1.1",
        handle()
    )));
    assert_eq!(request_body(&traffic[1]), "");
    assert_eq!(request_body(&traffic[2]), "");
}

#[tokio::test]
async fn v2_caps_rows_before_later_partitions() {
    let (origin, seen) = server(vec![Reply::json(v2_done(&[1, 2], 3))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    assert_eq!(output.rows, rows(&[1]));
    assert!(output.truncated);
    assert_eq!(seen.lock().await.len(), 1);
}

#[tokio::test]
async fn v2_rejects_cross_origin_and_prefix_trick_without_contacting_them() {
    for location in [
        "http://localhost:9/api/v2/statements/status",
        "http://127.0.0.1.evil/status",
    ] {
        let (origin, seen) = server(vec![Reply::status(
            "202 Accepted",
            json!({"statementHandle":handle(), "statementStatusUrl":location}),
        )])
        .await;
        let mut item = connector(Auth::Keypair(keypair()));
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "query failed: Snowflake query failed");
        assert_eq!(seen.lock().await.len(), 1);
    }
}

#[tokio::test]
async fn v2_errors_timeout_and_partition_failures_are_redacted_and_clear_active() {
    let marker = "LEAK-MARKER";
    for reply in [
        Reply::status("422 Unprocessable Content", json!({"message":marker})),
        Reply {
            status: "200 OK",
            body: marker.as_bytes().to_vec(),
            headers: vec![],
            delay: Duration::ZERO,
        },
    ] {
        let (origin, _) = server(vec![reply]).await;
        let mut item = connector(Auth::Keypair(keypair()));
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err();
        assert!(!error.to_string().contains(marker));
        assert!(item.active.lock().await.is_none());
    }
    let pending = Reply::status(
        "202 Accepted",
        json!({"statementHandle":handle(), "statementStatusUrl":"/status"}),
    );
    let (origin, _) = server(vec![
        pending,
        Reply::status(
            "202 Accepted",
            json!({"statementHandle":handle(), "statementStatusUrl":"/status"}),
        ),
    ])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    item.timeout = Duration::from_millis(70);
    assert!(
        item.execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .is_err()
    );
    assert!(item.active.lock().await.is_none());
    let (origin, _) = server(vec![
        Reply::json(v2_done(&[1], 2)),
        Reply::status("500 Internal Server Error", json!({"error":marker})),
    ])
    .await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 2))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(marker));
    assert!(item.active.lock().await.is_none());
}

#[tokio::test]
async fn cancellation_uses_uuid_endpoint_and_rejects_invalid_or_timed_out_handles() {
    let (origin, seen) = server(vec![Reply::json(json!({"ok":true}))]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    *item.active.lock().await = Some(handle().into());
    item.cancel().await.unwrap();
    let request = seen.lock().await[0].clone();
    assert!(request.starts_with(&format!("POST /api/v2/statements/{}/cancel", handle())));
    assert!(
        request
            .to_ascii_lowercase()
            .contains("x-snowflake-authorization-token-type: keypair_jwt")
    );
    *item.active.lock().await = Some("not-a-uuid".into());
    assert!(
        item.cancel()
            .await
            .unwrap_err()
            .to_string()
            .contains("no active")
    );
    let (origin, _) = server(vec![Reply {
        status: "200 OK",
        body: b"{}".to_vec(),
        headers: vec![],
        delay: Duration::from_millis(120),
    }])
    .await;
    let mut timed = connector(Auth::Keypair(keypair()));
    timed.origin = origin;
    timed.timeout = Duration::from_millis(30);
    *timed.active.lock().await = Some(handle().into());
    assert!(timed.cancel().await.is_err());
}

#[tokio::test]
async fn legacy_relogs_once_decodes_gzip_chunks_and_forwards_only_ssec_headers() {
    let login = || Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let expired = Reply::json(json!({"success":false,"data":{"code":"390104"}}));
    let query = Reply::json(
        json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1]],"chunkHeaders":{"x-amz-server-side-encryption-customer-key":"key","x-amz-server-side-encryption-customer-key-md5":"md5","x-not-forwarded":"nope"},"chunks":[{"url":"REPLACE_1"},{"url":"REPLACE_2"}]}}),
    );
    let (chunk_one, one_seen) = server(vec![Reply::gzip("[2],")]).await;
    let (chunk_two, two_seen) = server(vec![Reply::gzip("[3],")]).await;
    let query = Reply::json(query_json_with_urls(query.body, &chunk_one, &chunk_two));
    let (origin, seen) = server(vec![login(), expired, login(), query]).await;
    let mut item = connector(userpass());
    item.origin = origin;
    item.context = Context {
        warehouse: Some("WH".into()),
        database: Some("DB".into()),
        schema: Some("SCH".into()),
        role: Some("ROLE".into()),
    };
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap();
    assert_eq!(output.columns, vec!["N"]);
    assert_eq!(output.rows, rows(&[1, 2, 3]));
    let requests = seen.lock().await;
    assert_eq!(
        requests
            .iter()
            .filter(|item| item.contains("/session/v1/login-request"))
            .count(),
        2
    );
    assert!(
        requests
            .iter()
            .filter(|item| item.contains("/queries/v1/query-request"))
            .all(|item| item.contains("LIMIT 4"))
    );
    let first = one_seen.lock().await[0].to_ascii_lowercase();
    assert!(first.contains("x-amz-server-side-encryption-customer-key: key"));
    assert!(first.contains("x-amz-server-side-encryption-customer-key-md5: md5"));
    assert!(!first.contains("x-not-forwarded"));
    assert_eq!(two_seen.lock().await.len(), 1);
    let login = &requests[0];
    let login_json: Value = serde_json::from_str(request_body(login)).unwrap();
    assert_eq!(login_json["data"]["WAREHOUSE_NAME"], "WH");
    assert!(login_json["data"].get("WAREHOUSE").is_none());
    assert!(login.contains("warehouse=WH"));
    assert!(login.contains("databaseName=DB"));
    assert!(login.contains("schemaName=SCH"));
    assert!(login.contains("roleName=ROLE"));
}

#[tokio::test]
async fn legacy_401_relogs_once_early_stops_and_redacts_chunk_failures() {
    let login = || Reply::json(json!({"success":true,"data":{"token":"session"}}));
    let response = |url: &str| {
        Reply::json(
            json!({"success":true,"data":{"rowtype":[{"name":"N"}],"rowset":[[1],[2]],"chunks":[{"url":url}]}}),
        )
    };
    let (origin, seen) = server(vec![
        login(),
        Reply::status("401 Unauthorized", json!({"marker":"secret"})),
        login(),
        response("http://127.0.0.1:9/unused"),
    ])
    .await;
    let mut item = connector(userpass());
    item.origin = origin;
    let output = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap();
    assert!(output.truncated);
    assert_eq!(
        seen.lock()
            .await
            .iter()
            .filter(|request| request.contains("login-request"))
            .count(),
        2
    );
    let (bad, _) = server(vec![Reply::status(
        "500 Internal Server Error",
        json!({"marker":"chunk-secret"}),
    )])
    .await;
    let (origin, _) = server(vec![login(), response(&bad)]).await;
    let mut item = connector(userpass());
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("chunk-secret"));
    let (bad, _) = server(vec![Reply::gzip("not-json-secret")]).await;
    let (origin, _) = server(vec![login(), response(&bad)]).await;
    let mut item = connector(userpass());
    item.origin = origin;
    let error = item
        .execute(saya_types::QueryRequest::new("SELECT 1", 3))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains("not-json-secret"));
    assert!(item.active.lock().await.is_none());
}

fn query_json_with_urls(body: Vec<u8>, first: &str, second: &str) -> Value {
    let mut value: Value = serde_json::from_slice(&body).unwrap();
    value
        .pointer_mut("/data/chunks/0/url")
        .unwrap()
        .clone_from(&json!(first));
    value
        .pointer_mut("/data/chunks/1/url")
        .unwrap()
        .clone_from(&json!(second));
    value
}

#[tokio::test]
async fn schema_maps_catalog_rows_and_escapes_context_identifiers() {
    let response = Reply::json(v2_done(&[0], 0));
    let mut body: Value = serde_json::from_slice(&response.body).unwrap();
    body["data"] = json!([
        ["CAT", "SCH'EMA", "T", "C", "TEXT", "YES"],
        ["CAT", "SCH'EMA", "T", "D", "NUMBER", "NO"]
    ]);
    let (origin, seen) = server(vec![Reply::json(body)]).await;
    let mut item = connector(Auth::Keypair(keypair()));
    item.origin = origin;
    item.context.database = Some("CAT\"ALOG".into());
    item.context.schema = Some("SCH'EMA".into());
    let tree = item.schema().await.unwrap();
    assert_eq!(tree.databases[0].name, "CAT\"ALOG");
    assert_eq!(tree.databases[0].schemas[0].tables[0].columns.len(), 2);
    let request = seen.lock().await[0].clone();
    let posted: Value = serde_json::from_str(request_body(&request)).unwrap();
    let sql = posted["statement"].as_str().unwrap();
    assert!(sql.contains("\"CAT\"\"ALOG\".INFORMATION_SCHEMA.COLUMNS"));
    assert!(sql.contains("table_schema = 'SCH''EMA'"));
}

#[tokio::test]
async fn legacy_login_and_query_failures_are_generic_and_secret_free() {
    let marker = "server-payload-sentinel";
    for login in [
        Reply::status("500 Internal Server Error", json!({"message":marker})),
        Reply::json(json!({"success":false,"message":marker})),
    ] {
        let (origin, _) = server(vec![login]).await;
        let mut item = connector(userpass());
        item.origin = origin;
        let error = item.connect().await.unwrap_err().to_string();
        assert!(!error.contains(marker));
        assert!(!error.contains("password-sentinel"));
    }
    for query in [
        Reply::status("500 Internal Server Error", json!({"message":marker})),
        Reply::json(json!({"success":false,"data":{"message":marker}})),
    ] {
        let login = Reply::json(json!({"success":true,"data":{"token":"session"}}));
        let (origin, _) = server(vec![login, query]).await;
        let mut item = connector(userpass());
        item.origin = origin;
        let error = item
            .execute(saya_types::QueryRequest::new("SELECT 1", 1))
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains(marker));
        assert!(!error.contains("password-sentinel"));
    }
}

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

fn test_browser_opener(_: &str) -> Result<(), ()> {
    Ok(())
}

static OPENED_URL: OnceLock<std::sync::Mutex<Option<String>>> = OnceLock::new();

fn record_browser_opener(url: &str) -> Result<(), ()> {
    *OPENED_URL
        .get_or_init(|| std::sync::Mutex::new(None))
        .lock()
        .unwrap() = Some(url.into());
    Ok(())
}

fn failing_browser_opener(_: &str) -> Result<(), ()> {
    Err(())
}

fn external_connector() -> SnowflakeConnector {
    connector(Auth::ExternalBrowser(ExternalBrowser {
        enabled: true,
        token: Arc::new(Mutex::new(None)),
    }))
}

fn auth_response(url: &str) -> Reply {
    Reply::json(json!({"success":true,"data":{"ssoUrl":url,"proofKey":"proof-marker"}}))
}

#[tokio::test]
async fn external_browser_rejects_bad_urls_provider_failures_and_opener_failures() {
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

async fn sso_fixture_server() -> (String, Arc<Mutex<Vec<String>>>) {
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

async fn sso_exchange_failure_server() -> String {
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

#[tokio::test]
async fn external_browser_exchanges_fragmented_callback_caches_and_reauthenticates() {
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

#[test]
fn malformed_chunks_and_secrets_do_not_surface_values() {
    let error = super::result::chunk_rows("secret").unwrap_err();
    assert!(!error.to_string().contains("secret"));
}
