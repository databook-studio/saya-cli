use super::*;

// --- the local HTTP site: the only machine on the test network --------------

/// One scripted route. `location` makes it a redirect; otherwise the full
/// `body` is served (or a slice of it, to a ranged request), in `chunk`-size
/// writes. `abort_after` kills a *full* response mid-body: the bytes are
/// written, then the connection drops with the declared `Content-Length`
/// unserved — a killed download, not a short one.
#[derive(Clone)]
pub(crate) struct Route {
    pub(crate) body: Vec<u8>,
    pub(crate) location: Option<String>,
    pub(crate) chunk: usize,
    pub(crate) abort_after: Option<usize>,
}

pub(crate) fn route(body: &[u8], chunk: usize) -> Route {
    Route {
        body: body.to_vec(),
        location: None,
        chunk,
        abort_after: None,
    }
}

pub(crate) fn redirect(to: &str) -> Route {
    Route {
        body: Vec::new(),
        location: Some(to.to_owned()),
        chunk: usize::MAX,
        abort_after: None,
    }
}

/// The site: a loopback listener serving scripted routes, with logs of the
/// request paths and the ranges that were actually requested — the witness
/// that no connection was attempted where the downloader must refuse before
/// connecting.
pub(crate) struct LocalSite {
    pub(crate) addr: std::net::SocketAddr,
    pub(crate) routes: Arc<Mutex<Vec<(String, Route)>>>,
    pub(crate) served: Arc<Mutex<Vec<String>>>,
    pub(crate) ranges: Arc<Mutex<Vec<Option<u64>>>>,
}

impl LocalSite {
    pub(crate) async fn spawn(routes: Vec<(&'static str, Route)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("listener address");
        let routes = Arc::new(Mutex::new(
            routes
                .into_iter()
                .map(|(path, route)| (path.to_owned(), route))
                .collect(),
        ));
        let served = Arc::new(Mutex::new(Vec::new()));
        let ranges = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(serve(
            listener,
            routes.clone(),
            served.clone(),
            ranges.clone(),
        ));
        Self {
            addr,
            routes,
            served,
            ranges,
        }
    }

    pub(crate) fn served(&self) -> Vec<String> {
        self.served.lock().expect("log poisoned").clone()
    }

    pub(crate) fn ranges(&self) -> Vec<Option<u64>> {
        self.ranges.lock().expect("log poisoned").clone()
    }
}

/// The serve loop: accept connections forever; each one is handled alone.
async fn serve(
    listener: TcpListener,
    routes: Arc<Mutex<Vec<(String, Route)>>>,
    served: Arc<Mutex<Vec<String>>>,
    ranges: Arc<Mutex<Vec<Option<u64>>>>,
) {
    while let Ok((socket, _)) = listener.accept().await {
        tokio::spawn(handle_connection(
            socket,
            routes.clone(),
            served.clone(),
            ranges.clone(),
        ));
    }
}

/// Reads the request head, logs the request target and any range, and writes
/// the scripted response. Bodies are `Content-Length`-declared; an
/// `abort_after` writes fewer bytes than promised and drops the socket, so
/// the reader reports a broken transfer — never a short body as whole.
async fn handle_connection(
    mut socket: TcpStream,
    routes: Arc<Mutex<Vec<(String, Route)>>>,
    served: Arc<Mutex<Vec<String>>>,
    ranges: Arc<Mutex<Vec<Option<u64>>>>,
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
    let head = String::from_utf8_lossy(&head).into_owned();
    let path = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default()
        .to_owned();
    let range = head
        .lines()
        .find_map(|line| line.strip_prefix("Range: bytes="))
        .and_then(|value| value.split('-').next())
        .and_then(|offset| offset.parse::<u64>().ok());
    served.lock().expect("log poisoned").push(path.clone());
    ranges.lock().expect("log poisoned").push(range);
    let Some(route) = routes
        .lock()
        .expect("routes poisoned")
        .iter()
        .find(|(name, _)| *name == path)
        .map(|(_, route)| route.clone())
    else {
        let _ = socket
            .write_all(b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")
            .await;
        return;
    };
    if let Some(location) = &route.location {
        let _ = socket
            .write_all(
                format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nConnection: close\r\n\r\n")
                    .as_bytes(),
            )
            .await;
        return;
    }
    let body = &route.body;
    match range {
        // Ranged serving: the slice from the asked offset, 206 with a
        // Content-Range naming it, Content-Length of the slice.
        Some(start) if (start as usize) < body.len() => {
            let head = format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{}/{}\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len() - 1,
                body.len(),
                body.len(),
                body.len() - start as usize,
            );
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            for chunk in body[start as usize..].chunks(route.chunk) {
                if socket.write_all(chunk).await.is_err() {
                    return;
                }
            }
        }
        Some(_) => {
            let _ = socket
                .write_all(b"HTTP/1.1 416 Range Not Satisfiable\r\nConnection: close\r\n\r\n")
                .await;
        }
        // The full response: Content-Length declared, written in chunks; an
        // `abort_after` stops mid-body and drops the socket.
        None => {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len(),
            );
            if socket.write_all(head.as_bytes()).await.is_err() {
                return;
            }
            let limit = route.abort_after.unwrap_or(body.len());
            for chunk in body[..limit.min(body.len())].chunks(route.chunk) {
                if socket.write_all(chunk).await.is_err() {
                    return;
                }
            }
        }
    }
}
