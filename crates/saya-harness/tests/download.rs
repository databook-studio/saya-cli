//! The `http_download` battery, served entirely from `127.0.0.1` — no test
//! touches the real network; DNS is a fake table.
//!
//! The plan-required cases: the budget trip that pauses fail-safe and leaves
//! a resumable partial, the killed download that resumes to the whole
//! content's digest, the tampered partial refused on digest, the refused
//! redirect hop writing nothing, the resolved private address refused before
//! any connection (the listener is the witness), the per-file and per-run
//! byte bounds each tripping typed, and the contained destination with an
//! unchanged sentinel outside the workspace. Two loop-level tests prove the
//! declaration: the default denies `http_download` and no file appears.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;

use async_trait::async_trait;
use saya_agent::{
    AgentEvent, AgentEventSink, AgentLimits, AgentRequest, CancellationToken, ChatProvider,
    ChatRequest, ChatResponse, ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolError,
    ToolExecutor, run_agent_with_sink,
};
use saya_harness::fetch::{
    DownloadBudget, DownloadError, DownloadLimits, FetchBody, FetchDestination, FetchPolicy,
    FetchRefusal, FetchRequest, FetchTransport, FetchTransportError, WireResponse, http_download,
    http_download_definition,
};
use saya_harness::workspace::Workspace;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

// --- the local HTTP site: the only machine on the test network --------------

/// One scripted route. `location` makes it a redirect; otherwise the full
/// `body` is served (or a slice of it, to a ranged request), in `chunk`-size
/// writes. `abort_after` kills a *full* response mid-body: the bytes are
/// written, then the connection drops with the declared `Content-Length`
/// unserved — a killed download, not a short one.
#[derive(Clone)]
struct Route {
    body: Vec<u8>,
    location: Option<String>,
    chunk: usize,
    abort_after: Option<usize>,
}

fn route(body: &[u8], chunk: usize) -> Route {
    Route {
        body: body.to_vec(),
        location: None,
        chunk,
        abort_after: None,
    }
}

fn redirect(to: &str) -> Route {
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
struct LocalSite {
    addr: std::net::SocketAddr,
    routes: Arc<Mutex<Vec<(String, Route)>>>,
    served: Arc<Mutex<Vec<String>>>,
    ranges: Arc<Mutex<Vec<Option<u64>>>>,
}

impl LocalSite {
    async fn spawn(routes: Vec<(&'static str, Route)>) -> Self {
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

    fn served(&self) -> Vec<String> {
        self.served.lock().expect("log poisoned").clone()
    }

    fn ranges(&self) -> Vec<Option<u64>> {
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

// --- the hermetic transport: fake DNS, one loopback machine -----------------

/// The test network: DNS is a scripted table; every judged request is served
/// by the local listener regardless of which address the tool pinned.
struct TestNet {
    site: std::net::SocketAddr,
    dns: HashMap<String, Vec<IpAddr>>,
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

// --- the fixture ------------------------------------------------------------

const HOST: &str = "files.example.org";
const DEST: &str = "downloads/doc.bin";
const PART: &str = "downloads/doc.bin.saya-part";
const SIDECAR: &str = "downloads/doc.bin.saya-part.json";

fn destinations() -> Vec<FetchDestination> {
    vec![
        FetchDestination::new("https", HOST),
        FetchDestination::new("https", "private-resolve.example.org"),
        FetchDestination::new("https", "public-resolve.example.org"),
    ]
}

fn dns_table() -> HashMap<String, Vec<IpAddr>> {
    HashMap::from([
        (
            HOST.to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))],
        ),
        (
            "public-resolve.example.org".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 9))],
        ),
        (
            "private-resolve.example.org".to_owned(),
            vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))],
        ),
    ])
}

fn policy() -> FetchPolicy {
    FetchPolicy::new(destinations())
}

fn transport(site: &LocalSite) -> TestNet {
    TestNet {
        site: site.addr,
        dns: dns_table(),
    }
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace under the temp dir.
fn workspace(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-harness-download-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    let workspace = Workspace::open(&root).expect("workspace opens");
    (root, workspace)
}

/// The lowercase-hex SHA-256 of `content` — the digest the download must
/// match.
fn digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_json(root: &std::path::Path) -> serde_json::Value {
    let bytes = std::fs::read(root.join(SIDECAR)).expect("the sidecar exists");
    serde_json::from_slice(&bytes).expect("the sidecar parses")
}

// --- test 1: the budget trip pauses fail-safe, the partial is resumable -----

/// The run's download budget trips before the byte that would overrun it is
/// written: a typed pause, the destination still absent, and the partial on
/// disk — with its sidecar recording exactly the bytes it holds — resumes to
/// completion under a fresh wallet.
#[tokio::test]
async fn the_run_budget_trips_typed_and_the_partial_left_resumes() {
    let content = vec![b'a'; 40];
    let (root, workspace) = workspace("budget");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let budget = DownloadBudget::new(10);

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &budget,
    )
    .await
    .expect_err("the budget of 10 trips on the third 4-byte chunk");
    assert!(
        matches!(
            error,
            DownloadError::BudgetExhausted {
                limit: 10,
                received: 8
            }
        ),
        "typed pause, not overrun: {error:?}"
    );
    assert!(
        !root.join(DEST).exists(),
        "nothing landed at the destination"
    );
    let partial = std::fs::read(root.join(PART)).expect("the partial exists");
    assert_eq!(partial, content[..8], "exactly the bytes under the budget");
    let sidecar = sidecar_json(&root);
    assert_eq!(sidecar["len"], 8);
    assert_eq!(sidecar["sha256"], digest(&content[..8]));

    // The state a resume can use: a fresh wallet finishes the file.
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("the resume completes");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).expect("dest"), content);
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
}

// --- test 2: killed mid-download, resume completes to the whole digest ------

/// The first attempt is killed mid-body (the server writes part of the
/// promised bytes and dies — a transport failure, not a short body). The
/// resume asks for exactly the range past the partial, completes, and the
/// final digest is the whole content's.
#[tokio::test]
async fn a_killed_download_resumes_and_completes_to_the_whole_digest() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("kill");
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the killed transfer is a transport failure");
    assert!(matches!(error, DownloadError::Transport(_)), "{error:?}");
    let partial = std::fs::read(root.join(PART)).expect("the partial exists");
    assert_eq!(partial, content[..16]);

    // The server recovers; the resume continues from the recorded offset.
    site.routes.lock().expect("routes poisoned")[0]
        .1
        .abort_after = None;
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("the resume completes");
    assert_eq!(outcome.bytes, 40);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(std::fs::read(root.join(DEST)).expect("dest"), content);
    assert!(!root.join(SIDECAR).exists());
    assert_eq!(
        site.ranges(),
        vec![None, Some(16)],
        "the resume asked for exactly the range past the partial"
    );
}

// --- test 3: a tampered partial is refused on digest, typed ------------------

/// The partial's bytes were tampered with between the kill and the resume:
/// the sidecar digest no longer matches what is on disk, so the resume is a
/// typed refusal — the evidence stays in place, nothing is restarted over.
#[tokio::test]
async fn a_tampered_partial_is_refused_on_digest() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("tamper");
    let site = LocalSite::spawn(vec![(
        "/doc",
        Route {
            abort_after: Some(16),
            ..route(&content, 8)
        },
    )])
    .await;
    let net = transport(&site);

    http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the first attempt is killed");

    let mut tampered = content[..16].to_vec();
    tampered[3] ^= 0xff;
    workspace
        .write(PART, &tampered)
        .expect("tamper the partial");

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the tampered partial is refused on digest");
    assert!(
        matches!(error, DownloadError::ResumeMismatch { .. }),
        "typed, not a shrug: {error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the evidence stays"),
        tampered,
        "the partial is left in place"
    );
    assert!(!root.join(DEST).exists());
}

// --- test 4: a redirect to a refused destination is refused at that hop -----

/// The first hop's 302 points at the metadata endpoint: the hop is judged
/// and refused, the 302's body is never read, the refused target is never
/// connected to, and nothing is written.
#[tokio::test]
async fn a_redirect_to_a_refused_destination_is_refused_at_that_hop() {
    let (root, workspace) = workspace("redirect");
    let site = LocalSite::spawn(vec![(
        "/start",
        redirect("https://169.254.169.254/latest/meta-data/"),
    )])
    .await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/start"),
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("the redirect target is refused");
    assert!(
        matches!(
            error,
            DownloadError::Refused(FetchRefusal::DisallowedAddress { ref host })
                if host == "169.254.169.254"
        ),
        "typed refusal at the refused hop: {error:?}"
    );
    assert_eq!(site.served(), vec!["/start".to_owned()]);
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "nothing was written"
    );
}

// --- test 5: a resolved private address is refused before anything connects -

/// The policy judges the name; the downloader refuses what it resolves to:
/// a declared name resolving private is refused before any connection — the
/// listener is the witness that nothing was hit. The same shape with a
/// public resolution downloads.
#[tokio::test]
async fn a_resolved_private_address_is_refused_even_when_the_hostname_is_declared() {
    let content = b"public body".to_vec();
    let (_root, workspace) = workspace("private");
    let site = LocalSite::spawn(vec![("/data", route(content.as_slice(), 16))]).await;
    let net = transport(&site);

    let error = http_download(
        &policy(),
        &net,
        &workspace,
        "downloads/data.bin",
        "https://private-resolve.example.org/data",
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect_err("resolves private, so refused");
    assert!(
        matches!(
            error,
            DownloadError::Refused(FetchRefusal::DisallowedAddress { ref host })
                if host == "10.0.0.7"
        ),
        "the resolved address is named: {error:?}"
    );
    assert!(
        site.served().is_empty(),
        "refused before any connection: {:?}",
        site.served()
    );

    // The positive control: the same shape with a public resolution downloads.
    let outcome = http_download(
        &policy(),
        &net,
        &workspace,
        "downloads/data.bin",
        "https://public-resolve.example.org/data",
        DownloadLimits::default(),
        &DownloadBudget::default(),
    )
    .await
    .expect("public resolution downloads");
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(site.served(), vec!["/data".to_owned()]);
}

// --- test 6: the per-file and per-run byte bounds each trip as typed --------

/// The per-file bound trips as a typed error before the overrunning byte is
/// written; the partial below the bound is left in place.
#[tokio::test]
async fn the_file_bound_trips_as_a_typed_error() {
    let content = vec![b'x'; 40];
    let (root, workspace) = workspace("file-bound");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits {
            max_file_bytes: 8,
            ..DownloadLimits::default()
        },
        &DownloadBudget::default(),
    )
    .await
    .expect_err("over the file bound");
    assert!(
        matches!(error, DownloadError::FileTooLarge { limit: 8 }),
        "{error:?}"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the partial exists"),
        content[..8],
        "no overrun: exactly the bytes under the bound"
    );
}

/// The per-run budget trips as a typed error on its own, with a generous
/// file bound — the two bounds are independent.
#[tokio::test]
async fn the_run_bound_trips_as_a_typed_error() {
    let content = vec![b'b'; 40];
    let (_root, workspace) = workspace("run-bound");
    let site = LocalSite::spawn(vec![("/doc", route(&content, 4))]).await;
    let net = transport(&site);
    let error = http_download(
        &policy(),
        &net,
        &workspace,
        DEST,
        &url("/doc"),
        DownloadLimits::default(),
        &DownloadBudget::new(8),
    )
    .await
    .expect_err("over the run bound");
    assert!(
        matches!(
            error,
            DownloadError::BudgetExhausted {
                limit: 8,
                received: 8
            }
        ),
        "{error:?}"
    );
}

// --- test 7: the destination is contained; a sentinel outside is unchanged --

/// A `..` escape is refused by the same path discipline as
/// `workspace_write`; a sentinel outside the workspace is unchanged and no
/// request is ever made.
#[tokio::test]
async fn the_destination_is_contained_and_a_sentinel_outside_is_unchanged() {
    let (root, workspace) = workspace("contain");
    let sentinel = root.parent().unwrap().join("sentinel.txt");
    std::fs::write(&sentinel, b"keep me").expect("sentinel outside");
    let site = LocalSite::spawn(vec![("/doc", route(b"payload", 4))]).await;
    let net = transport(&site);

    for destination in ["../sentinel.txt", "out/../../escape.txt", "/etc/passwd"] {
        let error = http_download(
            &policy(),
            &net,
            &workspace,
            destination,
            &url("/doc"),
            DownloadLimits::default(),
            &DownloadBudget::default(),
        )
        .await
        .expect_err("the escape is refused");
        assert!(matches!(error, DownloadError::Workspace(_)), "{error:?}");
    }
    assert_eq!(
        std::fs::read(&sentinel).expect("sentinel"),
        b"keep me",
        "the sentinel outside the workspace is unchanged"
    );
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "nothing was written into the workspace"
    );
    assert!(site.served().is_empty(), "no request was ever made");
}

// --- the permit gate: the default denies http_download, no file appears -----

/// A provider that emits one tool call on its first `stream` invocation and
/// a plain answer afterwards, so a surviving turn completes.
struct OneCallProvider {
    call: ToolCall,
    turn: Mutex<u32>,
}

#[async_trait]
impl ChatProvider for OneCallProvider {
    fn name(&self) -> &str {
        "download-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(ProviderEvent::ToolCalls(vec![self.call.clone()])),
                Ok(ProviderEvent::Done),
            ]
        } else {
            vec![
                Ok(ProviderEvent::TextDelta("done".into())),
                Ok(ProviderEvent::Done),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

/// An executor that really would download: if the loop ever ran it, the
/// workspace would gain the part file, the sidecar, and the destination.
struct DownloadExecutor {
    policy: FetchPolicy,
    net: TestNet,
    workspace: Workspace,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl ToolExecutor for DownloadExecutor {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.calls.lock().unwrap().push(name.to_owned());
        let result = match http_download(
            &self.policy,
            &self.net,
            &self.workspace,
            arguments["destination"].as_str().unwrap_or_default(),
            arguments["url"].as_str().unwrap_or_default(),
            DownloadLimits::default(),
            &DownloadBudget::default(),
        )
        .await
        {
            Ok(outcome) => serde_json::json!({
                "destination": outcome.destination,
                "bytes": outcome.bytes,
                "sha256": outcome.sha256,
            }),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        };
        Ok(result)
    }
}

struct RecordingSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

#[async_trait]
impl AgentEventSink for RecordingSink {
    async fn emit(&self, event: AgentEvent) {
        self.events.lock().unwrap().push(event);
    }
}

/// Approval granted to everything — so in the gate-isolation test only the
/// workspace permit gate can refuse the tool.
struct AllowApproval;

#[async_trait]
impl saya_agent::ApprovalDecider for AllowApproval {
    async fn approve(&self, _: &saya_agent::ToolDefinition, _: &serde_json::Value) -> bool {
        true
    }
}

fn agent_request() -> AgentRequest {
    AgentRequest {
        prompt: "fetch the corpus".into(),
        profile_names: vec!["analytics".into()],
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// The default runner denies `http_download` (its declaration:
/// `external_side_effect` + `WriteWorkspace`), the tool never executes, and
/// no file appears in the workspace.
#[tokio::test]
async fn the_default_denies_http_download_and_no_file_appears() {
    let content = b"payload".to_vec();
    let (root, workspace) = workspace("deny");
    let site = LocalSite::spawn(vec![("/doc", route(content.as_slice(), 4))]).await;
    let net = transport(&site);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let executor = DownloadExecutor {
        policy: policy(),
        net,
        workspace,
        calls: calls.clone(),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let output = run_agent_with_sink(
        &OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: "http_download".into(),
                arguments: serde_json::json!({"url": url("/doc"), "destination": DEST}),
            },
            turn: Mutex::new(0),
        },
        &executor,
        agent_request(),
        vec![http_download_definition()],
        AgentLimits::default(),
        &saya_agent::AllowReadOnlyApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute when workspace writes are not permitted"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "http_download" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(!reason.is_empty(), "the denial names its gate: {reason}");
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "no file appears: the workspace is untouched"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}

/// The definition's `WriteWorkspace` declaration feeds the workspace permit
/// gate specifically: with approval granted (a variant that requires it, so
/// the side-effect gate does not bind), the default runner still refuses and
/// the denial names the workspace gate.
#[tokio::test]
async fn the_workspace_permit_gate_is_what_refuses_the_download_tool() {
    let content = b"payload".to_vec();
    let (root, workspace) = workspace("gate");
    let site = LocalSite::spawn(vec![("/doc", route(content.as_slice(), 4))]).await;
    let net = transport(&site);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let events = Arc::new(Mutex::new(Vec::new()));
    let executor = DownloadExecutor {
        policy: policy(),
        net,
        workspace,
        calls: calls.clone(),
    };
    let sink = RecordingSink {
        events: events.clone(),
    };
    let mut requires_approval = http_download_definition();
    requires_approval.effect.requires_approval = true;
    let output = run_agent_with_sink(
        &OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: "http_download".into(),
                arguments: serde_json::json!({"url": url("/doc"), "destination": DEST}),
            },
            turn: Mutex::new(0),
        },
        &executor,
        agent_request(),
        vec![requires_approval],
        AgentLimits::default(),
        &AllowApproval,
        &sink,
        CancellationToken::new(),
    )
    .await
    .expect("a denial is not a turn-ending error");
    assert!(
        calls.lock().unwrap().is_empty(),
        "the tool must not execute"
    );
    let reason = events
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolDenied { name, reason } if name == "http_download" => {
                Some(reason.clone())
            }
            _ => None,
        })
        .expect("a ToolDenied event must be emitted");
    assert!(
        reason.contains("workspace"),
        "the workspace permit gate is the refusing one: {reason}"
    );
    assert!(
        std::fs::read_dir(&root)
            .expect("workspace")
            .next()
            .is_none(),
        "no file appears"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
}
