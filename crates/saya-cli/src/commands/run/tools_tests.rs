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
use saya_types::{Capabilities, InterpreterScope, RunnerScope, StepSpec};

use crate::agent::tools::DatabaseTools;

use super::runner::RunRunner;
use super::tools::{RunFetch, ToolsetInputs, toolsets};

/// The run's cancellation token, as the composition root passes it.
fn cancellation() -> CancellationToken {
    CancellationToken::default()
}

/// The names each step's definitions must carry, in order, for a read-only
/// run with the privacy gate open: the database and workspace-read set, no
/// write tool, and no contract tools (a run passes no state store).
/// `workspace_write` sits between `grep` and the sql tools when approved;
/// the scope-asking tails append after it.
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
/// The tools each scope-asking step gets appended after the database
/// universe, in construction order: scratch, then fetch's pair, then the
/// runner's `run_program`.
const SCRATCH_TAIL: &[&str] = &["scratch_sql"];
const FETCH_TAIL: &[&str] = &["http_fetch", "http_download"];
const RUNNER_TAIL: &[&str] = &["run_program"];

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

/// The expected names are stated here, not derived from the builder's own
/// construction, so a drift is a diff in this test rather than a silent
/// universe change. Read-only and write-approving steps keep the exact
/// universe the single run-level one built; a scratch-asking step gets
/// `scratch_sql` appended — and a step that did not ask, in the same plan,
/// never sees it.
#[test]
fn every_step_s_definitions_follow_the_step_s_capabilities() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("defs");

    // Read-only steps: the unchanged universe, byte-identical across steps.
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
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
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: false,
            cancellation: &cancellation(),
        },
        &[step("closed gate", Capabilities::default())],
    );
    assert_eq!(names(&built, 0), CLOSED_GATE);

    let mut write_scopes = Capabilities::default();
    write_scopes.workspace_write = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
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
    let workspace = workspace("scratch-universe");
    let (root, scratch) = admitted_scratch("universe");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: Some(&scratch),
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("stage results", scratch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: scratch_sql, appended last, and no workspace_write —
    // scratch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, SCRATCH_TAIL].concat(),
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
    let workspace = workspace("scratch-ddl");
    let (root, scratch) = admitted_scratch("ddl");

    let mut scratch_caps = Capabilities::default();
    scratch_caps.scratch = true;
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: Some(&scratch),
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("stage results", scratch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, SCRATCH_TAIL].concat(),
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

/// The composite dispatches the four fixed harness names — and refuses the
/// ones no member backs exactly as the single executor always did: the
/// database tools' typed `UnsupportedTool`, fail closed. `run_program` is
/// not on this list any more: its member is wired (S3), and its narrowing
/// is pinned in the runner tests below.
#[tokio::test]
async fn the_harness_names_fall_through_to_the_same_typed_refusal() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("fall-through");
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("read only", Capabilities::default())],
    );
    for name in ["scratch_sql", "http_fetch", "http_download", "wat"] {
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

    let Ok(approved) = parse(
        &["fetch:https+example.com".to_string()],
        super::scopes::Surface::Run,
    ) else {
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
    let workspace = workspace("fetch-universe");
    let (_calls, fetch) = run_fetch(b"irrelevant");

    let mut fetch_caps = Capabilities::default();
    fetch_caps.fetch = Some(
        saya_types::FetchScope::new(vec![
            saya_types::Destination::new("https", "files.example.org").unwrap(),
        ])
        .expect("shaped"),
    );
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: Some(&fetch),
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("pull the corpus", fetch_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: both fetch tools, appended last, and no workspace
    // write tool — fetch alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, FETCH_TAIL].concat(),
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
    let workspace = workspace("download");
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
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: Some(&fetch),
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("pull the corpus", fetch_caps.clone())],
    );

    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, FETCH_TAIL].concat(),
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
        "saya-run-tools-ws-download-{}/downloads/corpus.bin",
        std::process::id()
    )))
    .expect("the file landed in the run workspace");
    assert_eq!(landed, content, "the bytes on disk are the served bytes");
    let _ = fs::remove_dir_all(
        std::env::temp_dir().join(format!("saya-run-tools-ws-download-{}", std::process::id())),
    );
}

/// A step whose capabilities ask for the runner gets nothing when the run's
/// runner wiring is absent — an unproven host, or a run that approved the
/// scope on a host the probe refused. The tool has no definition and the
/// composite refuses the name as an unknown tool: the capability is absent,
/// never degraded. (The definitions half needs no proven spawn.)
#[tokio::test]
async fn run_program_is_absent_where_no_runner_wiring_exists() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-absent");
    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: None,
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("run the harness", runner_caps.clone())],
    );
    assert_eq!(
        names(&built, 0),
        OPEN_GATE,
        "without wiring the runner tool must be absent from the universe"
    );
    let error = built[0]
        .executor
        .execute("run_program", serde_json::json!({}))
        .await
        .expect_err("no wiring means no member behind the composite");
    assert_eq!(error, ToolError::UnsupportedTool);
}

/// A proven spawn for the toolset tests: the startup probe decides per
/// host, and only the proven arm of `prepare` constructs a `RunnerSpawn`,
/// so this helper — and every test below that uses it — is macOS-only,
/// exactly like the escape battery it mirrors. The program directory sits
/// beside the roots, the staging contract.
#[cfg(target_os = "macos")]
fn proven_runner(tag: &str) -> (PathBuf, RunRunner) {
    use std::time::Duration;

    use saya_harness::runner::sandbox::RunSandbox;

    let run_root = std::env::temp_dir().join(format!(
        "saya-run-tools-runner-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&run_root);
    fs::create_dir_all(run_root.join("workspace")).unwrap();
    fs::create_dir_all(run_root.join("state")).unwrap();
    let programs = std::env::temp_dir().join(format!(
        "saya-run-tools-runner-programs-{tag}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&programs);
    fs::create_dir_all(&programs).unwrap();
    let sandbox = RunSandbox::new(
        [
            fs::canonicalize(run_root.join("workspace")).unwrap(),
            fs::canonicalize(run_root.join("state")).unwrap(),
        ],
        Vec::<(String, u16)>::new(),
    )
    .expect("the run's roots construct");
    let provision = sandbox.prepare(&programs).expect("preparation must run");
    assert!(
        provision.report().proves_runner(),
        "the probe must prove this host for the wiring to exist:\n{}",
        provision.report().render()
    );
    (
        run_root,
        RunRunner {
            spawn: provision.spawn().expect("proven").clone(),
            timeout: Duration::from_secs(300),
        },
    )
}

/// The inverse pin's second half: the runner tool is in the universe of —
/// and only of — the steps that asked for it. The run-level admission alone
/// (the run approved the scope and the probe proved the host) must not leak
/// `run_program` into a non-asking step, in either its definitions or its
/// executor.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_runner_tool_is_in_the_universe_of_the_steps_that_asked_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-universe");
    let (_run_root, runner) = proven_runner("universe");

    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("run the harness", runner_caps.clone()),
            step("read only", Capabilities::default()),
        ],
    );

    // The asking step: run_program appended last, no workspace write tool —
    // the runner alone is not the workspace-write scope.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, RUNNER_TAIL].concat(),
        "the runner-asking step's universe must end with run_program"
    );
    assert!(
        !names(&built, 0).contains(&"workspace_write".to_string()),
        "the runner alone must not carry the workspace-write tool"
    );
    // The non-asking step: byte-identical to the unchanged universe.
    assert_eq!(
        names(&built, 1),
        OPEN_GATE,
        "a step that did not ask for the runner must never see it"
    );

    // And the executor narrows with the definitions: the non-asking step's
    // composite refuses the name even though the run wired the scope.
    let error = built[1]
        .executor
        .execute("run_program", serde_json::json!({}))
        .await
        .expect_err("the non-asking step has no runner member behind its composite");
    assert_eq!(error, ToolError::UnsupportedTool);
}

/// The slice's headline gate on the approval surface: a step that asked only
/// for one program rejects a different allowlisted one — the tool carries
/// the *step's* narrowed `RunnerScope`, never the run's union, so the
/// enforcement matches the thing the approval view showed.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_step_s_narrowed_allowlist_rejects_a_program_it_did_not_ask_for() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("runner-narrowed");
    let (_run_root, runner) = proven_runner("narrowed");

    // The run approved both programs; this step asked only for `bench`.
    let mut step_caps = Capabilities::default();
    step_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[step("measure with the harness", step_caps)],
    );
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, RUNNER_TAIL].concat(),
        "the step's universe must carry run_program"
    );
    let error = built[0]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "ripgrep", "args": []}),
        )
        .await
        .expect_err("a program outside the step's narrowed scope must refuse");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("ripgrep") && message.contains("allowlist"),
        "the refusal must name the program and the allowlist: {message}"
    );
}

/// The per-step trap (the interpreter approval's design §6): the test a
/// run-wide boolean passes while being wrong. One step of a run holds the
/// interpreter scope, another asks only for the runner; the second step's
/// tool must not contain the interpreter. A run-wide
/// `allow_interpreters` threaded beside the scope machinery makes every step
/// interpreter-capable and cannot pass this — the grant is per-step, like
/// `scratch` and `fetch` before it.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn the_interpreter_grant_does_not_leak_into_the_steps_that_did_not_ask_for_it() {
    let database = Arc::new(DatabaseTools::new(None, 100, true));
    let workspace = workspace("interpreter-trap");
    let (_run_root, runner) = proven_runner("interpreter-trap");

    // Step 1 asks for the interpreter; step 2 asks only for a runner
    // program. The run approved the interpreter scope — a run-wide boolean
    // would open the door on both steps.
    let mut interpreter_caps = Capabilities::default();
    interpreter_caps.interpreter =
        Some(InterpreterScope::new(vec!["python3".to_owned()]).expect("refused name"));
    let mut runner_caps = Capabilities::default();
    runner_caps.runner = Some(RunnerScope::new(vec!["bench".to_owned()]).expect("shaped"));
    let built = toolsets(
        ToolsetInputs {
            database: &database,
            scratch: None,
            fetch: None,
            runner: Some(&runner),
            workspace: &workspace,
            allow_query_data: true,
            cancellation: &cancellation(),
        },
        &[
            step("score the fetched benchmark", interpreter_caps),
            step("measure with the harness", runner_caps),
        ],
    );

    // Both steps carry `run_program` — the two families route through one
    // tool, each step's doors narrowed to what that step itself asked for.
    assert_eq!(
        names(&built, 0),
        [OPEN_GATE, RUNNER_TAIL].concat(),
        "the interpreter-asking step's universe must carry run_program"
    );
    assert_eq!(
        names(&built, 1),
        [OPEN_GATE, RUNNER_TAIL].concat(),
        "the runner-asking step's universe must carry run_program"
    );

    // The asking step's interpreter door is open: a python3 call is not
    // refused by name — it passes the gate and dies on the staged-file
    // battery (nothing is staged here), the refusal an approved interpreter
    // reaches when its bytes are absent. The run-wide boolean passes this
    // half trivially; the next assertion is the one it cannot.
    let error = built[0]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": []}),
        )
        .await
        .expect_err("nothing is staged, so the door's own battery refuses");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("no allowlisted program file exists"),
        "the asking step's python3 must pass the name gate and reach the file \
         battery, not be refused by name: {message}"
    );

    // The trap: the second step never asked for the interpreter, so its
    // `run_program` must keep the runner's byte-identical name refusal —
    // the run's approval alone grants nothing to a step that did not ask.
    let error = built[1]
        .executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": []}),
        )
        .await
        .expect_err("the step that did not ask must refuse the interpreter");
    let ToolError::Runner(message) = &error else {
        panic!("the refusal must be the runner's typed error, got: {error:?}");
    };
    assert!(
        message.contains("shells and interpreters are refused by name"),
        "the non-asking step's refusal must be the runner's byte-identical \
         name refusal: {message}"
    );
}
