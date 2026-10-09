use super::*;

// --- the local HTTP site: the only machine on the test network --------------

/// One canned response: a full status line, an optional redirect target, and
/// the body written in chunks (with an inter-chunk delay) before the socket
/// closes — close-delimited, so a reader sees EOF at the end.
#[derive(Clone)]
pub(super) struct CannedResponse {
    pub(super) status_line: &'static str,
    pub(super) location: Option<String>,
    pub(super) chunks: Vec<Vec<u8>>,
    pub(super) chunk_delay: Duration,
}

pub(super) fn ok(chunks: Vec<&[u8]>) -> CannedResponse {
    CannedResponse {
        status_line: "HTTP/1.1 200 OK",
        location: None,
        chunks: chunks.into_iter().map(<[u8]>::to_vec).collect(),
        chunk_delay: Duration::ZERO,
    }
}

pub(super) fn redirect(to: &str) -> CannedResponse {
    CannedResponse {
        status_line: "HTTP/1.1 302 Found",
        location: Some(to.to_owned()),
        chunks: Vec::new(),
        chunk_delay: Duration::ZERO,
    }
}

pub(super) fn slow_ok(chunks: Vec<&[u8]>, delay: Duration) -> CannedResponse {
    CannedResponse {
        status_line: "HTTP/1.1 200 OK",
        location: None,
        chunks: chunks.into_iter().map(<[u8]>::to_vec).collect(),
        chunk_delay: delay,
    }
}

/// The site: a loopback listener serving scripted routes, with an access log
/// of the paths that were actually requested — the witness that no connection
/// was attempted where the tool must refuse before connecting.
pub(super) struct LocalSite {
    pub(super) addr: std::net::SocketAddr,
    log: Arc<std::sync::Mutex<Vec<String>>>,
}

impl LocalSite {
    pub(super) async fn spawn(routes: Vec<(&'static str, CannedResponse)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("listener address");
        let routes: Arc<std::sync::Mutex<Vec<(String, CannedResponse)>>> =
            Arc::new(std::sync::Mutex::new(
                routes
                    .into_iter()
                    .map(|(path, response)| (path.to_owned(), response))
                    .collect(),
            ));
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        tokio::spawn(serve(listener, routes, log.clone()));
        Self { addr, log }
    }

    pub(super) fn served(&self) -> Vec<String> {
        self.log.lock().expect("log poisoned").clone()
    }
}

async fn serve(
    listener: TcpListener,
    routes: Arc<std::sync::Mutex<Vec<(String, CannedResponse)>>>,
    log: Arc<std::sync::Mutex<Vec<String>>>,
) {
    while let Ok((socket, _)) = listener.accept().await {
        tokio::spawn(handle_connection(socket, routes.clone(), log.clone()));
    }
}

/// Reads the request head off the socket, logs the request target, and writes
/// the canned response — headers first, then body chunks with the scripted
/// delay between them, then EOF.
async fn handle_connection(
    mut socket: TcpStream,
    routes: Arc<std::sync::Mutex<Vec<(String, CannedResponse)>>>,
    log: Arc<std::sync::Mutex<Vec<String>>>,
) {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match socket.read(&mut byte).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                head.push(byte[0]);
                if head.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let path = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    log.lock().expect("log poisoned").push(path.clone());
    let response = routes
        .lock()
        .expect("routes poisoned")
        .iter()
        .find(|(route, _)| *route == path)
        .map(|(_, response)| response.clone());
    if let Some(response) = response {
        let mut head = format!("{}\r\n", response.status_line);
        if let Some(location) = &response.location {
            head.push_str(&format!("Location: {location}\r\n"));
        }
        head.push_str("Content-Type: text/plain; charset=utf-8\r\n");
        head.push_str("Connection: close\r\n\r\n");
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for (index, chunk) in response.chunks.iter().enumerate() {
            if index > 0 && !response.chunk_delay.is_zero() {
                tokio::time::sleep(response.chunk_delay).await;
            }
            if socket.write_all(chunk).await.is_err() {
                return;
            }
        }
    } else {
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
            .await;
    }
}
