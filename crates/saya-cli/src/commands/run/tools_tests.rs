//! The toolset builder's gates: the per-step toolsets must carry the run's
//! database and workspace universe byte-identically to the single executor
//! and run-level universe they replaced — and, since S1, the scratch tool
//! must appear in — and only in — the steps that asked for it, behind a
//! composite that actually runs it.

use std::fs;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{LocalStateEffect, ToolError};
use saya_harness::fetch::{
    DownloadBudget, FetchBody, FetchRequest, FetchTransport, FetchTransportError, WireResponse,
};
use saya_harness::scratch::ScratchSql;
use saya_harness::workspace::Workspace;
use saya_types::{Capabilities, StepSpec};

use crate::agent::tools::DatabaseTools;

use super::tools::{RunFetch, toolsets};

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, and no contract tools (a run passes no state store).
/// `workspace_write` sits between `grep` and the sql tools when approved;
/// `scratch_sql` is appended last when the step asked for scratch.
const OPEN_GATE: &[&str] = &[
    "schema_discovery",
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

/// The same universe with the privacy gate closed: every tool that touches
/// database data is hidden.
const CLOSED_GATE: &[&str] = &[
    "schema_discovery",
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
/// once per run and hands the toolset builder.
fn workspace() -> Arc<Workspace> {
    let root = std::env::temp_dir().join(format!("saya-run-tools-ws-{}", std::process::id()));
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

/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Read-only and write-approving steps keep the exact
/// universe the single run-level one built; a scratch-asking step gets
/// `scratch_sql` appended — and a step that did not ask, in the same plan,
/// never sees it.
#[test]
fn every_step_s_definitions_follow_the_step_s_capabilities() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();

    // Read-only steps: the unchanged universe, byte-identical across steps.
    let built = toolsets(
        &database,
        None,
        None,
        &workspace,
        true,
        &[
            step("read the schema", Capabilities::default()),
            step("summarize", Capabilities::default()),
        ],
    );
    for step_index in 0..2 {
        assert_eq!(
            names(&built, step_index),
            OPEN_GATE,
            "step {step_index}'s universe appeared, lost, or reordered a definition"
        );
        // The write filter still keys on the approved scopes, and with none
        // approved every tool is read-shaped or gate-shaped exactly as
        // before: no definition may carry the write effect.
        for definition in &built[step_index].definitions {
            assert_ne!(
                definition.effect.local_state,
                LocalStateEffect::WriteWorkspace,
                "{} must be hidden from a run without a write-shaped scope",
                definition.name
            );
        }
    }
    // Byte-identical across steps — the serialized definitions agree, not
    // merely the names.
    assert_eq!(
        serde_json::to_string(&built[0].definitions).unwrap(),
        serde_json::to_string(&built[1].definitions).unwrap(),
        "steps with the same capabilities must carry the same universe"
    );

    let built = toolsets(
        &database,
        None,
        None,
        &workspace,
        false,
        &[step("closed gate", Capabilities::default())],
    );
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        &database,
        None,
        None,
        &workspace,
        true,
        &[step("write files", write_scopes)],
    );
    assert_eq!(
        names(&built, 0),
        [
            "schema_discovery",
            "workspace_read",
            "workspace_list",
            "glob",
            "grep",
            "workspace_write",
            "bounded_sql_query",
            "bounded_sql_query_all",
            "result_shape",
            "column_health",
            "join_check",
            "render_chart",
            "designate_answer",
        ],
        "the approved write tool must keep its place in the universe"
    );
}

/// The inverse pin's second half: a step whose capabilities ask for scratch
/// carries `scratch_sql` in its definitions — appended after the database
/// universe — while a sibling step in the same plan that did not ask never
/// sees it, in either its definitions or its executor. The run-level
/// admission alone (the run approved scratch) must not leak the tool into
/// the non-asking step.
#[tokio::test]
async fn the_scratch_tool_is_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();
    let (root, scratch) = admitted_scratch("universe");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        &database,
        Some(&scratch),
        None,
        &workspace,
        true,
        &[
            step("stage results", scratch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: scratch_sql, appended last, and no workspace_write —
    // scratch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["scratch_sql"]].concat(),
        "the scratch-asking step's universe must end with scratch_sql"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "scratch alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for scratch must never see it"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses the name even though the run admitted the scope.
    let error = built[1]
        .executor
        .execute("scratch_sql", serde_json::json!({}))
        .await
        .expect_err("the non-asking step has no scratch member behind its composite");
    assert_eq!(error, ToolError::UnsupportedTool);

    let _ = fs::remove_dir_all(root);
}

/// The slice's headline gate: `--allow scratch` alone — without
/// `workspace-write` — runs DDL through `scratch_sql`. The permit union
/// from S0 carries the write permit for a scratch-only step (pinned in the
/// harness's brief tests); here the toolset builder puts the tool in the
/// step's universe and the composite executes the statement for real.
#[tokio::test]
async fn ddl_runs_through_scratch_sql_with_only_the_scratch_scope() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();
    let (root, scratch) = admitted_scratch("ddl");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        &database,
        Some(&scratch),
        None,
        &workspace,
        true,
        &[step("stage results", scratch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["scratch_sql"]].concat(),
        "the step's universe must carry scratch_sql and nothing write-shaped besides"
    );

    let executor = &built[0].executor;
    executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "CREATE TABLE staged (a INTEGER)"}),
        )
        .await
        .expect("DDL must run through scratch_sql with scratch alone");
    executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "INSERT INTO staged VALUES (42)"}),
        )
        .await
        .expect("DML must run through scratch_sql with scratch alone");
    let result = executor
        .execute(
            "scratch_sql",
            serde_json::json!({"sql": "SELECT a FROM staged"}),
        )
        .await
        .expect("the staged row must be readable back");
    assert!(
        serde_json::to_string(&result).unwrap().contains("42"),
        "the DDL and DML must have landed in the scratch file, got: {result}"
    );

    let _ = fs::remove_dir_all(root);
}

/// The composite dispatches the four fixed harness names — and refuses them
/// exactly as the single executor always did where no member executor backs
/// them: the database tools' typed `UnsupportedTool`, fail closed.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();
    let built = toolsets(
        &database,
        None,
        None,
        &workspace,
        true,
        &[step("read only", Capabilities::default())],
    );
    for name in [
        "scratch_sql",
        "http_fetch",
        "http_download",
        "run_program",
        "wat",
    ] {
        let error = built[0]
            .executor
            .execute(name, serde_json::json!({}))
            .await
            .expect_err("nothing in this slice can run any of these names");
        assert_eq!(
            error,
            ToolError::UnsupportedTool,
            "`{name}` must be refused with today's typed error, got: {error:?}"
        );
    }
}

/// The inverse pin's first half: `fetch:` is wired, so `--allow
/// fetch:https+example.com` approves the scope — the deletion of its
/// refusal entry alone proves nothing, this does. The second half (the
/// fetch tools in the universe of the steps that asked) lives beside the
/// toolset builder's tests below.
#[test]
fn fetch_is_wired_and_still_approves() {
    use super::scopes::parse;

    let Ok(approved) = parse(&["fetch:https+example.com".to_string()]) else {
        panic!("the wired scope must approve");
    };
    let fetch = approved
        .capabilities
        .fetch
        .expect("the fetch scope must be approved");
    assert_eq!(
        fetch
            .destinations
            .iter()
            .map(|destination| (destination.scheme.as_str(), destination.host.as_str()))
            .collect::<Vec<_>>(),
        vec![("https", "example.com")],
        "the declared destinations are the approved ones"
    );
    assert!(!approved.capabilities.workspace_write);
    assert!(!approved.capabilities.scratch);
    assert!(approved.capabilities.runner.is_none());
    assert!(approved.capabilities.endpoints.as_map().is_empty());
}

/// The inverse pin's second half: the fetch tools are in the universe of
/// the steps that asked for fetch — appended after the database universe —
/// while a sibling step in the same plan that did not ask never sees them,
/// in either its definitions or its executor. The run-level admission alone
/// (the run approved fetch) must not leak the tools into the non-asking
/// step.
#[tokio::test]
async fn the_fetch_tools_are_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();
    let (_calls, fetch) = run_fetch(b"irrelevant");

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let built = toolsets(
        &database,
        None,
        Some(&fetch),
        &workspace,
        true,
        &[
            step("pull the corpus", fetch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: both fetch tools, appended last, and no workspace
    // write tool — fetch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["http_fetch", "http_download"]].concat(),
        "the fetch-asking step's universe must carry both fetch tools"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "fetch alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for fetch must never see the tools"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses both names even though the run admitted the scope.
    for name in ["http_fetch", "http_download"] {
        let error = built[1]
            .executor
            .execute(name, serde_json::json!({}))
            .await
            .expect_err("the non-asking step has no fetch member behind its composite");
        assert_eq!(
            error,
            ToolError::UnsupportedTool,
            "`{name}` must be unknown to the step that did not ask"
        );
    }
}

/// The slice's headline gate: `--allow fetch:...` alone — without
/// `workspace-write` — downloads through `http_download` for real, through
/// the same composite the episodes dispatch through. The permit union
/// carries the write permit for a fetch-only step (pinned in the harness's
/// brief tests) and the egress permit carries the loop's guard down.
#[tokio::test]
async fn a_download_runs_through_the_composite_with_only_the_fetch_scope() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace();
    let content = b"the corpus bytes".to_vec();
    let (_calls, fetch) = run_fetch(&content);

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let built = toolsets(
        &database,
        None,
        Some(&fetch),
        &workspace,
        true,
        &[step("pull the corpus", fetch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, &["http_fetch", "http_download"]].concat(),
        "the step's universe must carry both fetch tools and nothing write-shaped besides"
    );

    let result = built[0]
        .executor
        .execute(
            "http_download",
            serde_json::json!({
                "url": "https://files.example.org/corpus.bin",
                "destination": "downloads/corpus.bin"
            }),
        )
        .await
        .expect("the download must run through the composite with fetch alone");
    let outcome = result.as_object().expect("the download's metadata");
    assert_eq!(outcome["bytes"], content.len() as u64);
    assert!(
        outcome["sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64),
        "the digest the resume verifies against: {outcome:?}"
    );
    let landed = std::fs::read(std::env::temp_dir().join(format!(
        "saya-run-tools-ws-{}/downloads/corpus.bin",
        std::process::id()
    )))
    .expect("the file landed in the run workspace");
    assert_eq!(landed, content, "the bytes on disk are the served bytes");
    let _ = fs::remove_dir_all(
        std::env::temp_dir().join(format!("saya-run-tools-ws-{}", std::process::id())),
    );
}
