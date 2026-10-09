use super::*;

// --- the hermetic transport: fake DNS, one loopback machine -----------------

/// The test network: DNS is a scripted table, and the only machine that can
/// serve is the local listener — every judged request is served by it,
/// regardless of which address the tool pinned. `body_pulls` counts how often
/// any response body was actually read, so tests can assert a body was never
/// read at all.
pub(super) struct TestNet {
    pub(super) site: std::net::SocketAddr,
    pub(super) dns: HashMap<String, Vec<IpAddr>>,
    pub(super) body_pulls: Arc<AtomicUsize>,
}

#[async_trait]
impl saya_harness::fetch::FetchTransport for TestNet {
    async fn resolve(
        &self,
        host: &str,
    ) -> Result<Vec<IpAddr>, saya_harness::fetch::FetchTransportError> {
        self.dns
            .get(host)
            .cloned()
            .ok_or(saya_harness::fetch::FetchTransportError::Resolve {
                host: host.to_owned(),
            })
    }

    async fn get(
        &self,
        request: saya_harness::fetch::FetchRequest,
    ) -> Result<saya_harness::fetch::WireResponse, saya_harness::fetch::FetchTransportError> {
        let url = url::Url::parse(request.url.as_str()).expect("policy-judged URL parses");
        let target = match url.query() {
            Some(query) => format!("{}?{}", url.path(), query),
            None => url.path().to_owned(),
        };
        let refused = |detail: String| saya_harness::fetch::FetchTransportError::Request {
            url: request.url.as_str().to_owned(),
            detail,
        };
        let mut socket = TcpStream::connect(self.site).await.map_err(|error| {
            saya_harness::fetch::FetchTransportError::Request {
                url: request.url.as_str().to_owned(),
                detail: error.to_string(),
            }
        })?;
        let head = format!(
            "GET {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            request.url.host().unwrap_or_default()
        );
        socket
            .write_all(head.as_bytes())
            .await
            .map_err(|e| refused(e.to_string()))?;
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            match socket.read(&mut byte).await {
                Ok(0) | Err(_) => {
                    return Err(refused("connection closed before response headers".into()));
                }
                Ok(_) => {
                    head.push(byte[0]);
                    if head.ends_with(b"\r\n\r\n") {
                        break;
                    }
                }
            }
        }
        let head = String::from_utf8_lossy(&head).into_owned();
        let mut lines = head.lines();
        let status_line = lines.next().unwrap_or_default();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| refused(format!("unparseable status line: {status_line}")))?;
        let location = lines.take_while(|line| !line.is_empty()).find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().to_owned())
        });
        let pulls = self.body_pulls.clone();
        Ok(saya_harness::fetch::WireResponse {
            status,
            location,
            content_range: None,
            body: Box::new(SocketBody { socket, pulls }),
        })
    }
}

/// The close-delimited body of one response, chunk by chunk.
struct SocketBody {
    socket: TcpStream,
    pulls: Arc<AtomicUsize>,
}

#[async_trait]
impl saya_harness::fetch::FetchBody for SocketBody {
    async fn next_chunk(
        &mut self,
    ) -> Result<Option<Vec<u8>>, saya_harness::fetch::FetchTransportError> {
        self.pulls.fetch_add(1, Ordering::SeqCst);
        let mut buffer = vec![0u8; 64 * 1024];
        let read = self.socket.read(&mut buffer).await.map_err(|error| {
            saya_harness::fetch::FetchTransportError::Request {
                url: String::new(),
                detail: error.to_string(),
            }
        })?;
        if read == 0 {
            Ok(None)
        } else {
            buffer.truncate(read);
            Ok(Some(buffer))
        }
    }
}
