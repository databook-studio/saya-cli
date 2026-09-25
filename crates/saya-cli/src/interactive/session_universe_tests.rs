//! The session universe's composition tests: what an interactive session
//! advertises where a prompt is possible, what refuses at composition, and
//! the containment facts the red tests pin.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{AgentMode, ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
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
                max_output_tokens_is_default: true,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            max_iterations: 4,
            candidates: 1,
            jobs: ResolvedJobs {
                wall_clock_seconds: None,
                tokens_per_endpoint: Default::default(),
                turns: Some(4),
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
            host_commands: ResolvedHostCommands::default(),
            session_deny: Default::default(),
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

const SESSION_WRITE_TOOLS: [&str; 6] = [
    "workspace_write",
    "scratch_sql",
    "scratch_import",
    "http_fetch",
    "http_download",
    "run_program",
];

fn advertised(
    universe: &SessionUniverse,
    agent_mode: AgentMode,
    mode: ApprovalPolicy,
    can_obtain_approval: bool,
) -> Vec<String> {
    universe
        .definitions(agent_mode, mode, can_obtain_approval, true, false, false)
        .into_iter()
        .map(|definition| definition.name)
        .collect()
}

/// The lane's advertisement follows the mode rule — read-only and never
/// sessions never see the tool — and a composed lane advertises under ask
/// (with a prompt) and under bypass. The universe helper composes the lane
/// here through the stated-launch helper below.
#[cfg(not(windows))]
#[test]
fn the_lane_s_advertisement_follows_the_mode_rule() {
    let project = worktree("host-advertise");
    let state = temp_dir("host-advertise-state");
    let runtime = session_runtime(None);
    let launch = crate::interactive::session_host::HostLaunch::for_tests_stated(&runtime);
    let universe = SessionUniverse::compose_with_launch(
        &runtime,
        None,
        None,
        true,
        &project,
        &state,
        Some(&launch),
    )
    .expect("composition succeeds on a plain worktree");
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::ReadOnly, true, "read-only"),
        (ApprovalPolicy::Never, true, "never"),
        (ApprovalPolicy::Ask, false, "no prompt surface"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert!(
            !names.contains(&"run_command".to_string()),
            "{label} never sees the tool: {names:?}"
        );
    }
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        ask_names.contains(&"run_command".to_string()),
        "a composed lane advertises under ask with a prompt: {ask_names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        bypass_names.contains(&"run_command".to_string()),
        "a composed lane advertises under bypass: {bypass_names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No-PATH property 1 — a session without PATH still starts: no host
/// lane, and everything else works (SQL, schema, workspace file tools,
/// approvals).
#[test]
fn a_session_without_path_still_starts() {
    let project = worktree("no-path-starts");
    let state = temp_dir("no-path-starts-state");
    let runtime = session_runtime(None);
    // The full composer with the PATH seam explicit: `None` is the no-PATH
    // environment (no process-environment mutation — the seam carries it).
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: None,
            scratch: None,
        },
    )
    .expect("a session without PATH still starts");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "no PATH: the lane composes nothing"
    );
    assert!(
        universe.root().is_some(),
        "the root still binds without PATH"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("no PATH says so at startup");
    if cfg!(windows) {
        assert!(notice.contains("unavailable on Windows"), "{notice:?}");
    } else {
        assert!(
            notice.contains("No PATH is set"),
            "the notice states the fact: {notice:?}"
        );
    }
    // Everything else works: the session's own surface is intact — SQL,
    // schema reads, workspace file tools, and approvals.
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "workspace_write",
        "http_download",
    ] {
        assert!(
            ask_names.contains(&tool.to_string()),
            "no PATH keeps {tool}: {ask_names:?}"
        );
    }
    assert!(
        !ask_names.contains(&"run_command".to_string()),
        "only the lane is gone: {ask_names:?}"
    );
    assert!(
        universe.workspace().is_some(),
        "workspace file tools still compose without PATH"
    );
    assert!(
        universe.approval_facts(&runtime).host.is_none(),
        "the host lane contributes no approval facts without PATH"
    );
    // Deny still composes: the lane-blind list gates every session's own
    // doors even with the lane off.
    let launch =
        crate::interactive::session_host::HostLaunch::from_deny_for_tests(vec!["curl".to_owned()]);
    let denied = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: Some(&launch),
            path: None,
            scratch: universe.scratch(),
        },
    )
    .expect("deny composes without PATH too");
    assert!(
        denied.deny_programs().contains(&"curl".to_string()),
        "the deny list rides the no-PATH session: {:?}",
        denied.deny_programs()
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Windows keeps the session available while the Unix-only host runner is
/// deliberately absent. The unavailable lane must contribute neither a tool
/// nor approval facts, including under bypass where it would otherwise run.
#[cfg(windows)]
#[test]
fn a_windows_session_starts_with_the_host_lane_off() {
    let project = worktree("windows-host-off");
    let state = temp_dir("windows-host-off-state");
    let runtime = session_runtime(None);
    let universe = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: Some("C:\\Windows\\System32".to_owned()),
            scratch: None,
        },
    )
    .expect("Windows starts the session with host commands unavailable");

    assert!(universe.root().is_some(), "the workspace still binds");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "host lane is off"
    );
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::Ask, true, "ask"),
        (ApprovalPolicy::Bypass, false, "bypass"),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
        assert!(
            !names.contains(&"run_command".to_string()),
            "Windows {label} does not advertise an unavailable tool: {names:?}"
        );
    }
    assert!(
        universe.approval_facts(&runtime).host.is_none(),
        "an unavailable host lane has no approval facts"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("startup names the unavailable lane");
    assert!(notice.contains("unavailable on Windows"), "{notice:?}");
    assert!(notice.contains("run_command is unavailable"), "{notice:?}");
    assert!(matches!(
        crate::interactive::session_host::compose_host(
            &crate::interactive::session_host::HostLaunch::unstated(&runtime),
            Some(&project),
            "C:\\Windows\\System32".to_owned(),
        ),
        Err(error) if error.contains("unavailable on Windows")
    ));
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No-PATH property 2 — the missing-PATH notice names the fact, the
/// consequence, and the remedy, in the unbound-workspace register.
#[cfg(not(windows))]
#[test]
fn the_missing_path_notice_names_fact_consequence_and_remedy() {
    use crate::interactive::session_host::NO_PATH_NOTICE;
    assert!(
        NO_PATH_NOTICE.contains("No PATH is set"),
        "the notice states the fact: {NO_PATH_NOTICE:?}"
    );
    assert!(
        NO_PATH_NOTICE.contains("run_command is unavailable"),
        "the notice names the consequence: {NO_PATH_NOTICE:?}"
    );
    assert!(
        NO_PATH_NOTICE.contains("set PATH"),
        "the notice names the remedy: {NO_PATH_NOTICE:?}"
    );
}

/// G2 property 2 — no workspace root, no lane. The tool stays hidden,
/// not advertised. Moved from `no_workspace_no_lane_even_with_the_flag`
/// (reason: there is no flag to state any more; the pin is the no-root
/// shape, unstated included).
#[test]
fn no_root_still_no_lane() {
    let plain = temp_dir("host-no-worktree");
    let state = temp_dir("host-no-worktree-state");
    let runtime = session_runtime(None);
    let launch = crate::interactive::session_host::HostLaunch::for_tests_stated(&runtime);
    let universe = SessionUniverse::compose_with_launch(
        &runtime,
        None,
        None,
        true,
        &plain,
        &state,
        Some(&launch),
    )
    .expect("composition succeeds without a root");
    assert!(
        universe.host_composed_for_tests().is_none(),
        "no workspace root: the lane does not compose"
    );
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"run_command".to_string()),
        "hidden, not advertised: {names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        !bypass_names.contains(&"run_command".to_string()),
        "hidden under bypass too, not advertised: {bypass_names:?}"
    );
    // The unstated shape composes the same way while retaining the live
    // scratch connection from the first universe.
    let unstated = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &plain,
            state_dir: &state,
            launch: None,
            path: std::env::var_os("PATH").map(|value| value.to_string_lossy().into_owned()),
            scratch: universe.scratch(),
        },
    )
    .expect("the unstated unbound session composes");
    assert!(
        unstated.host_composed_for_tests().is_none(),
        "unstated: no lane either"
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// red test 2 — a read-only session sees none of the write-shaped tools
// ---------------------------------------------------------------------------

/// Under `read-only` and under `never` — and under `ask` where no approval
/// surface exists at all — none of the write-shaped tools are advertised:
/// the advertised-but-unusable anti-pattern (a definition the engine always
/// denies) must not return. Everything read-shaped stays.
///
/// The flag is the approval-surface flag, not the stdin flag: a surface
/// that can obtain approvals (the line REPL, the TUI's modal) advertises
/// under `ask`; a surface that can obtain none still hides. `read-only`
/// and `never` hide on every surface, approval surface or not.
#[test]
fn write_shaped_tools_stay_hidden_where_a_prompt_is_impossible() {
    let project = worktree("readonly");
    let state = temp_dir("readonly-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for (mode, can_obtain_approval, label) in [
        (ApprovalPolicy::ReadOnly, true, "read-only"),
        (ApprovalPolicy::Never, true, "never"),
        (ApprovalPolicy::Ask, false, "no approval surface"),
        (
            ApprovalPolicy::ReadOnly,
            false,
            "read-only, no approval surface",
        ),
    ] {
        let names = advertised(&universe, AgentMode::Build, mode, can_obtain_approval);
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

/// The anti-silent-degradation pin (DESIGN §3): under `bypass` the session
/// advertises every write-shaped tool with no prompt surface at all — that is
/// the mode's meaning. Advertised-but-unusable cannot return under bypass,
/// because the advertised tools *are* usable: the engine resolves `Allow`.
/// A variant added without restating the advertisement rule would degrade
/// silently into "read-only with auto-approved SQL", hiding every
/// write-shaped tool while claiming everything runs.
#[test]
fn bypass_advertises_the_write_shaped_tools_without_a_prompt_surface() {
    let project = worktree("bypass-advertise");
    let state = temp_dir("bypass-advertise-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for can_obtain_approval in [true, false] {
        let names = advertised(
            &universe,
            AgentMode::Build,
            ApprovalPolicy::Bypass,
            can_obtain_approval,
        );
        for tool in [
            "workspace_write",
            "scratch_sql",
            "http_fetch",
            "http_download",
        ] {
            assert!(
                names.contains(&tool.to_string()),
                "bypass advertises {tool} whether or not a prompt surface exists \
                 (can_obtain_approval={can_obtain_approval}): {names:?}"
            );
        }
        let definitions = universe.definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            can_obtain_approval,
            true,
            false,
            false,
        );
        let write = definitions
            .iter()
            .find(|definition| definition.name == "workspace_write")
            .expect("workspace_write is advertised under bypass");
        assert!(
            write.effect.requires_approval,
            "the definition still declares its approval shape honestly: the engine resolves it"
        );
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
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
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
        !names.contains(&"scratch_import".to_string()),
        "no root, no contained import: {names:?}"
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
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "workspace_write",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "an ask session advertises {tool}: {names:?}"
        );
    }
    let definitions = universe.definitions(
        saya_agent::AgentMode::Build,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    for tool in [
        "workspace_write",
        "scratch_sql",
        "scratch_import",
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

#[tokio::test]
async fn advertised_scratch_import_routes_through_the_session_executor() {
    let project = worktree("scratch-import-executor");
    let state = temp_dir("scratch-import-executor-state");
    std::fs::write(project.join("data.csv"), b"name\nAda\n").unwrap();
    let universe = compose(&session_runtime(None), &project, &state);
    assert!(
        advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
            .contains(&"scratch_import".to_owned())
    );
    let database = Arc::new(
        crate::agent::tools::DatabaseTools::new(None, 100, true)
            .with_workspace(universe.workspace()),
    );
    let result = universe
        .executor(database, &CancellationToken::new(), false)
        .execute(
            "scratch_import",
            serde_json::json!({"path":"data.csv","table":"people"}),
        )
        .await
        .expect("advertised import routes to scratch");
    assert_eq!(result["rows_imported"], 1);
    let _ = (fs::remove_dir_all(project), fs::remove_dir_all(state));
}

// ---------------------------------------------------------------------------
// red test 6 — a program dir inside the session's workspace root refuses
// ---------------------------------------------------------------------------

/// The placement guard now runs against the project: a checked-in tool
/// directory is inside the session's fs root and refuses, naming the
/// directory and the root it sits inside. `<project>` itself refuses too.
#[cfg(not(windows))]
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
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
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
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
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
// the interpreter door — the seam fix (DESIGN §8.1)
// ---------------------------------------------------------------------------

/// The seam-fix regression (DESIGN §8.1): under `ask`, a granted
/// `interpreter:<program>` token resolves the engine's `Allow` — and the
/// granted interpreter actually reaches execution. The ask offers the token
/// (`grant_token.rs`), the engine honours the grant — but the composition
/// must carry the door too, or the user approved a capability the
/// composition never constructed: a lying approval. The family refusal
/// (an unstaged name) is test 8's pin.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn a_granted_interpreter_actually_runs_under_ask() {
    let project = worktree("interpreter-grant");
    let state = temp_dir("interpreter-grant-state");
    // The trusted config stages the interpreter universe: [jobs.interpreter]
    // allow carries python3, staged beside the runner programs.
    let mut runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    runtime.resolved.jobs.interpreter.allow = vec!["python3".to_string()];
    let universe = compose(&runtime, &project, &state);
    assert!(
        universe.runner.is_some(),
        "the probe proves this host (macOS): the runner composes"
    );

    // The engine half: under ask, the granted token pre-answers the call.
    let definitions = universe.definitions(
        saya_agent::AgentMode::Build,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    let run_program = definitions
        .iter()
        .find(|definition| definition.name == "run_program")
        .expect("a proven runner advertises run_program");
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    policy.grants().grant("interpreter:python3");
    assert_eq!(
        policy.resolve(&run_program.effect, Some("interpreter:python3")),
        saya_agent::ApprovalDecision::Allow,
        "the granted interpreter token pre-answers the ask"
    );

    // The door half: the granted interpreter call must reach the interpreter
    // door and be admitted — the child's own outcome (whatever it is) is the
    // report, never the family refusal. Today the composition carries no
    // interpreter scope, so the call falls to the runner door and refuses
    // with INTERPRETER_REFUSAL: the user approved a capability the
    // composition never constructed.
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "python3", "args": ["-c", "print(41 + 1)"]}),
        )
        .await;
    let value = match result {
        Ok(value) => value,
        Err(error) => panic!(
            "the granted interpreter call was refused: {error:?} — the ask approved a \
             capability the composition never constructed"
        ),
    };
    let outcome = serde_json::to_value(&value).unwrap();
    assert!(
        outcome.get("error").is_none(),
        "the call was admitted: the child's own outcome is the report, not a refusal: {outcome}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The interpreter door's universe is the trusted config's staged
/// `[jobs.interpreter] allow` — built at composition, mode-independently:
/// capability in the composition, consent in the approval engine. With
/// nothing staged, the door does not exist at all (`None`), so an interpreter
/// call keeps the runner door's family refusal.
#[cfg(target_os = "macos")]
#[test]
fn the_session_s_interpreter_door_is_the_staged_config_universe() {
    let project = worktree("interpreter-door");
    let state = temp_dir("interpreter-door-state");
    let mut staged = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    staged.resolved.jobs.interpreter.allow = vec!["python3".to_string(), "perl".to_string()];
    let universe = compose(&staged, &project, &state);
    let runner = universe
        .runner
        .as_ref()
        .expect("the probe proves this host (macOS)");
    let door = runner
        .interpreters
        .as_ref()
        .expect("staged interpreters open the door");
    assert_eq!(door.programs, vec!["python3", "perl"]);

    let unstaged = compose(
        &session_runtime(Some((
            vec!["touch".to_string()],
            Some(PathBuf::from("/usr/bin")),
        ))),
        &project,
        &state,
    );
    let doorless = unstaged
        .runner
        .expect("the runner composes regardless of interpreters");
    assert!(
        doorless.interpreters.is_none(),
        "nothing staged in [jobs.interpreter] allow: the door does not exist"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// An unstaged interpreter keeps the byte-identical family refusal even
/// while the interpreter door is open: the door exists only for names the
/// runner refuses *and* the staged scope carries, so `bash` — refused, not
/// staged — falls back to the runner's own words. (The bytes are pinned on
/// the run surface, `runner/mod.rs`; this pins the session side of the same
/// constant.)
#[cfg(target_os = "macos")]
#[tokio::test]
async fn an_unstaged_interpreter_keeps_the_byte_identical_family_refusal() {
    const FAMILY_REFUSAL: &str = "shells and interpreters are refused by name: \
         an interpreter can spawn arbitrary children with arbitrary argv and would void \
         the typed-argv contract from inside the allowlist";
    let project = worktree("interpreter-family");
    let state = temp_dir("interpreter-family-state");
    let mut runtime = session_runtime(Some((
        vec!["touch".to_string()],
        Some(PathBuf::from("/usr/bin")),
    )));
    runtime.resolved.jobs.interpreter.allow = vec!["python3".to_string()];
    let universe = compose(&runtime, &project, &state);
    assert!(universe.runner.is_some(), "the probe proves this host");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(None, 100, true));
    let executor = universe.executor(Arc::clone(&database), &CancellationToken::new(), false);
    let result = executor
        .execute(
            "run_program",
            serde_json::json!({"program": "bash", "args": ["-c", "echo hi"]}),
        )
        .await
        .expect_err("bash is not staged: the family refusal stands");
    assert!(
        result.to_string().contains(FAMILY_REFUSAL),
        "the unstaged interpreter keeps the byte-identical family refusal: {result}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The probe gates every door, and its refusal is *said* (DESIGN §2, test
/// 15): a config that declared a program dir and an allow can compose no
/// runner for exactly one reason — the probe did not prove this host — and
/// the absence lands on the notice seam, referenced by the bypass activation
/// line. On a proven host the positive control holds: the runner composes,
/// the notice stays silent, and `run_program` is advertised.
#[test]
fn bypass_composes_no_runner_where_the_probe_refuses_and_says_so() {
    use super::super::session_runner::PROBE_REFUSED_NOTICE;
    let project = worktree("probe-refusal");
    let state = temp_dir("probe-refusal-state");
    let tools = temp_dir("probe-refusal-tools");
    let runtime = session_runtime(Some((vec!["bench".to_string()], Some(tools))));
    let universe = compose(&runtime, &project, &state);
    match universe.runner.as_ref() {
        Some(_) => {
            // Positive control (a proven host): the runner is there, nothing
            // is said, and bypass advertises the tool.
            assert!(
                !universe.probe_refused && universe.notice.is_none(),
                "a proven probe is silent: {:?}",
                universe.notice
            );
            assert!(
                advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
                    .contains(&"run_program".to_string()),
                "bypass advertises the proven runner with no prompt surface"
            );
        }
        None => {
            // The config declared program_dir and allow, and the placement
            // guard passed (composition did not Err) — so the only possible
            // cause of an absent runner is the refused probe. It must be
            // said, and run_program must be absent from every mode's list.
            let notice = universe
                .notice
                .as_deref()
                .expect("a refused probe is said, never silent");
            if cfg!(windows) {
                assert!(notice.contains(PROBE_REFUSED_NOTICE), "{notice:?}");
                assert!(
                    notice.contains("Host commands are unavailable on Windows"),
                    "{notice:?}"
                );
            } else {
                assert_eq!(notice, PROBE_REFUSED_NOTICE);
            }
            assert!(universe.probe_refused, "the activation line's fact rides");
            assert!(
                !advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false)
                    .contains(&"run_program".to_string()),
                "no runner, no run_program advertisement — under bypass like any mode"
            );
        }
    }
    // The words the refusal emits are the design's line, whatever host this
    // test runs on.
    assert!(
        PROBE_REFUSED_NOTICE.contains("run_program is unavailable")
            && PROBE_REFUSED_NOTICE.contains("the sandbox probe did not prove this host"),
        "the notice names the probe: {PROBE_REFUSED_NOTICE}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// the SQL safety layer is untouchable under bypass
// ---------------------------------------------------------------------------

/// The SQL safety layer is not an approval question, so bypass does not move
/// it (DESIGN §4, test 14): a write statement through the query tools still
/// refuses under a bypass policy — and the refusal is the safety layer's own
/// words, not the approval engine's, because the engine resolved `Allow` and
/// handed the call to the tool. A real DuckDB connector backs the registry,
/// so the statement meets the actual `prepare_*` gate.
#[tokio::test]
async fn bypass_leaves_the_sql_safety_layer_untouched() {
    use saya_connectors::{ConnectorOptions, DuckDbConnector};
    let project = worktree("safety-layer");
    let state = temp_dir("safety-layer-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let connector = DuckDbConnector::open(":memory:", false, ConnectorOptions::default())
        .await
        .expect("an in-memory duckdb opens");
    let database = Arc::new(crate::agent::tools::DatabaseTools::new(
        Some(Box::new(connector)),
        100,
        true,
    ));
    // The engine half: under bypass the write-shaped SQL call is *allowed* —
    // the refusal cannot come from approval.
    let sql = universe
        .definitions(
            saya_agent::AgentMode::Build,
            ApprovalPolicy::Bypass,
            false,
            true,
            false,
            false,
        )
        .iter()
        .find(|definition| definition.name == "bounded_sql_query")
        .expect("the read-shaped SQL tools are always advertised")
        .clone();
    assert_eq!(
        saya_agent::SessionPolicy::new(ApprovalPolicy::Bypass).resolve(&sql.effect, None),
        saya_agent::ApprovalDecision::Allow,
        "bypass allows the SQL call: the safety layer is the next line of defence, not approval"
    );
    let executor = universe.executor(database, &CancellationToken::new(), false);
    let refused = executor
        .execute(
            "bounded_sql_query",
            serde_json::json!({"sql": "DROP TABLE users"}),
        )
        .await
        .expect_err("a write statement still refuses under bypass");
    let refusal = format!("{refused:?}");
    assert!(
        refusal.contains("read-only safety policy")
            || refusal.contains("not parseable as one read-only statement"),
        "the refusal is the safety layer's own: {refusal}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
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
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session_paths::default_session_dir(),
    )
    .expect("the first holder acquires");
    let error = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-1",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session_paths::default_session_dir(),
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
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session_paths::default_session_dir(),
    )
    .expect("the lock is reclaimable after release");
    // Two different sessions on the same project both hold: no project lock.
    crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lock-session-2",
        saya_agent::ApprovalPolicy::Ask,
        &crate::interactive::session_paths::default_session_dir(),
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

/// Recomposition keeps the first universe alive for active TUI snapshots,
/// while both universes share the one DuckDB connection for this session.
#[tokio::test]
async fn recomposing_a_session_reuses_its_live_scratch_connection() {
    let project = worktree("scratch-recompose");
    let state = temp_dir("scratch-recompose-state");
    let runtime = session_runtime(None);
    let first = compose(&runtime, &project, &state);
    let scratch = first.scratch.clone().expect("scratch composed");
    first
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("the first universe stages a table");

    let second = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: true,
            cwd: &project,
            state_dir: &state,
            launch: None,
            path: Some("test-path".to_owned()),
            scratch: Some(scratch),
        },
    )
    .expect("recomposition shares the live scratch database");

    let second_scratch = second.scratch.as_ref().expect("scratch composed");
    let rows = second_scratch
        .run("SELECT v FROM stage")
        .await
        .expect("the second universe reads the first universe's stage");
    assert_eq!(rows.rows.len(), 1, "the staged row remains available");
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A later unbound universe retains scratch state but must clear the prior
/// workspace binding, hiding the CSV importer from its tool surface.
#[tokio::test]
async fn recomposing_unbound_clears_scratch_import_access() {
    let project = worktree("scratch-recompose-unbound");
    let plain = temp_dir("scratch-recompose-plain");
    let state = temp_dir("scratch-recompose-unbound-state");
    let runtime = session_runtime(None);
    let first = compose(&runtime, &project, &state);
    first
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("CREATE TABLE stage AS SELECT 42 AS v")
        .await
        .expect("the bound universe stages a table");

    let second = SessionUniverse::compose_with_launch_and_path(
        crate::interactive::session_universe::SessionComposition {
            runtime: &runtime,
            explicit: None,
            pinned_root: None,
            walk_when_unpinned: false,
            cwd: &plain,
            state_dir: &state,
            launch: None,
            path: Some("test-path".to_owned()),
            scratch: first.scratch(),
        },
    )
    .expect("the unbound recomposition retains scratch");

    let names = advertised(&second, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        !names.contains(&"scratch_import".to_owned()),
        "an unbound universe hides scratch imports: {names:?}"
    );
    second
        .scratch
        .as_ref()
        .expect("scratch composed")
        .execute(
            "scratch_import",
            serde_json::json!({"path": "data.csv", "table": "stage"}),
        )
        .await
        .expect_err("an unbound universe refuses scratch imports");
    let rows = second
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("SELECT v FROM stage")
        .await
        .expect("the shared scratch database retains staged data");
    assert_eq!(rows.rows.len(), 1, "the staged row remains available");
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&state),
    );
}

/// The resume premise the session's own tool description must state (U7):
/// the scratch database lives in the session's state directory and nothing
/// deletes it, so a resumed session — which reuses the record's id and
/// re-enters the same `sessions/<id>/` — re-opens it with its staged tables
/// intact. Deleting it at session end would silently destroy exactly this
/// work; the engine state is durable, and the description must say so.
#[tokio::test]
async fn a_resumed_session_re_enters_its_state_dir_and_reopens_the_scratch() {
    let root = temp_dir("resume-scratch-root");
    let id = "resume-scratch-1";
    // The first process acquires, stages a table, and exits (the runtime
    // drops; the lock releases; no file is removed).
    {
        let first = crate::interactive::session_runtime::SessionRuntime::acquire(
            &session_runtime(None),
            None,
            true,
            None,
            id,
            ApprovalPolicy::Ask,
            &root,
        )
        .expect("the first process acquires");
        first
            .universe()
            .scratch
            .as_ref()
            .expect("scratch composed")
            .run("CREATE TABLE stage AS SELECT 42 AS v")
            .await
            .expect("the first process stages a table");
    }
    // Nothing ended the database: the file is exactly where the state dir
    // put it, with the staged table in it.
    let db = root.join(id).join("scratch.duckdb");
    assert!(
        db.exists(),
        "the scratch database survives the process: {}",
        db.display()
    );
    // The resumed session reuses the same id — the state dir is re-entered,
    // not recreated — and its scratch reads the staged table back.
    let resumed = crate::interactive::session_runtime::SessionRuntime::acquire(
        &session_runtime(None),
        None,
        false,
        None,
        id,
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("the resumed session acquires");
    let rows = resumed
        .universe()
        .scratch
        .as_ref()
        .expect("scratch composed")
        .run("SELECT v FROM stage")
        .await
        .expect("the staged table comes back on resume");
    let values = &rows.rows;
    assert_eq!(
        values.len(),
        1,
        "exactly the staged row comes back: {values:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// The contrast the session journal exists to make: a resumed session
/// inherits no grant from the journal. The journal is the audit record of
/// what the user consented to — never a grant source, the deliberate
/// opposite of runs, whose grants ARE re-derived from their journal. The
/// resume re-uses the same session id and state dir; the policy it builds
/// there is empty by construction, and the journal is left byte-identical.
#[test]
fn a_resumed_session_inherits_no_grant_from_the_journal() {
    let root = temp_dir("resume-journal-root");
    let id = "resume-journal-1";
    {
        let first = crate::interactive::session_runtime::SessionRuntime::acquire(
            &session_runtime(None),
            None,
            true,
            None,
            id,
            ApprovalPolicy::Ask,
            &root,
        )
        .expect("the first process acquires");
        // A previous process granted a token — a `[s]` answer, journalled.
        first
            .journal()
            .granted("sql:analytics", saya_store::GrantSource::Prompt)
            .expect("the first process journals its grant");
        drop(first);
    }
    let before = saya_store::SessionJournal::open(root.join(id))
        .read()
        .expect("the journal reads");
    // The resume re-enters the same state dir under the same id...
    let resumed = crate::interactive::session_runtime::SessionRuntime::acquire(
        &session_runtime(None),
        None,
        false,
        None,
        id,
        ApprovalPolicy::Ask,
        &root,
    )
    .expect("the resumed session acquires");
    // ...and its grant store is empty: the journal line grants nothing.
    assert!(
        resumed.policy().grants().is_empty(),
        "a resumed session starts with an empty grant store"
    );
    let effect = saya_agent::ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    assert_eq!(
        resumed.policy().resolve(&effect, Some("sql:analytics")),
        saya_agent::ApprovalDecision::Ask,
        "the journalled grant is not in force: the call the previous process \
         was allowed still asks on resume"
    );
    // And the resume records no new consent: the journal is byte-identical.
    assert_eq!(
        saya_store::SessionJournal::open(root.join(id))
            .read()
            .expect("the journal reads"),
        before,
        "a resume re-grants nothing and re-journals nothing"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Slice 1 (G1 property 1) — a fresh session launched outside any worktree
/// with no `--workspace` binds nothing, and the composition says so at
/// startup: the notice names the fact, why the file tools are absent, and
/// the remedy. Both session loops already print `SessionRuntime::notice`,
/// so carrying it here is what makes an unbound session say so.
#[test]
fn fresh_unbound_composition_carries_the_no_workspace_notice() {
    let plain = temp_dir("unbound-notice");
    let state = temp_dir("unbound-notice-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &plain, &state)
            .expect("composition succeeds without a root");
    assert!(
        universe.root().is_none(),
        "outside a worktree with no --workspace, nothing binds"
    );
    let notice = universe
        .notice
        .as_deref()
        .expect("an unbound session says so at startup");
    assert!(
        notice.contains("No workspace is bound"),
        "the notice states the fact: {notice:?}"
    );
    assert!(
        notice.contains("file tools are unavailable"),
        "the notice names why the file tools are absent: {notice:?}"
    );
    assert!(
        notice.contains("--workspace <dir>"),
        "the notice names the explicit-bind remedy: {notice:?}"
    );
    assert!(
        notice.contains("git worktree"),
        "the notice names the worktree remedy: {notice:?}"
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

/// Slice 1 (G1 property 2) — a bound session carries no such notice: the
/// worktree case composes silently, byte-identical to today's output.
#[test]
fn a_bound_session_carries_no_such_notice() {
    let project = worktree("bound-silent");
    let state = temp_dir("bound-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &project, &state)
            .expect("composition succeeds on a worktree");
    assert!(
        universe.root().is_some(),
        "inside a worktree, the root binds"
    );
    if cfg!(windows) {
        assert!(
            universe
                .notice
                .as_deref()
                .is_some_and(|notice| notice.contains("Host commands are unavailable on Windows")),
            "Windows names the unavailable host lane: {:?}",
            universe.notice
        );
    } else {
        assert!(
            universe.notice.is_none(),
            "a bound session stays silent: {:?}",
            universe.notice
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// G2 property 1 (slices 3-5) — the unstated lane composes wherever a
/// workspace root binds: the plain `compose` path (no launch statement)
/// over a worktree carries the lane, and advertises `run_command` under
/// ask-with-prompt and under bypass.
#[cfg(not(windows))]
#[test]
fn unstated_lane_composes_where_a_root_binds() {
    let project = worktree("unstated-lane");
    let state = temp_dir("unstated-lane-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, true, &project, &state)
            .expect("composition succeeds on a worktree");
    assert!(
        universe.host_composed_for_tests().is_some(),
        "a bound root composes the lane with no statement"
    );
    let ask_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    assert!(
        ask_names.contains(&"run_command".to_string()),
        "the unstated lane advertises under ask with a prompt: {ask_names:?}"
    );
    let bypass_names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert!(
        bypass_names.contains(&"run_command".to_string()),
        "the unstated lane advertises under bypass: {bypass_names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Slice 1, extra pin the list misses — a resumed session whose record
/// predates the workspace keeps its old silence: `walk_when_unpinned` is
/// false there, so the fresh-unbound notice must not fire on a path that
/// was silent by design.
#[test]
fn a_pre_workspace_resume_stays_silent_when_unbound() {
    let plain = temp_dir("resume-silent");
    let state = temp_dir("resume-silent-state");
    let universe =
        SessionUniverse::compose(&session_runtime(None), None, None, false, &plain, &state)
            .expect("composition succeeds without a root");
    assert!(
        universe.root().is_none(),
        "no pin and no walk: nothing binds"
    );
    assert!(
        universe.notice.is_none(),
        "a pre-workspace resume keeps its old silence: {:?}",
        universe.notice
    );
    let _ = (fs::remove_dir_all(&plain), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// mode slice 2 — Plan hides the write-shaped tools from the model
// ---------------------------------------------------------------------------

/// The write-shaped names Plan must never advertise: the session members
/// plus the host lane. `run_program` joins the asserted absence only where
/// the runner composes, so the runner-bearing test below names it explicitly
/// while this shared list names the four the plain composition carries.
const PLAN_HIDDEN_TOOLS: [&str; 7] = [
    "workspace_write",
    "scratch_sql",
    "scratch_import",
    "http_fetch",
    "http_download",
    "run_program",
    "run_command",
];

/// Plan advertises exactly the read-only surface: for a fully composed
/// universe under ask-with-prompt, the Plan definition list equals the list
/// the same universe produces under read-only in Build. Compared as a set —
/// Plan is hiding, not denying: the tools are absent from the list, never
/// offered and refused.
#[test]
fn plan_advertises_exactly_the_read_only_surface() {
    let project = worktree("plan-readonly-surface");
    let state = temp_dir("plan-readonly-surface-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let plan: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Plan, ApprovalPolicy::Ask, true)
            .into_iter()
            .collect();
    let read_only: std::collections::BTreeSet<String> =
        advertised(&universe, AgentMode::Build, ApprovalPolicy::ReadOnly, true)
            .into_iter()
            .collect();
    assert_eq!(
        plan, read_only,
        "Plan hides write-shaped tools instead of offering-then-denying them, \
         so its Ask surface equals the read-only surface: plan={plan:?} read-only={read_only:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Plan hides under bypass too: bypass-with-Plan advertises no write-shaped
/// tool — the case a reader will doubt, because Build-with-bypass advertises
/// them all.
#[test]
fn plan_hides_write_shaped_tools_under_bypass_too() {
    let project = worktree("plan-bypass");
    let state = temp_dir("plan-bypass-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Plan, ApprovalPolicy::Bypass, false);
    for tool in PLAN_HIDDEN_TOOLS {
        assert!(
            !names.contains(&tool.to_string()),
            "Plan hides {tool} even under bypass (hiding, not denying): {names:?}"
        );
    }
    // The positive control: Build-with-bypass advertises the four the plain
    // composition carries.
    let build = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for tool in [
        "workspace_write",
        "scratch_sql",
        "http_fetch",
        "http_download",
    ] {
        assert!(
            build.contains(&tool.to_string()),
            "Build-with-bypass still advertises {tool}: {build:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Build is unchanged: for each of the four approval policies the Build
/// definition list is exactly what `release/0.4.1` produces — except
/// read-only and never no longer advertise `render_chart`: the tool declares
/// `external_side_effect: true`, so their enforcement denies every call,
/// and advertisement now agrees — plus `tasks_set`, the session-metadata
/// write every policy but `never` carries. Pinned by name on the plain
/// worktree composition (no runner composed, so no `run_program`; the
/// unstated host lane composes, so `run_command` rides).
///
/// The flag is the approval-surface flag: `ask` with an approval surface
/// advertises the write-shaped set; `bypass` needs no surface; `read-only`
/// and `never` hide whatever the surface. The `(Ask, false)` row — a
/// surface that can obtain no approval — keeps the anti-pattern pinned.
#[cfg(not(windows))]
#[test]
fn build_advertisement_is_pinned_for_every_approval_policy() {
    let project = worktree("build-pinned");
    let state = temp_dir("build-pinned-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for (mode, can_obtain_approval, names) in [
        (
            ApprovalPolicy::Ask,
            false,
            vec![
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
                "designate_answer",
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Ask,
            true,
            vec![
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
                "tasks_set",
                "workspace_write",
                "scratch_sql",
                "scratch_import",
                "http_fetch",
                "http_download",
                "run_command",
            ],
        ),
        (
            ApprovalPolicy::Bypass,
            false,
            vec![
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
                "tasks_set",
                "workspace_write",
                "scratch_sql",
                "scratch_import",
                "http_fetch",
                "http_download",
                "run_command",
            ],
        ),
        (
            ApprovalPolicy::ReadOnly,
            true,
            vec![
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
                "designate_answer",
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Never,
            true,
            vec![
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
                "designate_answer",
            ],
        ),
    ] {
        assert_eq!(
            advertised(&universe, AgentMode::Build, mode, can_obtain_approval),
            names
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<String>>(),
            "Build under {mode:?} pins the advertised names"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// `render_chart` is advertised exactly where the engine would not deny it:
/// ask-with-prompt or bypass under Build. For each of the four approval
/// policies × {Build, Plan}, the advertised set contains the chart iff a
/// `SessionPolicy` under that posture does not deny its effect. Tool names
/// are pinned on the chart's presence so a future drift fails loudly.
#[test]
fn render_chart_advertisement_agrees_with_enforcement_for_every_policy_and_mode() {
    use saya_agent::{ApprovalDecision, SessionPolicy, ToolEffect};
    let project = worktree("chart-advert");
    let state = temp_dir("chart-advert-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let chart_effect = ToolEffect {
        database_data: false,
        external_side_effect: true,
        requires_approval: true,
        local_state: saya_agent::LocalStateEffect::None,
    };
    for (agent_mode, policy, can_obtain_approval) in [
        (AgentMode::Build, ApprovalPolicy::Ask, true),
        (AgentMode::Build, ApprovalPolicy::Ask, false),
        (AgentMode::Build, ApprovalPolicy::Bypass, false),
        (AgentMode::Build, ApprovalPolicy::ReadOnly, true),
        (AgentMode::Build, ApprovalPolicy::Never, true),
        (AgentMode::Plan, ApprovalPolicy::Ask, true),
        (AgentMode::Plan, ApprovalPolicy::Bypass, false),
        (AgentMode::Plan, ApprovalPolicy::ReadOnly, true),
        (AgentMode::Plan, ApprovalPolicy::Never, true),
    ] {
        let definitions =
            universe.definitions(agent_mode, policy, can_obtain_approval, true, false, false);
        let advertised_chart = definitions
            .iter()
            .any(|definition| definition.name == "render_chart");
        // Denied means the engine denies — or the decider would with no
        // approval surface to answer: under ask without one every
        // approval-gated tool (the stdin fallback's `Ask if !can_prompt
        // => false` is the same shape: an unanswerable ask denies) is
        // refused, which is exactly why the write-shaped members hide
        // there too.
        let denied = matches!(
            SessionPolicy::new(policy)
                .with_agent_mode(agent_mode)
                .resolve(&chart_effect, None),
            ApprovalDecision::Deny { .. }
        ) || (policy == ApprovalPolicy::Ask && !can_obtain_approval);
        assert_eq!(
            advertised_chart, !denied,
            "{agent_mode:?} under {policy:?} (can_obtain_approval={can_obtain_approval}): advertised={advertised_chart} but denied={denied}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The normal interactive case does not regress: ask with a prompt surface
/// under Build still advertises `render_chart` alongside the chart-less SQL
/// family, in the same position it always held.
#[test]
fn ask_with_prompt_under_build_still_advertises_render_chart() {
    let project = worktree("chart-ask");
    let state = temp_dir("chart-ask-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    let position = names
        .iter()
        .position(|name| name == "render_chart")
        .expect("ask-with-prompt under Build advertises render_chart");
    assert_eq!(
        &names[position - 1..position + 2],
        &["join_check", "render_chart", "designate_answer"],
        "render_chart keeps its place in the normal interactive tool list: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The derived permit falls off: a Plan definition list contains no
/// `WriteWorkspace` definition, so the derivation at `runtime.rs:216-218`
/// yields `false`. Asserted on the definition list itself — the derivation
/// is only reachable through the full turn composition, and it reads this
/// exact property off the list.
#[test]
fn plan_definitions_carry_no_workspace_write_permit() {
    use saya_agent::LocalStateEffect;
    let project = worktree("plan-permit");
    let state = temp_dir("plan-permit-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let definitions = universe.definitions(
        AgentMode::Plan,
        ApprovalPolicy::Ask,
        true,
        true,
        false,
        false,
    );
    assert!(
        definitions
            .iter()
            .all(|definition| definition.effect.local_state != LocalStateEffect::WriteWorkspace),
        "no Plan definition writes the workspace, so the runtime derivation reads false: {:?}",
        definitions
            .iter()
            .map(|definition| &definition.name)
            .collect::<Vec<_>>()
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Advertisement matches enforcement: for every definition Plan
/// advertises, `SessionPolicy` under Plan does not deny it. An
/// advertised-but-always-refused tool is the exact anti-pattern the
/// hidden-not-advertised rule exists to prevent — this is the anti-drift
/// test between slice 1 and slice 2. `permit_candidate_writes` admits
/// nothing write-shaped into the database surface — the flag gates no tool
/// declared today.
#[test]
fn every_plan_advertised_definition_is_not_denied_by_plan_enforcement() {
    use saya_agent::{ApprovalDecision, SessionPolicy};
    let project = worktree("plan-anti-drift");
    let state = temp_dir("plan-anti-drift-state");
    let universe = compose(&session_runtime(None), &project, &state);
    for policy in [
        ApprovalPolicy::Ask,
        ApprovalPolicy::Bypass,
        ApprovalPolicy::ReadOnly,
    ] {
        let definitions = universe.definitions(
            AgentMode::Plan,
            policy,
            policy == ApprovalPolicy::Ask,
            true,
            false,
            false,
        );
        let enforcer = SessionPolicy::new(policy).with_agent_mode(AgentMode::Plan);
        for definition in &definitions {
            assert!(
                !matches!(
                    enforcer.resolve(&definition.effect, None),
                    ApprovalDecision::Deny { .. }
                ),
                "Plan advertises {} under {policy:?} but enforcement denies it",
                definition.name
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
