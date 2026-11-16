//! The toolset builder's gates: the per-step toolsets must carry the run's
//! database and workspace universe byte-identically to the single executor
//! and run-level universe, and the fetch tools must appear in — and only
//! in — the steps that asked for them, behind a composite that actually
//! runs them. The scope narrows per step to the `FetchScope` that step
//! itself asked for. The command door is no scope: every step carries the
//! run's one `shell` (see `tools_tests/shell_member.rs`).

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_agent::{LocalStateEffect, ToolError};
use saya_harness::fetch::{
    DownloadBudget, FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse,
};
use saya_harness::workspace::Workspace;
use saya_types::{Capabilities, StepSpec};

use crate::agent::tools::DatabaseTools;

use super::tools::{RunFetch, ToolsetInputs, run_shell, toolsets};

/// A fresh cancellation token, as the composition root passes one to the
/// run's shell.
fn cancellation() -> CancellationToken {
    CancellationToken::default()
}

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, no chart (the model tool is retired), and no contract tools
/// (a run passes no state store). The clarification
/// ask rides the always-available set, right after `schema_discovery`.
/// `write` sits between `grep` and the sql tools when approved, and the
/// scope-asking tails append after it; `notebook_edit` — added after this
/// order was fixed — goes last of all, and only where the run's workspace
/// holds a notebook (`tools_tests/notebook_presence.rs`).
const OPEN_GATE: &[&str] = &[
    "schema_discovery",
    "request_clarification",
    "read",
    "glob",
    "grep",
    "bounded_sql_query",
    "bounded_sql_query_all",
    "result_shape",
    "column_health",
    "join_check",
    "designate_answer",
];
/// The tools each scope-asking step gets appended after the database
/// universe, in construction order: fetch's pair.
const FETCH_TAIL: &[&str] = &["http_fetch", "http_download"];

/// The same universe with the privacy gate closed: every tool that touches
/// database data is hidden.
const CLOSED_GATE: &[&str] = &[
    "schema_discovery",
    "request_clarification",
    "read",
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
                content_type: None,
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

/// The names the widest possible step composition produces — every scope
/// approved, a workspace bound, the run's shell composed — for the allow-list
/// drift test in `agent::tools::tools_allow_drift_tests`. Built through
/// `toolsets`, the same function the composition root calls, so a definition
/// pushed there is what moves that test.
pub(crate) fn widest_composed_names() -> Vec<String> {
    // A per-call label: the drift tests run in parallel and each one builds
    // this composition, so a shared temp root would let one call's cleanup
    // wipe another's in-flight workspace.
    static DRIFT_CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let label = format!(
        "drift-widest-{}",
        DRIFT_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let ws = workspace(&label);
    // A notebook, so the widest composition really is the widest: the notebook
    // edit is offered only where the run's workspace holds one.
    fs::write(ws.root().join("widest.ipynb"), b"{\"cells\":[]}").unwrap();
    let (_calls, fetch) = run_fetch(b"drift");
    let shell = run_shell(
        &ws,
        saya_agent::ApprovalPolicy::Ask,
        None,
        saya_harness::runner::HostPolicy::default(),
        cancellation(),
        saya_config::CommandRules::default(),
    );
    let mut capabilities = Capabilities::default();
    capabilities.workspace_write = true;
    capabilities.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("a shaped scope"),
    );
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            fetch: Some(&fetch),
            shell: shell.as_ref(),
            workspace: &ws,
            allow_query_data: true,
            tools: &saya_config::ResolvedTools::default(),
            mcp: None,
            hooks: None,
        },
        &[step("drift", capabilities)],
    );
    names(&built, 0)
}

include!("tools_tests/step_capabilities.rs");

include!("tools_tests/fall_through.rs");

include!("tools_tests/fetch_scope.rs");

include!("tools_tests/shell_member.rs");

include!("tools_tests/tools_allow.rs");

include!("tools_tests/notebook_presence.rs");

include!("tools_tests/hooks.rs");

include!("tools_tests/mcp_member.rs");

include!("tools_tests/advertisement.rs");
