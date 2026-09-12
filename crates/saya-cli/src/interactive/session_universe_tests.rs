//! The session universe's composition tests: what an interactive session
//! advertises where a prompt is possible, what refuses at composition, and
//! the containment facts the red tests pin.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory, ResolvedRunnerJobs,
    ThemeChoice,
};

use super::SessionUniverse;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-session-universe-{label}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// A worktree-shaped project: `.git` present, so the walk binds its top.
fn worktree(label: &str) -> PathBuf {
    let project = temp_dir(label);
    fs::create_dir_all(project.join(".git")).unwrap();
    project
}

fn session_runtime(
    runner: Option<(Vec<String>, Option<PathBuf>)>,
) -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            profile_name: None,
            profile: None,
            ai: ResolvedAi {
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                max_output_tokens: 4096,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: 4,
                tool_calls: None,
                fetch: ResolvedFetchJobs::default(),
                interpreter: ResolvedInterpreterJobs::default(),
                runner: match runner {
                    Some((allow, program_dir)) => ResolvedRunnerJobs {
                        allow,
                        program_dir,
                        timeout_seconds: 300,
                    },
                    None => ResolvedRunnerJobs::default(),
                },
            },
            query_timeout_seconds: 5,
            output_format: OutputFormat::Text,
            output_color: ColorChoice::Auto,
            ui_theme: ThemeChoice::Auto,
            memory: ResolvedMemory {
                mode: MemoryMode::Off,
                max_contracts: 5,
                max_claims_per_contract: 12,
                max_context_bytes: 16384,
            },
            ignored_project_overrides: Vec::new(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-universe"),
        secret_values: Default::default(),
    }
}

fn compose(
    runtime: &crate::config::runtime::RuntimeConfig,
    project: &Path,
    state_dir: &Path,
) -> SessionUniverse {
    compose_result(runtime, project, state_dir).expect("composition succeeds on a plain worktree")
}

fn compose_result(
    runtime: &crate::config::runtime::RuntimeConfig,
    cwd: &Path,
    state_dir: &Path,
) -> Result<SessionUniverse, String> {
    SessionUniverse::compose(runtime, None, None, true, cwd, state_dir)
}

const SESSION_WRITE_TOOLS: [&str; 5] = [
    "workspace_write",
    "scratch_sql",
    "http_fetch",
    "http_download",
    "run_program",
];

fn advertised(universe: &SessionUniverse, mode: ApprovalPolicy, can_prompt: bool) -> Vec<String> {
    universe
        .definitions(mode, can_prompt, true, false, false)
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

// ---------------------------------------------------------------------------
// red test 2 — a read-only session sees none of the write-shaped tools
// ---------------------------------------------------------------------------

/// Under `read-only` and under `never` — and under `ask` where nothing can
/// prompt — none of the write-shaped tools are advertised: the
/// advertised-but-unusable anti-pattern (a definition the engine always
/// denies) must not return. Everything read-shaped stays.
#[test]
fn write_shaped_tools_stay_hidden_where_a_prompt_is_impossible() {
    let project = worktree("readonly");
    let state = temp_dir("readonly-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for (mode, can_prompt, label) in [
        (ApprovalPolicy::ReadOnly, true, "read-only"),
        (ApprovalPolicy::Never, true, "never"),
        (ApprovalPolicy::Ask, false, "no prompt surface"),
        (ApprovalPolicy::ReadOnly, false, "read-only, no prompt"),
    ] {
        let names = advertised(&universe, mode, can_prompt);
        for tool in SESSION_WRITE_TOOLS {
            assert!(
                !names.contains(&tool.to_string()),
                "{mode_label} must not advertise {tool}: {names:?}",
                mode_label = label,
                names = names
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// red test 3 — outside a worktree no root binds
// ---------------------------------------------------------------------------

/// Outside any worktree with no `--workspace`: the write-shaped file tools
/// and `run_program` are absent from the definitions, `scratch_sql` and
/// `http_fetch` still work, and the workspace reads stay advertised with
/// their typed no-workspace refusal at dispatch.
#[test]
fn outside_a_worktree_the_write_shaped_tools_are_hidden_and_scratch_and_fetch_work() {
    let plain = temp_dir("no-worktree-universe");
    let state = temp_dir("no-worktree-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &plain, &state)
            .expect("composition succeeds without a root");
    let names = advertised(&universe, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"workspace_write".to_string()),
        "no root, no write tool: {names:?}"
    );
    assert!(
        !names.contains(&"http_download".to_string()),
        "no root, no download destination: {names:?}"
    );
    assert!(
        !names.contains(&"run_program".to_string()),
        "no root, no runner: {names:?}"
    );
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "scratch needs no root: {names:?}"
    );
    assert!(
        names.contains(&"http_fetch".to_string()),
        "fetch needs no root: {names:?}"
    );
    assert!(universe.root().is_none());
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

/// Inside a worktree with prompts possible: the write-shaped tools are
/// advertised, each ask-gated — one engine, no scope flags. `run_program`
/// joins them only where the probe proved the host, so its advertisement is
/// pinned with the proven-host test below; its absence here (no
/// `program_dir` configured — nothing composes a runner) is the honest
/// shape, not a hidden capability.
#[test]
fn a_worktree_session_advertises_every_write_shaped_tool_ask_gated() {
    let project = worktree("ask-universe");
    let state = temp_dir("ask-universe-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, ApprovalPolicy::Ask, true);
    for tool in [
        "workspace_write",
        "scratch_sql",
        "http_fetch",
        "http_download",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "an ask session advertises {tool}: {names:?}"
        );
    }
    let definitions = universe.definitions(ApprovalPolicy::Ask, true, true, false, false);
    for tool in [
        "workspace_write",
        "scratch_sql",
        "http_fetch",
        "http_download",
    ] {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == tool)
            .expect("the write-shaped tool is advertised");
        assert!(
            definition.effect.requires_approval,
            "{tool} is ask-gated: the engine decides every call"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// red test 6 — a program dir inside the session's workspace root refuses
// ---------------------------------------------------------------------------

/// The placement guard now runs against the project: a checked-in tool
/// directory is inside the session's fs root and refuses, naming the
/// directory and the root it sits inside. `<project>` itself refuses too.
#[test]
fn a_program_dir_inside_the_session_workspace_refuses_naming_directory_and_root() {
    let project = worktree("placement");
    let state = temp_dir("placement-state");
    let tools = project.join("tools");
    fs::create_dir_all(&tools).unwrap();
    let runtime = session_runtime(Some((vec!["bench".to_string()], Some(tools.clone()))));
    let error = compose_result(&runtime, &project, &state)
        .map(|_: SessionUniverse| ())
        .expect_err("a checked-in tool directory refuses for sessions");
    assert!(
        error.contains(tools.canonicalize().unwrap().display().to_string().as_str()),
        "the refusal names the directory: {error}"
    );
    assert!(
        error.contains(
            project
                .canonicalize()
                .unwrap()
                .display()
                .to_string()
                .as_str()
        ),
        "the refusal names the root it sits inside: {error}"
    );
    assert!(
        error.contains("cannot express an exclusion"),
        "the refusal says why: {error}"
    );

    // The project root itself as program_dir: the same refusal.
    let error = compose_result(
        &session_runtime(Some((vec!["bench".to_string()], Some(project.clone())))),
        &project,
        &state,
    )
    .map(|_: SessionUniverse| ())
    .expect_err("the workspace root itself as program dir refuses");
    assert!(
        error.contains("overlaps this session's workspace root"),
        "the refusal states the containment: {error}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// red test 5 — no file tool reaches sessions/<id>/
// ---------------------------------------------------------------------------

/// The session's state directory is outside the workspace root and outside
/// every child's `fs_roots`: a sentinel planted there is unreachable by the
/// file tools — a planted symlink inside the project pointing at it is
/// refused at argument validation (symlink components), relative paths
/// cannot climb out, absolute paths are refused — and the runner
/// composition's only fs root is the workspace root itself.
#[cfg(unix)]
#[tokio::test]
async fn no_file_tool_reaches_the_session_state_dir() {
    use std::os::unix::fs::symlink;
    let project = worktree("sentinel");
    let state = temp_dir("sentinel-state");
    let sentinel = state.join("sentinel.txt");
    fs::write(&sentinel, b"STATE_SENTINEL_9f3a\n").unwrap();
    let universe = compose(&session_runtime(None), &project, &state);

    // The composition itself: the session state dir sits outside the root.
    let root = universe.root().expect("the worktree binds").to_path_buf();
    assert!(
        !state.starts_with(&root),
        "the session state dir must sit outside the workspace root"
    );

    // The file tools ride the workspace seam. A planted symlink into the
    // state dir inside the *project* is refused at every hop, and the
    // traversal shapes are refused before any filesystem contact.
    let link = project.join("state-link");
    symlink(&state, &link).unwrap();
    let workspace = universe.workspace();
    let database = Arc::new(
        crate::agent::tools::DatabaseTools::new(None, 100, true).with_workspace(workspace),
    );
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new());
    let read = executor
        .execute(
            "workspace_read",
            serde_json::json!({"path": "state-link/sentinel.txt"}),
        )
        .await
        .expect_err("a symlink into the session state dir is refused");
    let read = format!("{read:?}");
    assert!(
        read.contains("symlink"),
        "the refusal names the symlink: {read}"
    );
    for escape in [
        serde_json::json!({"path": format!("{}/sentinel.txt", state.display())}),
        serde_json::json!({"path": "../sentinel.txt"}),
        serde_json::json!({"path": "../sentinel-state/sentinel.txt"}),
    ] {
        let result = executor.execute("workspace_read", escape).await;
        assert!(
            result.is_err(),
            "no relative or absolute path reaches the state dir: {result:?}"
        );
    }
    // The sentinel was never read: it still holds only the planted bytes.
    assert_eq!(
        fs::read(&sentinel).unwrap(),
        b"STATE_SENTINEL_9f3a\n",
        "the sentinel was never rewritten"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state);
}

// ---------------------------------------------------------------------------
// red test 4 — a child's cwd is the workspace root; its writes land in the
// project where the user can see them
// ---------------------------------------------------------------------------

/// On a proven host, the session's runner composes once — one fs root (the
/// workspace), empty egress, the placement guard, the probe — and a child's
/// cwd pins to that root: a staged program writing a relative path lands at
/// the project root, where the user and `git status` see it. The program
/// directory is a real operator-staged binary tree (a copy of a system
/// binary is refused by macOS's signature enforcement under sandbox-exec,
/// so the staging contract's own discipline — real, operator-owned files —
/// is what executes).
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_session_child_runs_with_the_workspace_root_as_its_cwd() {
    let project = worktree("child-cwd");
    let state = temp_dir("child-state");
    let runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    let universe = compose(&runtime, &project, &state);
    let runner = universe
        .runner
        .as_ref()
        .expect("the probe proves this host (macOS)");
    assert_eq!(
        runner.spawn.fs_roots(),
        &[project.canonicalize().unwrap()],
        "one fs root: the workspace, and nothing else — never the session state dir"
    );

    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new());
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "touch", "args": ["out.json"]}),
        )
        .await
        .expect("the staged program runs in the project root");
    let value = serde_json::to_value(&result).unwrap();
    assert!(value.get("error").is_none(), "the child ran: {value}");
    // The relative path landed at the workspace root — the cwd proof — and
    // the file is visible in the project.
    assert!(
        project.join("out.json").exists(),
        "the child's write landed in the project, not a state dir"
    );
    // The outcome record did not create a saya directory in the project.
    assert!(
        !project.join("run_program").exists(),
        "the project gains no saya-created directories"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state);
}

// ---------------------------------------------------------------------------
// the lock, and the per-session scratch boundary
// ---------------------------------------------------------------------------

/// A second process on a live session's id refuses, naming the holder pid;
/// the pid lockfile is the same one runs use, at `sessions/<id>/lock`.
#[test]
fn a_second_acquisition_of_a_live_session_refuses() {
    let project = worktree("lock");
    let runtime = session_runtime(None);
    let first = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
    )
    .expect("the first holder acquires");
    let error = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
    )
    .map(|_: crate::interactive::session_runtime::SessionRuntime| ())
    .expect_err("a live holder refuses");
    assert!(
        error.contains("already running") && error.contains("pid"),
        "the refusal names the holder: {error}"
    );
    drop(first);
    // The released lock is reclaimable.
    crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
    )
    .expect("the lock is reclaimable after release");
    // Two different sessions on the same project both hold: no project lock.
    crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-2",
    )
    .expect("a different session on the same project acquires");
    let _ = fs::remove_dir_all(&project);
}

/// The shared-scratch trap: two concurrent sessions over the same project
/// hold separate scratch files, each at its own `sessions/<id>/`, and data
/// staged in one is invisible to the other. The per-session id is the
/// containment boundary for state.
#[tokio::test]
async fn scratch_is_per_session_not_per_project() {
    let project = worktree("scratch-boundary");
    let state_a = temp_dir("scratch-a");
    let state_b = temp_dir("scratch-b");
    let universe_a = compose(&session_runtime(None), &project, &state_a);
    let universe_b = compose(&session_runtime(None), &project, &state_b);
    let scratch_a = universe_a.scratch.as_ref().expect("scratch composed");
    let scratch_b = universe_b.scratch.as_ref().expect("scratch composed");
    scratch_a
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("session A stages a table");
    assert!(
        state_a.join("scratch.duckdb").exists() && state_b.join("scratch.duckdb").exists(),
        "each session's scratch lives at its own state dir"
    );
    let leaked = scratch_b.run("SELECT v FROM stage").await;
    assert!(
        leaked.is_err(),
        "session B must not see session A's staged table: {leaked:?}"
    );
    let _ = fs::remove_dir_all(&project);
    let _ = fs::remove_dir_all(&state_a);
    let _ = fs::remove_dir_all(&state_b);
}
