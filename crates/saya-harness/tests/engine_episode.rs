//! The episode driver — contract tests (M1-5, episode slice).
//!
//! Five guarantees, one per test:
//! 1. A tool outside the step's capabilities is **absent** from the
//!    definitions the loop receives — hidden, not advertised-and-refused.
//! 2. The step's budget becomes the loop's `AgentLimits`, and
//!    `SAYA_AGENT_MAX_TURNS` set in the environment changes nothing (a run
//!    is reproducible from its spec and config alone — plan G3).
//! 3. `permit_candidate_writes` is pinned false by construction, even when
//!    the caller carries the user's memory-mode permission to write
//!    candidates (DESIGN §5.8).
//! 4. A failing episode retries a bounded number of times, each with a
//!    fresh brief, then records a typed failure and pauses — never
//!    unbounded, never silent.
//! 5. The brief carries the workspace manifest: names, sizes, digests.
//!
//! The scope-wiring slices add their own gates on the same harness; S1's
//! (scratch) proves a scratch-only step runs DDL through `scratch_sql` in a
//! real episode.

use std::{
    collections::VecDeque,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, CancellationToken, ChatMessage, ChatProvider, ChatRequest, ChatResponse,
    LocalStateEffect, ProviderError, ToolCall, ToolDefinition, ToolEffect, ToolError, ToolExecutor,
};
use saya_harness::engine::{
    EngineEventSink, EpisodeCollaborators, EpisodeDriver, EpisodeError, EpisodeRequest, EpisodeRun,
    ManifestBounds, RunState, SinkBudgets, StepToolset, UsageTotals,
};
use saya_harness::fetch::{FetchDestination, FetchTransportError};
use saya_harness::journal::Journal;
use saya_harness::workspace::{Workspace, manifest};
use saya_store::{
    NewRun, RunBudgets, RunCapabilityFlags, RunStatus, RunStepStatus, RunStore, SqliteStateStore,
};
use saya_types::{
    Budgets, Capabilities, PauseReason, RunEvent, RunFailureCode, RunId, RunPlan, StepSpec,
};

#[path = "engine_episode/counts.rs"]
mod counts;
#[path = "engine_episode/deadline.rs"]
mod deadline;

include!("engine_episode/fixtures/run.rs");
include!("engine_episode/fixtures/provider.rs");
include!("engine_episode/fixtures/tools.rs");
include!("engine_episode/fixtures/driver.rs");

#[path = "engine_episode/capability_scope.rs"]
mod capability_scope;

#[path = "engine_episode/budgets.rs"]
mod budgets;

#[path = "engine_episode/candidate_writes.rs"]
mod candidate_writes;

#[path = "engine_episode/retry_brief.rs"]
mod retry_brief;

#[path = "engine_episode/scratch.rs"]
mod scratch;

// --- S2: the fetch step, end to end ------------------------------------------

/// A hermetic in-process transport: every policy-judged URL resolves to a
/// public TEST-NET address and is served the canned body in declared
/// chunks. No test touches the real network.
struct FetchNet {
    chunks: VecDeque<Vec<u8>>,
}

#[async_trait]
impl saya_harness::fetch::FetchTransport for FetchNet {
    async fn resolve(&self, _: &str) -> Result<Vec<std::net::IpAddr>, FetchTransportError> {
        Ok(vec![std::net::IpAddr::V4(std::net::Ipv4Addr::new(
            192, 0, 2, 7,
        ))])
    }
    async fn get(
        &self,
        _: saya_harness::fetch::FetchRequest,
    ) -> Result<saya_harness::fetch::WireResponse, FetchTransportError> {
        Ok(saya_harness::fetch::WireResponse {
            status: 200,
            location: None,
            content_range: None,
            body: Box::new(FetchChunks(self.chunks.clone())),
        })
    }
}

/// The canned body, chunk by chunk.
struct FetchChunks(VecDeque<Vec<u8>>);

#[async_trait]
impl saya_harness::fetch::FetchBody for FetchChunks {
    async fn next_chunk(&mut self) -> Result<Option<Vec<u8>>, FetchTransportError> {
        Ok(self.0.pop_front())
    }
}

/// The bytes on disk under a directory tree, summed — sidecar `.json`
/// files excluded (they are the resume contract, not downloaded bytes).
fn walk_dir_bytes(root: &std::path::Path) -> u64 {
    std::fs::read_dir(root)
        .expect("tree")
        .filter_map(|entry| entry.ok())
        .map(|entry| {
            let path = entry.path();
            if path.is_dir() {
                walk_dir_bytes(&path)
            } else if path.to_string_lossy().ends_with(".json") {
                0
            } else {
                entry.metadata().expect("size").len()
            }
        })
        .sum()
}

#[path = "engine_episode/downloads.rs"]
mod downloads;
