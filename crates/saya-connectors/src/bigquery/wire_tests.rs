//! No-network wire tests. A localhost TCP socket stands in for Google's API:
//! the token exchange, the dry-run, and the query each get a scripted reply,
//! and the raw request bytes are captured so the request body and headers can
//! be asserted directly. No real credentials or network are used.

use rsa::RsaPrivateKey;
use rsa::pkcs8::{EncodePrivateKey, LineEnding};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::client::BigQueryConnector;
use crate::ConnectorOptions;
use crate::DatabaseConnector;

async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
        let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") else {
            continue;
        };
        let header = String::from_utf8_lossy(&bytes[..end + 4]);
        let length = header
            .lines()
            .find_map(|line| {
                line.strip_prefix("content-length: ")
                    .or_else(|| line.strip_prefix("Content-Length: "))
            })
            .and_then(|v| v.trim().parse::<usize>().ok())
            .unwrap_or(0);
        if bytes.len() >= end + 4 + length {
            break;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// A scripted server that replies to the token exchange, the dry-run, and the
/// query in order, capturing every raw request for inspection.
async fn server(
    dry_run: Value,
    query: Value,
) -> (String, std::sync::Arc<tokio::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let capture = seen.clone();
    let token_body =
        r#"{"access_token":"test-access-token","token_type":"Bearer","expires_in":3600}"#;
    tokio::spawn(async move {
        let mut dry_run = Some(dry_run);
        let mut query = Some(query);
        for _ in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            let body = if request.starts_with("POST /token") {
                token_body.to_string()
            } else if request.contains("/jobs") {
                dry_run.take().unwrap().to_string()
            } else {
                query.take().unwrap().to_string()
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            capture.lock().await.push(request);
            socket.write_all(headers.as_bytes()).await.unwrap();
            socket.write_all(body.as_bytes()).await.unwrap();
        }
    });
    (format!("http://{address}"), seen)
}

fn connector(origin: &str, token_uri: &str, cap: Option<u64>) -> BigQueryConnector {
    let key = RsaPrivateKey::new(&mut rsa::rand_core::OsRng, 2048).unwrap();
    let private_key = key.to_pkcs8_pem(LineEnding::LF).unwrap().to_string();
    let json = format!(
        r#"{{"client_email":"reader@proj.iam.gserviceaccount.com","private_key":{private_key:?},"token_uri":{token_uri:?}}}"#
    );
    let mut connector = BigQueryConnector::new(
        "my-project".into(),
        Some("analytics".into()),
        None,
        cap,
        json,
        ConnectorOptions {
            query_timeout_seconds: 2,
            ..Default::default()
        },
    )
    .unwrap();
    connector.api_origin = origin.into();
    connector
}

fn request_body(request: &str) -> &str {
    request.split_once("\r\n\r\n").map_or("", |(_, body)| body)
}

#[tokio::test]
async fn query_request_carries_byte_cap_and_bearer_token_over_the_wire() {
    let dry_run = serde_json::json!({"statistics":{"query":{"totalBytesProcessed":"0"}},"status":{"state":"DONE"}});
    let query = serde_json::json!({
        "schema": {"fields": [{"name": "id", "type": "INTEGER"}]},
        "rows": [{"f": [{"v": "1"}]}]
    });
    let (origin, seen) = server(dry_run, query).await;
    let connector = connector(&origin, &format!("{origin}/token"), Some(1024));
    let result = connector
        .execute(saya_types::QueryRequest::new("SELECT id FROM `p.d.t`", 10))
        .await
        .unwrap();
    assert_eq!(result.columns, vec!["id"]);
    assert_eq!(result.rows, vec![serde_json::json!(["1"])]);
    let requests = seen.lock().await;
    // token exchange, dry-run, query — in that order.
    assert_eq!(requests.len(), 3);
    let query_request = &requests[2];
    assert!(query_request.starts_with("POST /projects/my-project/queries"));
    let body: Value = serde_json::from_str(request_body(query_request)).unwrap();
    assert_eq!(body["maximumBytesBilled"], "1024");
    assert_eq!(body["maxResults"], 11);
    let dry_run_request = &requests[1];
    let dry_body: Value = serde_json::from_str(request_body(dry_run_request)).unwrap();
    assert_eq!(dry_body["configuration"]["dryRun"], true);
    assert_eq!(
        dry_body["configuration"]["query"]["maximumBytesBilled"],
        "1024"
    );
    // The token rides the Authorization header on every API call, never the
    // body or the URL.
    assert!(dry_run_request.contains("authorization: Bearer test-access-token"));
    assert!(query_request.contains("authorization: Bearer test-access-token"));
}

#[tokio::test]
async fn over_budget_query_is_refused_before_it_runs() {
    let dry_run = serde_json::json!({"statistics":{"query":{"totalBytesProcessed":"9999"}},"status":{"state":"DONE"}});
    // The query reply is never consumed; the server still expects it but the
    // connector must stop after the dry-run.
    let query = serde_json::json!({"rows": []});
    let (origin, _) = server(dry_run, query).await;
    let connector = connector(&origin, &format!("{origin}/token"), Some(100));
    let error = connector
        .execute(saya_types::QueryRequest::new(
            "SELECT * FROM `p.d.huge`",
            10,
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("byte budget"));
}

#[tokio::test]
async fn error_messages_never_carry_the_token_or_server_body() {
    let marker = "server-payload-marker";
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let origin = format!("http://{address}");
    let token_uri = format!("{origin}/token");
    tokio::spawn(async move {
        // token exchange, then dry-run, then a failing query — in order.
        for (index, body) in [
            r#"{"access_token":"test-access-token","expires_in":3600}"#.to_string(),
            r#"{"statistics":{"query":{"totalBytesProcessed":"0"}}}"#.to_string(),
            format!(r#"{{"error":{{"message":"{marker}"}}}}"#),
        ]
        .into_iter()
        .enumerate()
        {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut socket).await;
            let status = if index == 2 {
                "500 Internal Server Error"
            } else {
                "200 OK"
            };
            let h = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(h.as_bytes()).await.unwrap();
        }
    });
    let connector = connector(&origin, &token_uri, Some(1024));
    let error = connector
        .execute(saya_types::QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err();
    assert!(!error.to_string().contains(marker));
    assert!(!error.to_string().contains("test-access-token"));
    assert!(!error.to_string().contains("Bearer"));
}
