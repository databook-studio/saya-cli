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

/// A mock whose reply queue can be filled after binding, so a query reply can
/// embed same-origin chunk URLs that reference the server's own address.
async fn dynamic_server() -> (String, Arc<Mutex<Vec<Reply>>>, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let capture = seen.clone();
    let queue: Arc<Mutex<Vec<Reply>>> = Arc::new(Mutex::new(Vec::new()));
    let pending = queue.clone();
    tokio::spawn(async move {
        loop {
            // Wait for the test to enqueue work; an empty queue at bind time
            // is normal, not shutdown.
            let reply = loop {
                let mut guard = pending.lock().await;
                if !guard.is_empty() {
                    break guard.remove(0);
                }
                drop(guard);
                sleep(Duration::from_millis(5)).await;
            };
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
    (format!("http://{address}"), queue, seen)
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
            ..Default::default()
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

mod browser_account;
mod browser_cases;
mod browser_exchange;
mod browser_support;
mod callback;
mod callback_cors;
mod connector_auth;
mod legacy_chunks;
mod legacy_errors;
mod malformed_chunks;
mod parameters;
mod v2;
