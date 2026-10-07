//! The cancellation battery for `http_download`: the turn's token rides the
//! download from admission through every wait to the point right before
//! publication, and a cancelled turn never publishes an incomplete file.
//!
//! Served entirely from in-memory fakes — no test touches the real network.
//! The cases: a pre-cancelled download writes nothing; a cancelled header
//! wait writes nothing; a cancelled mid-body leaves an exact, resumable
//! partial (the sidecar records exactly the served bytes) and a retry
//! completes to the whole-file digest; a cancellation after the last byte
//! but before publication publishes nothing; the tool adapter reports the
//! cancellation typed; and the old uncancelled wrapper still completes.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};
use saya_harness::fetch::{
    DownloadBudget, DownloadCall, DownloadError, DownloadLimits, DownloadOutcome, FetchBody,
    FetchDestination, FetchLimits, FetchPolicy, FetchRequest, FetchTools, FetchTransport,
    FetchTransportError, WireResponse, http_download, http_download_cancellable,
};
use saya_harness::workspace::Workspace;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

/// Every await is under this guard, so a regression hangs into a failure
/// instead of stalling the suite.
const ABORT: Duration = Duration::from_secs(10);

// --- the fixture ------------------------------------------------------------

const HOST: &str = "files.example.org";
const DEST: &str = "downloads/doc.bin";
const OK_DEST: &str = "downloads/ok.bin";
const PART: &str = "downloads/doc.bin.saya-part";
const SIDECAR: &str = "downloads/doc.bin.saya-part.json";

/// The public TEST-NET address every fake resolves the declared host to —
/// allowed by the address refusal, like the real battery's table.
const RESOLVED: IpAddr = IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7));

fn policy() -> FetchPolicy {
    FetchPolicy::new(vec![FetchDestination::new("https", HOST)])
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace under the temp dir.
fn workspace(label: &str) -> (PathBuf, Workspace) {
    let root = std::env::temp_dir().join(format!(
        "saya-harness-download-cancel-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    let workspace = Workspace::open(&root).expect("workspace opens");
    (root, workspace)
}

/// The lowercase-hex SHA-256 of `content`.
fn digest(content: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(content)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sidecar_json(root: &Path) -> serde_json::Value {
    let bytes = std::fs::read(root.join(SIDECAR)).expect("the sidecar exists");
    serde_json::from_slice(&bytes).expect("the sidecar parses")
}

/// One download under the turn's token, spawned so the test can cancel while
/// it runs. The wallet moves in by clone, so the caller's clone sees the
/// claims.
fn spawn_download(
    net: impl FetchTransport + 'static,
    workspace: Workspace,
    budget: DownloadBudget,
    token: CancellationToken,
) -> JoinHandle<Result<DownloadOutcome, DownloadError>> {
    tokio::spawn(async move {
        let policy = policy();
        http_download_cancellable(DownloadCall {
            policy: &policy,
            transport: &net,
            workspace: &workspace,
            destination: DEST,
            expected_destination_sha256: None,
            url: &url("/doc"),
            limits: DownloadLimits::default(),
            budget: &budget,
            cancellation: &token,
        })
        .await
    })
}

// --- the fakes: one machine, canned bodies ----------------------------------

/// Logs every GET's URL and returns a canned body.
fn logged(url: &str, calls: &Arc<Mutex<Vec<String>>>) {
    calls.lock().expect("calls").push(url.to_owned());
}

/// The request/headers wait never resolves; the test cancels while the
/// download waits inside `get`.
struct HeldGetNet {
    entered: Arc<Notify>,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for HeldGetNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        self.entered.notify_one();
        std::future::pending().await
    }
}

/// Serves the first `stall_after` chunks of the content, then holds forever —
/// signalling when the hold begins, so the cancel lands mid-body, after the
/// served chunks were fully written and claimed.
struct StalledNet {
    content: Arc<Vec<u8>>,
    chunk: usize,
    stall_after: usize,
    held: Arc<Notify>,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for StalledNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        let chunks: VecDeque<Vec<u8>> = self
            .content
            .chunks(self.chunk)
            .take(self.stall_after)
            .map(<[u8]>::to_vec)
            .collect();
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(StalledBody {
                chunks,
                held: Arc::clone(&self.held),
            }),
        })
    }
}

struct StalledBody {
    chunks: VecDeque<Vec<u8>>,
    held: Arc<Notify>,
}

#[async_trait]
impl FetchBody for StalledBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        match self.chunks.pop_front() {
            Some(chunk) => Ok(Some(chunk)),
            None => {
                self.held.notify_one();
                std::future::pending().await
            }
        }
    }
}

/// Serves every chunk; the end-of-stream call cancels the turn's token
/// before returning, as a real cancel racing the end of the body would.
struct EndCancelNet {
    content: Arc<Vec<u8>>,
    chunk: usize,
    cancellation: CancellationToken,
    calls: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl FetchTransport for EndCancelNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(EndCancelBody {
                chunks: self
                    .content
                    .chunks(self.chunk)
                    .map(<[u8]>::to_vec)
                    .collect(),
                cancellation: self.cancellation.clone(),
            }),
        })
    }
}

struct EndCancelBody {
    chunks: VecDeque<Vec<u8>>,
    cancellation: CancellationToken,
}

#[async_trait]
impl FetchBody for EndCancelBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        match self.chunks.pop_front() {
            Some(chunk) => Ok(Some(chunk)),
            None => {
                self.cancellation.cancel();
                Ok(None)
            }
        }
    }
}

/// The cooperative resume fake: a ranged GET is answered with a 206 whose
/// `Content-Range` names the asked offset and a body of the slice from it;
/// a full GET answers 200 with the whole content. Logs the ranges asked.
struct RangedNet {
    content: Arc<Vec<u8>>,
    chunk: usize,
    calls: Arc<Mutex<Vec<String>>>,
    ranges: Arc<Mutex<Vec<Option<u64>>>>,
}

#[async_trait]
impl FetchTransport for RangedNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![RESOLVED])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        logged(request.url.as_str(), &self.calls);
        self.ranges
            .lock()
            .expect("ranges")
            .push(request.range_start);
        let (status, content_range, served): (u16, Option<String>, &[u8]) =
            match request.range_start {
                Some(start) => {
                    let start = start as usize;
                    (
                        206,
                        Some(format!(
                            "bytes {start}-{}/{}",
                            self.content.len() - 1,
                            self.content.len()
                        )),
                        &self.content[start..],
                    )
                }
                None => (200, None, &self.content),
            };
        Ok(WireResponse {
            status,
            location: None,
            content_range,
            body: Box::new(CannedBody {
                chunks: served.chunks(self.chunk).map(<[u8]>::to_vec).collect(),
            }),
        })
    }
}

struct CannedBody {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.chunks.pop_front())
    }
}

// --- test a: pre-cancelled ---------------------------------------------------

/// A download admitted on an already-cancelled turn refuses before anything
/// happens: no transport call, no part file, no sidecar, no destination.
#[tokio::test]
async fn a_precancelled_download_writes_nothing() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("precancel");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let net = RangedNet {
        content: Arc::new(content),
        chunk: 8,
        calls: Arc::clone(&calls),
        ranges: Arc::clone(&ranges),
    };
    let token = CancellationToken::new();
    token.cancel();
    let policy = policy();
    let error = tokio::time::timeout(
        ABORT,
        http_download_cancellable(DownloadCall {
            policy: &policy,
            transport: &net,
            workspace: &workspace,
            destination: DEST,
            expected_destination_sha256: None,
            url: &url("/doc"),
            limits: DownloadLimits::default(),
            budget: &budget,
            cancellation: &token,
        }),
    )
    .await
    .expect("a pre-cancelled download must not hang")
    .expect_err("a pre-cancelled download refuses");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "a pre-cancelled download makes no transport call: {:?}",
        calls.lock().expect("calls")
    );
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "nothing was written: no part file, no sidecar, no destination"
    );
}

// --- test b: cancelled header wait -------------------------------------------

/// The download is inside `get`, waiting for the response head, when the
/// turn cancels: the wait aborts typed, the transport saw the one call and
/// nothing after, and the session never opened — nothing was written.
#[tokio::test]
async fn cancelling_a_header_wait_writes_nothing() {
    let (root, workspace) = workspace("header-wait");
    let entered = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = HeldGetNet {
        entered: Arc::clone(&entered),
        calls: Arc::clone(&calls),
    };
    let budget = DownloadBudget::default();
    let token = CancellationToken::new();
    let task = spawn_download(net, workspace, budget, token.clone());
    entered.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled header wait aborts instead of hanging")
        .expect("the download task joins")
        .expect_err("cancellation is an error, never a result");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert_eq!(
        calls.lock().expect("calls").clone(),
        vec![url("/doc")],
        "the transport was entered once and never again"
    );
    assert!(
        !root.join(DEST).exists() && !root.join(PART).exists() && !root.join(SIDECAR).exists(),
        "a cancelled header wait wrote nothing: the session never opened"
    );
}

// --- test c: cancelled mid-body, exact partial, retry completes --------------

/// Two 8-byte chunks are served and fully written and claimed; the third
/// wait is held and the turn cancels there. The partial on disk holds
/// exactly the served bytes, the sidecar records exactly that length and
/// digest, the destination is untouched, and the budget holds the served
/// count — then the retry (same wallet, a cooperative ranged fake) completes
/// to the whole-file digest, with every byte claimed exactly once.
#[tokio::test]
async fn cancelling_mid_body_leaves_an_exact_resumable_partial_and_a_retry_completes() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("mid-body");
    let budget = DownloadBudget::default();
    let served = 16usize;

    let held = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = StalledNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        stall_after: 2,
        held: Arc::clone(&held),
        calls: Arc::clone(&calls),
    };
    let token = CancellationToken::new();
    let task = spawn_download(net, workspace.clone(), budget.clone(), token.clone());
    held.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled body wait aborts instead of hanging")
        .expect("the download task joins")
        .expect_err("cancellation is an error, never a result");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the partial exists"),
        content[..served],
        "exactly the served bytes, nothing more"
    );
    let sidecar = sidecar_json(&root);
    assert_eq!(
        sidecar["len"], served as u64,
        "the sidecar records exactly the served length: {sidecar}"
    );
    assert_eq!(
        sidecar["sha256"],
        digest(&content[..served]),
        "the sidecar digests exactly the served bytes"
    );
    assert!(
        !root.join(DEST).exists(),
        "the destination is never touched by a cancelled download"
    );
    assert_eq!(
        budget.consumed(),
        served as u64,
        "each served byte was claimed from the wallet exactly once"
    );

    let retry_calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let retry = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&retry_calls),
        ranges: Arc::clone(&ranges),
    };
    let outcome = tokio::time::timeout(
        ABORT,
        http_download(
            &policy(),
            &retry,
            &workspace,
            DEST,
            &url("/doc"),
            DownloadLimits::default(),
            &budget,
        ),
    )
    .await
    .expect("the retry must not hang")
    .expect("the retry completes");
    assert_eq!(outcome.bytes, content.len() as u64);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(
        std::fs::read(root.join(DEST)).expect("the destination"),
        content,
        "the retry completed the whole file"
    );
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
    assert_eq!(
        budget.consumed(),
        content.len() as u64,
        "each byte was claimed exactly once across both legs"
    );
    assert_eq!(
        ranges.lock().expect("ranges").clone(),
        vec![Some(served as u64)],
        "the retry asked for exactly the recorded offset"
    );
}

// --- test d: cancelled after the last byte, before publication ---------------

/// The body's end-of-stream call cancels the turn's token: the download
/// stops right before the promotion. Nothing is published; the complete
/// partial and its matching sidecar stay for an explicit retry.
#[tokio::test]
async fn cancelling_after_the_last_byte_but_before_publication_publishes_nothing() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("before-publish");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let token = CancellationToken::new();
    let net = EndCancelNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        cancellation: token.clone(),
        calls: Arc::clone(&calls),
    };
    let task = spawn_download(net, workspace, budget, token.clone());
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("the download must not hang")
        .expect("the download task joins")
        .expect_err("cancelled before publication, so an error");
    assert!(matches!(error, DownloadError::Cancelled), "{error:?}");
    assert!(
        !root.join(DEST).exists(),
        "nothing was published: the destination is absent"
    );
    assert_eq!(
        std::fs::read(root.join(PART)).expect("the complete partial stays"),
        content,
        "the complete partial is left for an explicit retry"
    );
    let sidecar = sidecar_json(&root);
    assert_eq!(sidecar["len"], content.len() as u64);
    assert_eq!(sidecar["sha256"], digest(&content));
}

// --- test e: the tool adapter reports cancellation typed ----------------------

/// Through `FetchTools` and the loop's `execute_with_context`, a download
/// cancelled mid-body surfaces as the lane's typed detail — exactly
/// "download cancelled"; and an uncancelled context completes normally with
/// the metadata result.
#[tokio::test]
async fn the_tool_adapter_reports_a_cancelled_download_typed() {
    let content: Vec<u8> = (0..40u8).collect();
    let (_root, cancel_workspace) = workspace("adapter-cancel");
    let held = Arc::new(Notify::new());
    let calls = Arc::new(Mutex::new(Vec::new()));
    let net = StalledNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        stall_after: 2,
        held: Arc::clone(&held),
        calls: Arc::clone(&calls),
    };
    let tools = FetchTools::new(
        policy(),
        Arc::new(net),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::new(cancel_workspace)),
        DownloadBudget::default(),
    );
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            tools
                .execute_with_context(
                    "http_download",
                    serde_json::json!({"url": url("/doc"), "destination": DEST}),
                    ToolExecutionContext {
                        result_cap: FetchLimits::tool_lane_cap(),
                        cancellation: token,
                    },
                )
                .await
        }
    });
    held.notified().await;
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled download aborts instead of hanging")
        .expect("the tool task joins")
        .expect_err("cancellation is an error, never a result");
    let ToolError::Fetch(text) = error else {
        panic!("the download lane's typed error, got: {error:?}")
    };
    assert_eq!(text, "download cancelled", "the lane's cancellation detail");

    // The positive control: an uncancelled context completes normally.
    let (_ok_root, ok_workspace) = workspace("adapter-ok");
    let ok_calls = Arc::new(Mutex::new(Vec::new()));
    let ok_ranges = Arc::new(Mutex::new(Vec::new()));
    let ok_net = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&ok_calls),
        ranges: Arc::clone(&ok_ranges),
    };
    let ok_tools = FetchTools::new(
        policy(),
        Arc::new(ok_net),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::new(ok_workspace)),
        DownloadBudget::default(),
    );
    let result = tokio::time::timeout(
        ABORT,
        ok_tools.execute_with_context(
            "http_download",
            serde_json::json!({"url": url("/doc"), "destination": OK_DEST}),
            ToolExecutionContext {
                result_cap: FetchLimits::tool_lane_cap(),
                cancellation: CancellationToken::new(),
            },
        ),
    )
    .await
    .expect("an uncancelled download must not hang")
    .expect("the uncancelled download completes");
    assert_eq!(result["destination"], OK_DEST);
    assert_eq!(result["bytes"], 40);
    assert_eq!(result["sha256"], digest(&content));
}

// --- test f: the old wrapper still completes ----------------------------------

/// The regression leg: an uncancelled `http_download` through the unchanged
/// 7-parameter wrapper still downloads, promotes, and clears the sidecar.
#[tokio::test]
async fn an_uncancelled_download_through_the_old_wrapper_still_completes() {
    let content: Vec<u8> = (0..40u8).collect();
    let (root, workspace) = workspace("wrapper");
    let budget = DownloadBudget::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let net = RangedNet {
        content: Arc::new(content.clone()),
        chunk: 8,
        calls: Arc::clone(&calls),
        ranges: Arc::clone(&ranges),
    };
    let outcome = tokio::time::timeout(
        ABORT,
        http_download(
            &policy(),
            &net,
            &workspace,
            DEST,
            &url("/doc"),
            DownloadLimits::default(),
            &budget,
        ),
    )
    .await
    .expect("the wrapper must not hang")
    .expect("the uncancelled download completes");
    assert_eq!(outcome.bytes, content.len() as u64);
    assert_eq!(outcome.sha256, digest(&content));
    assert_eq!(
        std::fs::read(root.join(DEST)).expect("the destination"),
        content
    );
    assert!(!root.join(SIDECAR).exists(), "the sidecar is cleared");
}
