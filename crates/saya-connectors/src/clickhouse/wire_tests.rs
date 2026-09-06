//! End-to-end proof that the connector now reads the failure body and forwards
//! an allow-listed object name, while an excluded conversion fault stays
//! redacted. A tiny mock HTTP server stands in for ClickHouse; no live database
//! is needed.

use saya_types::QueryRequest;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

use super::ClickHouseConnector;
use crate::ConnectorOptions;

/// Binds a one-shot mock that replies to a single query with `status`, an
/// optional `X-ClickHouse-Exception-Code` header, and `body`. Returns the port
/// the connector should target.
async fn mock(status: &'static str, code_header: Option<u16>, body: &str) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = body.to_owned();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        read_request(&mut socket).await;
        let mut headers = format!(
            "HTTP/1.1 {status}\r\ncontent-type: text/plain\r\ncontent-length: \
             {}\r\nconnection: close\r\n",
            body.len()
        );
        if let Some(code) = code_header {
            headers.push_str(&format!("x-clickhouse-exception-code: {code}\r\n"));
        }
        socket
            .write_all(format!("{headers}\r\n").as_bytes())
            .await
            .unwrap();
        socket.write_all(body.as_bytes()).await.unwrap();
    });
    port
}

/// Reads the full request (headers plus a content-length body) so reqwest does
/// not error on a response sent before the body was consumed.
async fn read_request(socket: &mut tokio::net::TcpStream) {
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
}

fn connector(port: u16) -> ClickHouseConnector {
    ClickHouseConnector::new(
        "127.0.0.1".into(),
        Some(port),
        None,
        None,
        None,
        None,
        ConnectorOptions {
            query_timeout_seconds: 2,
            ..Default::default()
        },
    )
    .unwrap()
}

#[tokio::test]
async fn a_missing_table_names_the_object_end_to_end() {
    let port = mock(
        "404 Not Found",
        Some(60),
        "Code: 60. DB::Exception: Table default.orders doesn't exist. (UNKNOWN_TABLE)",
    )
    .await;
    let connector = connector(port);
    let error = super::execute::query(&connector, QueryRequest::new("SELECT * FROM orders", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("orders"), "object name dropped: {error}");
    assert!(error.contains("doesn't exist"), "opaque: {error}");
}

#[tokio::test]
async fn a_conversion_fault_does_not_leak_a_row_value_end_to_end() {
    let port = mock(
        "400 Bad Request",
        Some(6),
        "Code: 6. DB::Exception: Cannot parse string '4111-1111-1111-1111' as UInt64. \
         (CANNOT_PARSE_QUOTED_STRING)",
    )
    .await;
    let connector = connector(port);
    let error = super::execute::query(&connector, QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err()
        .to_string();
    assert!(!error.contains("4111"), "row value leaked: {error}");
    assert_eq!(error, "query failed: ClickHouse query failed");
}

#[tokio::test]
async fn a_server_fault_is_not_read_as_a_sql_fault_end_to_end() {
    let port = mock(
        "500 Internal Server Error",
        Some(60),
        "Code: 60. DB::Exception: Table default.orders doesn't exist. (UNKNOWN_TABLE)",
    )
    .await;
    let connector = connector(port);
    let error = super::execute::query(&connector, QueryRequest::new("SELECT 1", 1))
        .await
        .unwrap_err();
    assert!(
        matches!(error, saya_types::ConnectionError::ConnectionFailed(_)),
        "5xx read as a SQL fault: {error}"
    );
    assert!(
        !error.to_string().contains("orders"),
        "server fault diagnosed: {error}"
    );
}
