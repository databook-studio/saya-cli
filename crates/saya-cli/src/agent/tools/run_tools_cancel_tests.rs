//! The composite's cancellation forwarding: `RunTools::execute_with_context`
//! must forward the loop's turn token to the scratch and fetch members, whose
//! harness implementations already honour it (A01–A04). The database
//! fall-through keeps its existing behaviour, and `execute` /
//! `execute_with_result_cap` keep minting fresh tokens. Every await sits
//! under a guard so a regression hangs into a failure instead of stalling
//! the suite.

use std::{
    fs,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use saya_agent::{
    AgentError, AgentLimits, AgentRequest, AllowReadOnlyApproval, ChatProvider, ChatRequest,
    ChatResponse, NoopEventSink, ProviderError, ProviderEvent, ProviderStream, ToolCall,
    ToolDefinition, run_agent_with_sink,
};
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchBody, FetchDestination, FetchLimits, FetchPolicy,
    FetchRequest, FetchTransport, FetchTransportError, WireResponse,
};
use saya_harness::workspace::Workspace;
use serde_json::json;
use tokio::sync::Notify;

use super::*;

/// The runaway cross product, straight from the scratch battery: it never
/// finishes on its own, so only the turn token (or the 30 s ceiling) ends it.
const RUNAWAY: &str = "SELECT count(*) FROM range(100000000) a, range(100000000) b";
/// The guard every await sits under, generous against CI jitter.
const GUARD: Duration = Duration::from_secs(15);
/// The tighter guard for the fetch paths, which must abort in milliseconds.
const ABORT: Duration = Duration::from_secs(5);
/// The fetch lane's fake host — a policy-approved TEST-NET destination.
const HOST: &str = "files.example.org";
/// The untrusted-context delimiters a fetch success must carry.
const OPEN: &str = "<<<CONTEXT_BLOCK_BEGIN>>>";
const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";

/// The loop-assigned context: the tool-message cap and the turn's token.
fn context(token: &CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: saya_agent::MAX_TOOL_MESSAGE_BYTES,
        cancellation: token.clone(),
    }
}

/// A private root under the OS temp dir, removed on drop. The tests run in
/// parallel, so every root is its own.
struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-run-tools-cancel-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A scratch member over a private root, as a session surface composes it.
fn scratch_member(label: &str) -> (TempDir, Arc<ScratchSql>) {
    let root = TempDir::new(label);
    let scratch = Arc::new(ScratchSql::open(&root.0).unwrap());
    (root, scratch)
}

/// The composite with a scratch member and the test-only database member.
fn scratch_composite(scratch: &Arc<ScratchSql>) -> RunTools {
    RunTools::compose(
        Arc::new(DatabaseTools::new(None, 100, true)),
        Some(Arc::clone(scratch)),
        None,
        None,
    )
}

/// A fetch member over a private workspace root, with a hermetic transport.
fn fetch_member(
    label: &str,
    transport: impl FetchTransport + 'static,
) -> (TempDir, Arc<FetchTools>) {
    let root = TempDir::new(label);
    let tools = FetchTools::new(
        FetchPolicy::new([FetchDestination::new("https", HOST)]),
        Arc::new(transport),
        FetchLimits::for_tool_lane(),
        DownloadLimits::default(),
        Some(Arc::new(Workspace::open(&root.0).unwrap())),
        DownloadBudget::default(),
    );
    (root, Arc::new(tools))
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

// --- the hermetic transports -------------------------------------------------

/// A transport whose request/headers wait never resolves, signalling the
/// test when `get` is entered so the cancel lands while the fetch waits.
struct HeldHeadersNet {
    entered: Arc<Notify>,
}

#[async_trait]
impl FetchTransport for HeldHeadersNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, _: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.entered.notify_one();
        std::future::pending().await
    }
}

/// Serves one canned body to any policy-judged URL — the regression transport.
struct CannedNet {
    body: &'static [u8],
}

#[async_trait]
impl FetchTransport for CannedNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, _: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(CannedBody {
                chunk: Some(self.body.to_vec()),
            }),
        })
    }
}

struct CannedBody {
    chunk: Option<Vec<u8>>,
}

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.chunk.take())
    }
}

// --- the required cases ------------------------------------------------------

/// The composite forwards the turn's token to the scratch member: the runaway
/// statement is interrupted, the cancelled detail surfaces, the elapsed time
/// is well under the 30 s query ceiling, and the released connection still
/// serves a follow-up statement through the same composite.
#[tokio::test]
async fn composite_forwards_the_turn_token_to_scratch() {
    let (_root, scratch) = scratch_member("scratch");
    let composite = scratch_composite(&scratch);
    let token = CancellationToken::new();
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            token.cancel();
        })
    };
    let started = Instant::now();
    let error = tokio::time::timeout(
        GUARD,
        composite.execute_with_context("scratch_sql", json!({"sql": RUNAWAY}), context(&token)),
    )
    .await
    .expect("the guard: the turn token must interrupt the runaway statement")
    .expect_err("a cancelled call is an error, never a result");
    let elapsed = started.elapsed();
    canceller.await.unwrap();
    match error {
        ToolError::QueryFailedDetail(detail) => {
            assert!(
                detail.contains("cancelled"),
                "the cancelled detail: {detail}"
            );
            assert!(
                !detail.contains("timed out"),
                "never the ceiling path: {detail}"
            );
        }
        other => panic!("cancellation must surface through the composite, got {other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(5),
        "the cancel must land well under the 30 s query ceiling, took {elapsed:?}"
    );
    // The interrupt-and-join released the connection: the composite still serves.
    let answer = tokio::time::timeout(
        GUARD,
        composite.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT 42 AS answer"}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("the follow-up guard")
    .unwrap();
    assert_eq!(answer["rows"], json!([[42]]));
}

/// The composite forwards the turn's token to the fetch member: a fetch
/// waiting on request/headers returns the lane's typed cancellation instead
/// of hanging to its own 30 s budget.
#[tokio::test]
async fn composite_forwards_the_turn_token_to_fetch() {
    let entered = Arc::new(Notify::new());
    let (_root, fetch) = fetch_member(
        "fetch",
        HeldHeadersNet {
            entered: Arc::clone(&entered),
        },
    );
    let composite = RunTools::compose(
        Arc::new(DatabaseTools::new(None, 100, true)),
        None,
        Some(fetch),
        None,
    );
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            composite
                .execute_with_context("http_fetch", json!({"url": url("/doc")}), context(&token))
                .await
        }
    });
    entered.notified().await; // the fetch is inside `get`, now waiting
    token.cancel();
    let error = tokio::time::timeout(ABORT, task)
        .await
        .expect("a cancelled fetch aborts instead of hanging")
        .expect("the fetch task joins")
        .expect_err("cancellation is an error, never a result");
    match error {
        ToolError::Fetch(detail) => {
            assert_eq!(detail, "fetch cancelled", "the lane's cancellation detail")
        }
        other => panic!("the fetch lane's typed error, got: {other:?}"),
    }
}

/// The regression pin: an uncancelled turn keeps both lanes working through
/// the composite — the scratch statement's rows and the fetch's contained
/// block arrive exactly as before the forwarding existed.
#[tokio::test]
async fn an_uncancelled_turn_keeps_scratch_and_fetch_working_through_the_composite() {
    let (_root, scratch) = scratch_member("regression-scratch");
    let composite = scratch_composite(&scratch);
    let answer = tokio::time::timeout(
        GUARD,
        composite.execute_with_context(
            "scratch_sql",
            json!({"sql": "SELECT 41 + 1 AS answer"}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("the guard")
    .unwrap();
    assert_eq!(answer["rows"], json!([[42]]));
    let (_fetch_root, fetch) = fetch_member(
        "regression-fetch",
        CannedNet {
            body: b"an ordinary page",
        },
    );
    let composite = RunTools::compose(
        Arc::new(DatabaseTools::new(None, 100, true)),
        None,
        Some(fetch),
        None,
    );
    let value = tokio::time::timeout(
        ABORT,
        composite.execute_with_context(
            "http_fetch",
            json!({"url": url("/doc")}),
            context(&CancellationToken::new()),
        ),
    )
    .await
    .expect("an uncancelled fetch completes")
    .unwrap();
    let content = value["content"].as_str().unwrap();
    assert!(
        content.contains("an ordinary page") && content.contains(OPEN) && content.contains(CLOSE),
        "the whole body rides the contained block: {content}"
    );
    assert_eq!(value["url"].as_str().unwrap(), url("/doc"));
}

// --- the loop-level gate -----------------------------------------------------

/// The loop provider: one assistant message carrying two tool calls, then a
/// plain answer afterwards. Counts every provider request so the test can
/// assert the cancelled turn never asked again.
struct TwoCallsProvider {
    calls: Vec<ToolCall>,
    turn: std::sync::Mutex<u32>,
    requests: Arc<std::sync::Mutex<u32>>,
}

#[async_trait]
impl ChatProvider for TwoCallsProvider {
    fn name(&self) -> &str {
        "two-calls-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        _: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        *self.requests.lock().unwrap() += 1;
        let first = {
            let mut turn = self.turn.lock().unwrap();
            let was = *turn;
            *turn += 1;
            was == 0
        };
        let events = if first {
            vec![
                Ok(ProviderEvent::ToolCalls(self.calls.clone())),
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

fn request() -> AgentRequest {
    AgentRequest {
        prompt: "stage then write".into(),
        profile_names: Vec::new(),
        model: "mock-model".into(),
        system_prompt: None,
        history: Vec::new(),
        context_blocks: Vec::new(),
    }
}

/// The real definition, taken from the advertised list, so the loop exercises
/// the gate against the definition the model would actually see.
fn workspace_write_definition() -> ToolDefinition {
    DatabaseTools::definitions(false, false, false, true, false)
        .into_iter()
        .find(|tool| tool.name == "workspace_write")
        .expect("workspace_write is advertised when writes are permitted")
}

/// Through the real loop over the real composite: the turn's token cancelled
/// mid-scratch stops the run — the follow-up `workspace_write` canary never
/// lands, and the provider is asked only once. The batch is conservatively
/// serial (both tools declare `Serial`), so once the cancelled scratch call
/// settles, the loop's own `check_cancelled` ends the run before the canary
/// call is ever reached.
#[tokio::test]
async fn a_cancelled_turn_runs_no_follow_up_effect_through_the_real_loop() {
    let ws_root = TempDir::new("loop-ws");
    let workspace = Arc::new(Workspace::open(&ws_root.0).unwrap());
    let (_scratch_root, scratch) = scratch_member("loop-scratch");
    let database = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    )
    .with_workspace(Some(Arc::clone(&workspace)));
    let composite = RunTools::compose(Arc::new(database), Some(scratch), None, None);
    let calls = vec![
        ToolCall {
            id: "c1".into(),
            name: "scratch_sql".into(),
            arguments: json!({"sql": RUNAWAY}),
        },
        ToolCall {
            id: "c2".into(),
            name: "workspace_write".into(),
            arguments: json!({"path": "canary.txt", "content": "canary"}),
        },
    ];
    let requests = Arc::new(std::sync::Mutex::new(0));
    let provider = TwoCallsProvider {
        calls,
        turn: std::sync::Mutex::new(0),
        requests: Arc::clone(&requests),
    };
    let definitions = vec![ScratchSql::definition(), workspace_write_definition()];
    let token = CancellationToken::new();
    let canceller = {
        let token = token.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            token.cancel();
        })
    };
    let started = Instant::now();
    let outcome = tokio::time::timeout(
        GUARD,
        run_agent_with_sink(
            &provider,
            &composite,
            request(),
            definitions,
            AgentLimits {
                permit_workspace_writes: true,
                ..AgentLimits::default()
            },
            &AllowReadOnlyApproval,
            &NoopEventSink,
            token,
        ),
    )
    .await
    .expect("the guard: the turn token must reach the scratch member and stop the run");
    canceller.await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        matches!(outcome, Err(AgentError::Cancelled)),
        "the run must end cancelled, got {outcome:?}"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the run must end cancelled quickly, took {elapsed:?}"
    );
    assert!(
        !ws_root.0.join("canary.txt").exists(),
        "the cancelled turn must not run the follow-up write"
    );
    assert_eq!(
        *requests.lock().unwrap(),
        1,
        "the provider is asked only once"
    );
}
