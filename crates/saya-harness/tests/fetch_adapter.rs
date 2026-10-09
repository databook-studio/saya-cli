//! The fetch adapter battery: the toolset-level lane the model's
//! `http_fetch` calls ride, over a hermetic in-process transport — no test
//! touches the real network.
//!
//! The S2 decision's required cases: the pre-bound (a body over the wired
//! bound is a typed error, never a short success; a body at the bound
//! arrives closed with no loop truncation marker), the backstop (an
//! over-cap render is cut on the body, the block stays closed), the
//! sentinel battery ported to the adapter's output, the never-raw pin, the
//! latch trip through the adapter's download, and the loop-level gate: a
//! provider issuing `http_fetch` sees a contained block in the tool message
//! and a conversation that keeps its shape.

use std::collections::VecDeque;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use saya_agent::{
    AgentLimits, AgentRequest, CancellationToken, ChatProvider, ChatRequest, ChatResponse,
    ProviderError, ProviderEvent, ProviderStream, ToolCall, ToolError, ToolExecutionContext,
    ToolExecutor, run_agent_with_sink,
};
use saya_harness::fetch::{
    DownloadBudget, DownloadLimits, FetchBody, FetchDestination, FetchLimits, FetchPolicy,
    FetchRequest, FetchTools, FetchTransport, FetchTransportError, WireResponse,
};
use saya_harness::workspace::Workspace;
use tokio::sync::Notify;

const OPEN: &str = "<<<CONTEXT_BLOCK_BEGIN>>>";
const CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";
const HOST: &str = "files.example.org";
// --- the hermetic transport: one machine, canned bodies ---------------------

/// Serves one canned body, in declared chunks, to any policy-judged URL.
#[derive(Clone)]
struct StaticNet {
    chunks: Vec<Vec<u8>>,
    calls: Arc<Mutex<Vec<String>>>,
}

impl StaticNet {
    fn serve(body: &[u8], chunk: usize) -> (Self, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                chunks: if chunk >= body.len() {
                    vec![body.to_vec()]
                } else {
                    body.chunks(chunk).map(<[u8]>::to_vec).collect()
                },
                calls: calls.clone(),
            },
            calls,
        )
    }
}

#[async_trait]
impl FetchTransport for StaticNet {
    async fn resolve(&self, _: &str) -> Result<Vec<IpAddr>, FetchTransportError> {
        Ok(vec![IpAddr::V4(Ipv4Addr::new(192, 0, 2, 7))])
    }

    async fn get(&self, request: FetchRequest) -> Result<WireResponse, FetchTransportError> {
        self.calls
            .lock()
            .expect("log")
            .push(request.url.as_str().to_owned());
        Ok(WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(CannedBody {
                chunks: VecDeque::from(self.chunks.clone()),
            }),
        })
    }
}

/// The canned body, chunk by chunk.
struct CannedBody {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl FetchBody for CannedBody {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.chunks.pop_front())
    }
}

// --- the fixture ------------------------------------------------------------

fn destinations() -> Vec<FetchDestination> {
    vec![FetchDestination::new("https", HOST)]
}

fn url(path: &str) -> String {
    format!("https://{HOST}{path}")
}

/// One isolated workspace, as `assemble` opens it per run.
fn workspace(label: &str) -> (PathBuf, Arc<Workspace>) {
    let root =
        std::env::temp_dir().join(format!("saya-fetch-adapter-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp workspace");
    (
        root.clone(),
        Arc::new(Workspace::open(&root).expect("workspace opens")),
    )
}

/// The step's adapter, as the composition root builds it: the step's policy
/// over the run's shared transport, workspace, and wallet.
fn adapter(
    transport: impl FetchTransport + 'static,
    workspace: Arc<Workspace>,
    budget: DownloadBudget,
    limits: FetchLimits,
) -> FetchTools {
    FetchTools::new(
        FetchPolicy::new(destinations()),
        Arc::new(transport),
        limits,
        DownloadLimits::default(),
        Some(workspace),
        budget,
    )
}

/// The lane body bound, the same derivation the composition root uses.
fn lane_bound() -> usize {
    FetchLimits::for_tool_lane().max_total_bytes
}

#[path = "fetch_adapter/lane_bounds.rs"]
mod lane_bounds;

#[path = "fetch_adapter/sentinel_delivery.rs"]
mod sentinel_delivery;

#[path = "fetch_adapter/download_budget.rs"]
mod download_budget;

// --- the loop-level gate -----------------------------------------------------

/// A provider that emits one tool call, records the next request, and
/// finishes — the seam the loop-level assertions read.
struct RecordingProvider {
    call: ToolCall,
    requests: Arc<Mutex<Vec<ChatRequest>>>,
    turn: Mutex<u32>,
}

#[async_trait]
impl ChatProvider for RecordingProvider {
    fn name(&self) -> &str {
        "fetch-adapter-mock"
    }
    async fn complete(&self, _: ChatRequest) -> Result<ChatResponse, ProviderError> {
        unreachable!("stream path is used")
    }
    async fn stream(
        &self,
        request: ChatRequest,
        _: CancellationToken,
    ) -> Result<ProviderStream, ProviderError> {
        self.requests
            .lock()
            .expect("requests")
            .push(request.clone());
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

#[path = "fetch_adapter/loop_delivery.rs"]
mod loop_delivery;

// --- turn cancellation -------------------------------------------------------
//
// The loop hands every call a `ToolExecutionContext` carrying the turn's
// token, and `http_fetch` rides it: a cancelled turn stops waiting on the
// transport, never begins another redirect hop, and never reports a result
// that only finished after cancellation. Every case is under a timeout guard
// so a regression hangs into a failure instead of stalling the suite.

const ABORT: Duration = Duration::from_secs(5);

#[path = "fetch_adapter/cancellation_fixtures.rs"]
mod cancellation_fixtures;
use cancellation_fixtures::*;

#[path = "fetch_adapter/turn_waits.rs"]
mod turn_waits;

#[path = "fetch_adapter/turn_hops.rs"]
mod turn_hops;
