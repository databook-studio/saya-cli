use super::*;

// --- the hermetic transport: fake DNS, one loopback machine -----------------

/// The test network: DNS is a scripted table; every judged request is served
/// by the local listener regardless of which address the tool pinned.
pub(crate) struct TestNet {
    pub(super) site: std::net::SocketAddr,
    pub(super) dns: HashMap<String, Vec<IpAddr>>,
}

#[async_trait]
impl FetchTransport for TestNet {
    async fn resolve(&self, host: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        self.dns
            .get(host)
            .cloned()
            .ok_or(FetchTransportError::Resolve {
                host: host.to_owned(),
            })
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        let url = url::Url::parse(request.url.as_str()).expect("policy-judged URL parses");
        let target = match url.query() {
            Some(query) => format!("{}?{}", url.path(), query),
            None => url.path().to_owned(),
        };
        let refused = |detail: String| FetchTransportError::Request {
            url: request.url.as_str().to_owned(),
            detail,
        };
        let mut socket = TcpStream::connect(self.site)
            .await
            .map_err(|error| refused(error.to_string()))?;
        let mut head = format!(
            "GET {target} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n",
            request.url.host().unwrap_or_default()
        );
        if let Some(start) = request.range_start {
            head.push_str(&format!("Range: bytes={start}-\r\n"));
        }
        head.push_str("\r\n");
        socket
            .write_all(head.as_bytes())
            .await
            .map_err(|error| refused(error.to_string()))?;
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
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .ok_or_else(|| refused(format!("unparseable response head: {head}")))?;
        let location = header_value(&head, "location");
        let content_range = header_value(&head, "content-range");
        let content_length =
            header_value(&head, "content-length").and_then(|value| value.parse().ok());
        Ok(WireResponse {
            status,
            location,
            content_range,
            body: Box::new(SocketBody {
                socket,
                remaining: content_length,
            }),
        })
    }
}

/// One `name: value` response-head line, matched case-insensitively.
fn header_value(head: &str, name: &str) -> Option<String> {
    head.lines()
        .take_while(|line| !line.is_empty())
        .find_map(|line| {
            let (field, value) = line.split_once(':')?;
            field
                .eq_ignore_ascii_case(name)
                .then(|| value.trim().to_owned())
        })
}

/// The length-delimited body of one response, chunk by chunk. A premature
/// EOF — the killed download — is a transport error, never a short body
/// passed off as whole.
struct SocketBody {
    socket: TcpStream,
    remaining: Option<u64>,
}

/// The pull cap: each `next_chunk` returns at most this many bytes, read
/// until the cap or EOF — so a test's chunked writes arrive as predictable
/// chunks instead of whatever the socket coalesced.
const PULL_CAP: usize = 8;

#[async_trait]
impl FetchBody for SocketBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        let error = |detail: String| FetchTransportError::Request {
            url: String::new(),
            detail,
        };
        let want = self
            .remaining
            .map_or(PULL_CAP, |remaining| (remaining as usize).min(PULL_CAP));
        let mut chunk = Vec::new();
        while chunk.len() < want {
            let mut buffer = vec![0u8; want - chunk.len()];
            let read = self
                .socket
                .read(&mut buffer)
                .await
                .map_err(|io_error| error(io_error.to_string()))?;
            if read == 0 {
                break;
            }
            chunk.extend_from_slice(&buffer[..read]);
            if let Some(remaining) = self.remaining.as_mut() {
                *remaining = remaining.saturating_sub(read as u64);
            }
        }
        if chunk.is_empty() {
            return match self.remaining.take() {
                Some(remaining) if remaining > 0 => Err(error(format!(
                    "connection closed {remaining} bytes before the promised end"
                ))),
                _ => Ok(None),
            };
        }
        Ok(Some(chunk))
    }
}
