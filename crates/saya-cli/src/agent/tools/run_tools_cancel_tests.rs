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

#[path = "run_tools_cancel_tests/loop_cancellation.rs"]
mod loop_cancellation;
#[path = "run_tools_cancel_tests/member_cancellation.rs"]
mod member_cancellation;
