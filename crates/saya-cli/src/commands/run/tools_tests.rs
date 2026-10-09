//! The toolset builder's gates: the per-step toolsets must carry the run's
//! database and workspace universe byte-identically to the single executor
//! and run-level universe they replaced — and, since S1, the scratch tool
//! must appear in — and only in — the steps that asked for it, behind a
//! composite that actually runs it. The same holds for fetch (S2) and the
//! runner (S3), whose proven spawn narrows per step to the `RunnerScope`
//! that step itself asked for.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_agent::{LocalStateEffect, ToolError};
use saya_harness::fetch::{
    DownloadBudget, FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse,
};
use saya_harness::scratch::ScratchSql;
use saya_harness::workspace::Workspace;
#[cfg(target_os = "macos")]
use saya_types::InterpreterScope;
use saya_types::{Capabilities, RunnerScope, StepSpec};

use crate::agent::tools::DatabaseTools;

#[cfg(target_os = "macos")]
use super::runner::RunRunner;
use super::tools::{RunFetch, ToolsetInputs, toolsets};

/// The run's cancellation token, as the composition root passes it.
fn cancellation() -> CancellationToken {
    CancellationToken::default()
}

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, no chart (a step that approved no egress approves no chart),
/// and no contract tools (a run passes no state store). The clarification
/// ask (B3c) rides the always-available set, right after `schema_discovery`.
/// `workspace_write` sits between `grep` and the sql tools when approved; the
/// scope-asking tails append after it.
const OPEN_GATE: &[&str] = &[
    "schema_discovery",
    "request_clarification",
    "workspace_read",
    "workspace_list",
    "glob",
    "grep",
    "bounded_sql_query",
    "bounded_sql_query_all",
    "result_shape",
    "column_health",
    "join_check",
    "designate_answer",
];
/// The same universe for a step that approved an egress scope (fetch or
/// runner): `render_chart` sits where it always did, between `join_check`
/// and `designate_answer`.
const OPEN_GATE_WITH_CHART: &[&str] = &[
    "schema_discovery",
    "request_clarification",
    "workspace_read",
    "workspace_list",
    "glob",
    "grep",
    "bounded_sql_query",
    "bounded_sql_query_all",
    "result_shape",
    "column_health",
    "join_check",
    "render_chart",
    "designate_answer",
];
/// The tools each scope-asking step gets appended after the database
/// universe, in construction order: scratch, then fetch's pair, then the
/// runner's `run_program`.
const SCRATCH_TAIL: &[&str] = &["scratch_sql", "scratch_import"];
const FETCH_TAIL: &[&str] = &["http_fetch", "http_download"];
#[cfg(target_os = "macos")]
const RUNNER_TAIL: &[&str] = &["run_program"];

/// The same universe with the privacy gate closed: every tool that touches
/// database data is hidden.
const CLOSED_GATE: &[&str] = &[
    "schema_discovery",
    "request_clarification",
    "workspace_read",
    "workspace_list",
    "glob",
    "grep",
];

fn names(toolsets: &[saya_harness::engine::StepToolset], step: usize) -> Vec<String> {
    toolsets[step]
        .definitions
        .iter()
        .map(|definition| definition.name.clone())
        .collect()
}

fn step(goal: &str, capabilities: Capabilities) -> StepSpec {
    StepSpec::new(goal, capabilities, None, Vec::new(), None).unwrap()
}

/// An admitted `ScratchSql` over a private run root, as `assemble` builds
/// it for a run that approved scratch.
fn admitted_scratch(label: &str) -> (PathBuf, Arc<ScratchSql>) {
    let mut capabilities = Capabilities::default();
    capabilities.scratch = true;
    let root = std::env::temp_dir().join(format!("saya-run-tools-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let scratch = ScratchSql::admit(&root, &capabilities)
        .unwrap()
        .expect("the run approved scratch, so admission admits");
    (root, Arc::new(scratch))
}

/// An open `Workspace` over a private root — the `Arc` `assemble` builds
/// once per run and hands the toolset builder. The label keeps each test's
/// root its own: the tests run in parallel, and a shared root would let one
/// test's cleanup wipe another's in-flight writes.
fn workspace(label: &str) -> Arc<Workspace> {
    let root =
        std::env::temp_dir().join(format!("saya-run-tools-ws-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    Arc::new(Workspace::open(&root).unwrap())
}

/// The run-level fetch wiring with a hermetic in-process transport: every
/// policy-judged URL resolves to a public TEST-NET address and is served
/// one canned body. No test touches the real network. Returns the request
/// log (the witness for the assertion that nothing was ever requested
/// where it must not be) and the wiring.
fn run_fetch(body: &[u8]) -> (Arc<std::sync::Mutex<Vec<String>>>, RunFetch) {
    use std::collections::VecDeque;

    #[derive(Clone)]
    struct StaticNet {
        chunks: VecDeque<Vec<u8>>,
        calls: Arc<std::sync::Mutex<Vec<String>>>,
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
                    chunks: self.chunks.clone(),
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

    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let net = StaticNet {
        chunks: VecDeque::from(vec![body.to_vec()]),
        calls: calls.clone(),
    };
    (
        calls,
        RunFetch {
            transport: Arc::new(net),
            budget: DownloadBudget::default(),
        },
    )
}

include!("tools_tests/step_capabilities.rs");

include!("tools_tests/scratch_scope.rs");

include!("tools_tests/fetch_scope.rs");

include!("tools_tests/runner_absence.rs");

#[cfg(target_os = "macos")]
include!("tools_tests/runner_scenarios.rs");
