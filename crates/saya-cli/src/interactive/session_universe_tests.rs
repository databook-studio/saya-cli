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
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            investigation_budgets: Default::default(),
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
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-session-universe"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
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

const SESSION_WRITE_TOOLS: [&str; 7] = [
    "workspace_write",
    "workspace_edit",
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
            recovery: None,
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
            recovery: None,
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
            recovery: None,
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
            recovery: None,
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
            "workspace_edit",
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
        for tool in ["workspace_write", "workspace_edit"] {
            let write = definitions
                .iter()
                .find(|definition| definition.name == tool)
                .expect("the guarded file tool is advertised under bypass");
            assert!(
                write.effect.requires_approval,
                "{tool} still declares its approval shape honestly: the engine resolves it"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The digest-guarded edit rides beside the write, and only there: a bound
/// Build session advertises `workspace_edit` exactly once — no duplicate
/// from the run surface (whose `permit_workspace_writes` is off) and no
/// drift into compositions that cannot use it — while an unbound session
/// (no workspace root) and Plan mode never see it.
#[test]
fn a_bound_session_advertises_workspace_edit_exactly_once_and_unbound_and_plan_do_not() {
    let project = worktree("edit-advertise");
    let state = temp_dir("edit-advertise-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "workspace_edit")
            .count(),
        1,
        "the bound Build session advertises workspace_edit exactly once: {names:?}"
    );
    let plain = temp_dir("edit-advertise-unbound");
    let plain_state = temp_dir("edit-advertise-unbound-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    assert!(
        !advertised(&unbound, AgentMode::Build, ApprovalPolicy::Bypass, false)
            .contains(&"workspace_edit".to_string()),
        "no root, no edit tool"
    );
    assert!(
        !advertised(&universe, AgentMode::Plan, ApprovalPolicy::Bypass, false)
            .contains(&"workspace_edit".to_string()),
        "Plan hides the edit tool even under bypass"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
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
        "workspace_edit",
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
        "workspace_edit",
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

// ---------------------------------------------------------------------------
// deletion tombstones gate acquisition
// ---------------------------------------------------------------------------

/// A tombstoned id refuses acquisition BEFORE the state dir is touched: a
/// deleted or being-deleted session cannot be resurrected by an open
/// attempt, and its `sessions/<id>/` stays absent.
#[test]
fn a_deleted_session_cannot_be_acquired_and_its_state_dir_is_not_recreated() {
    let root = temp_dir("tombstone-acquire");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    store
        .begin_delete("tombstoned-id")
        .expect("the tombstone is written");
    let error = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "tombstoned-id",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .map(|_: crate::interactive::session_runtime::SessionRuntime| ())
    .expect_err("a tombstoned id refuses acquisition");
    assert!(
        error.contains("was deleted"),
        "the refusal names the deleted session: {error}"
    );
    assert!(
        !sessions_root.join("tombstoned-id").exists(),
        "acquisition must not recreate the deleted session's state dir"
    );
    let _ = fs::remove_dir_all(&root);
}

/// Deletion through the lifecycle owner: while a live runtime holds the id,
/// `delete_session` refuses (LockHeld); once the runtime drops, the deletion
/// completes and the id is FREE. That freedom is a recorded decision, not an
/// accident: a completed deletion removes the tombstone too, so nothing
/// remembers the old record and acquiring the same id again starts a FRESH
/// state dir and works.
#[test]
fn a_session_deleted_through_the_lifecycle_owner_cannot_be_reacquired() {
    use saya_store::SessionStore;

    let root = temp_dir("tombstone-lifecycle");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    // A real saved record behind the id, so the deletion removes a live one.
    super::super::session_resume::block_on(
        store.save(
            super::super::SessionState::new("lifecycle-deleted", None, "test-model").redacted(),
        ),
    )
    .expect("the session record saves");
    let live = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lifecycle-deleted",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the live session acquires");
    let error = saya_harness::session_lifecycle::delete_session(
        &store,
        &sessions_root,
        "lifecycle-deleted",
    )
    .expect_err("deletion refuses a live holder");
    let error = format!("{error}");
    assert!(
        error.contains("held by a live process") && error.contains("pid"),
        "the refusal names the live holder: {error}"
    );
    drop(live);
    let status = saya_harness::session_lifecycle::delete_session(
        &store,
        &sessions_root,
        "lifecycle-deleted",
    )
    .expect("the deletion completes once the holder is gone");
    assert_eq!(status, saya_store::DeleteStatus::Complete);
    // DECISION: a completed deletion frees the id — the next acquisition of
    // it starts a FRESH state dir and succeeds.
    let fresh = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        true,
        None,
        "lifecycle-deleted",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the id is free after a completed deletion: a fresh session acquires it");
    assert!(
        sessions_root.join("lifecycle-deleted").exists(),
        "the fresh acquisition created a fresh state dir"
    );
    drop(fresh);
    let _ = fs::remove_dir_all(&root);
}

/// The `/resume` seam (`reacquire`) refuses a tombstoned id the same way,
/// and the refused swap leaves the current session untouched: it still holds
/// its own id's lock, and its question lifecycle keeps working.
#[test]
fn reacquire_refuses_a_tombstoned_id() {
    let root = temp_dir("tombstone-reacquire");
    let sessions_root = root.join("sessions");
    let runtime = session_runtime(None);
    let store = saya_store::FsSessionStore::new(sessions_root.clone());
    let mut session = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "reacquire-live",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .expect("the live session acquires");
    store
        .begin_delete("reacquire-tombstoned")
        .expect("the tombstone is written");
    let error = session
        .reacquire(
            &runtime,
            None,
            "reacquire-tombstoned",
            saya_agent::ApprovalPolicy::Ask,
        )
        .expect_err("a tombstoned id refuses the swap");
    assert!(
        error.contains("was deleted"),
        "the refusal names the deleted session: {error}"
    );
    // The old runtime is not replaced: it still holds its own id's lock.
    let again = crate::interactive::session_runtime::SessionRuntime::acquire(
        &runtime,
        None,
        false,
        None,
        "reacquire-live",
        saya_agent::ApprovalPolicy::Ask,
        &sessions_root,
    )
    .map(|_: crate::interactive::session_runtime::SessionRuntime| ())
    .expect_err("the old runtime still holds its lock");
    assert!(
        again.contains("already running") && again.contains("pid"),
        "the old runtime kept its lock: {again}"
    );
    let scope = session
        .begin_question(&runtime)
        .expect("the old runtime's question lifecycle still works");
    session.settle_question(&scope, false);
    drop(session);
    let _ = fs::remove_dir_all(&root);
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
            recovery: None,
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
            recovery: None,
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
const PLAN_HIDDEN_TOOLS: [&str; 8] = [
    "workspace_write",
    "workspace_edit",
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
        "workspace_edit",
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
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Ask,
            true,
            vec![
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
                "tasks_set",
                "workspace_write",
                "workspace_edit",
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
                "tasks_set",
                "workspace_write",
                "workspace_edit",
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
                "tasks_set",
            ],
        ),
        (
            ApprovalPolicy::Never,
            true,
            vec![
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

// ---------------------------------------------------------------------------
// the grant surface — an explicit workspace-write grant in headless ask
// ---------------------------------------------------------------------------

/// A session policy with `tokens` granted, bound onto the universe exactly
/// the way the session surfaces bind each turn: `session_request::run` and
/// the TUI spawn read the policy they hold and bind its grant store onto
/// the session universe before the turn runs. Returns the policy so a test
/// can resolve through it like the turn's decider does.
fn granted(universe: &SessionUniverse, tokens: &[&str]) -> saya_agent::SessionPolicy {
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    for token in tokens {
        policy.grants().grant(token);
    }
    universe.bind_grants(policy.grants());
    policy
}

/// POST-03, red at the real definitions boundary: a headless ask session
/// (no approval surface, `can_obtain_approval=false`) whose live policy
/// holds an explicit `workspace-write` grant advertises exactly the two
/// guarded file tools — the grant pre-answers their calls
/// (`SessionPolicy::resolve` → `Allow`), so the advertised tools are
/// usable, never the advertised-but-unusable anti-pattern — and nothing
/// else write-shaped: the grant is one family, not a posture. No
/// `render_chart` (the chart rides the approval surface alone), no
/// `scratch_sql`, no `run_command`, no `run_program`.
#[test]
fn a_headless_ask_session_advertises_only_the_two_file_tools_under_its_workspace_write_grant() {
    let project = worktree("grant-headless");
    let state = temp_dir("grant-headless-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let policy = granted(&universe, &["workspace-write"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["workspace_write", "workspace_edit"] {
        assert!(
            names.contains(&tool.to_string()),
            "the granted workspace file family advertises under headless ask: {names:?}"
        );
    }
    for tool in [
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_program",
        "run_command",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a workspace-write grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The advertised calls are usable and honestly declared: the engine
    // resolves the grant to `Allow`, and each definition still carries its
    // per-call approval shape.
    let definitions = universe.definitions(
        AgentMode::Build,
        ApprovalPolicy::Ask,
        false,
        true,
        false,
        false,
    );
    for tool in ["workspace_write", "workspace_edit"] {
        let definition = definitions
            .iter()
            .find(|definition| definition.name == tool)
            .expect("the granted file tool is advertised");
        assert!(
            definition.effect.requires_approval,
            "{tool} declares its approval shape honestly: the engine resolves the grant"
        );
    }
    let effect = &definitions
        .iter()
        .find(|definition| definition.name == "workspace_write")
        .expect("workspace_write advertised")
        .effect;
    assert_eq!(
        policy.resolve(effect, Some("workspace-write")),
        saya_agent::ApprovalDecision::Allow,
        "the grant pre-answers the advertised call: usable, not advertised-but-denied"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A held workspace-write grant cannot widen a restrictive posture:
/// read-only and never never consult grants (the engine denies without
/// them), and Plan denies before the approval match — so the file tools
/// stay hidden under each, grant or no grant. The advertisement rule never
/// outruns the engine.
#[test]
fn a_workspace_write_grant_cannot_widen_read_only_never_or_plan() {
    let project = worktree("grant-restrictive");
    let state = temp_dir("grant-restrictive-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["workspace-write"]);
    for (agent_mode, mode, can_obtain_approval, label) in [
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            "read-only",
        ),
        (AgentMode::Build, ApprovalPolicy::Never, true, "never"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            false,
            "plan, headless",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "plan, with a surface",
        ),
    ] {
        let names = advertised(&universe, agent_mode, mode, can_obtain_approval);
        for tool in ["workspace_write", "workspace_edit"] {
            assert!(
                !names.contains(&tool.to_string()),
                "{label} stays closed under a held grant: {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// No grant, no advertisement — the headless ask shape stays exactly as
/// today — and a grant from another family (`scratch`) is not the workspace
/// file family: one token authorizes its own family only.
#[test]
fn an_unbound_view_and_a_foreign_grant_keep_the_headless_ask_closed() {
    let project = worktree("grant-closed");
    let state = temp_dir("grant-closed-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["workspace_write", "workspace_edit"] {
        assert!(
            !names.contains(&tool.to_string()),
            "no grant: the headless ask stays closed exactly as today: {names:?}"
        );
    }
    granted(&universe, &["scratch"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["workspace_write", "workspace_edit"] {
        assert!(
            !names.contains(&tool.to_string()),
            "a foreign family's grant authorizes nothing here: {names:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The interactive shapes do not move: ask with an approval surface
/// advertises the full write-shaped set exactly as before, grant held or
/// not — and the granted case advertises the file tools exactly once, no
/// duplicate from the grant arm. Bypass is untouched: the mode allows on
/// its own.
#[test]
fn interactive_ask_advertisement_is_unchanged_under_a_bound_grant_view() {
    let project = worktree("grant-interactive");
    let state = temp_dir("grant-interactive-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["workspace-write"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, true);
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_command",
    ] {
        assert!(
            names.contains(&tool.to_string()),
            "an approval surface advertises as before, grant or no grant: {names:?}"
        );
    }
    assert_eq!(
        names
            .iter()
            .filter(|name| *name == "workspace_edit")
            .count(),
        1,
        "the granted ask advertises workspace_edit exactly once: {names:?}"
    );
    let bypass = advertised(&universe, AgentMode::Build, ApprovalPolicy::Bypass, false);
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "http_fetch",
    ] {
        assert!(
            bypass.contains(&tool.to_string()),
            "bypass unchanged under a bound grant view: {bypass:?}"
        );
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The process-level confirmation, through the turn boundary: a headless
/// ask session whose live policy holds the workspace-write grant runs a
/// `workspace_write` call the model issues — the definitions the turn
/// builds include the granted family, the derived workspace-write permit
/// follows, and the file lands in the workspace. Without the grant the
/// same call is an unsupported tool call: the family stays hidden and the
/// turn fails closed.
#[tokio::test]
async fn a_headless_ask_session_runs_a_workspace_write_call_under_its_grant() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ScriptedWrite {
        answered_tool_call: AtomicBool,
    }
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedWrite {
        fn name(&self) -> &str {
            "scripted-write"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            if !self.answered_tool_call.swap(true, Ordering::SeqCst) {
                let mut message = saya_agent::ChatMessage::text("assistant", "");
                message.tool_calls = vec![saya_agent::ToolCall {
                    id: "call-1".into(),
                    name: "workspace_write".into(),
                    arguments: serde_json::json!({
                        "path": "grant-headless.txt",
                        "content": "written under the grant\n"
                    }),
                }];
                Ok(saya_agent::ChatResponse::new(message))
            } else {
                Ok(saya_agent::ChatResponse::new(
                    saya_agent::ChatMessage::text("assistant", "done"),
                ))
            }
        }
    }
    struct RecordingSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);
    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for RecordingSink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let project = worktree("grant-process");
    let state = temp_dir("grant-process-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = granted(&universe, &["workspace-write"]);
    // The turn's real decider: over the same live policy (a clone shares
    // the grant store), unable to prompt, carrying the session's own facts
    // and primary handle — the shape `session_request::run` builds.
    let decider: Arc<dyn saya_agent::ApprovalDecider> = Arc::new(
        crate::prompt_approval::TerminalApproval::from_session_with_cancellation(
            policy,
            false,
            universe.primary.clone(),
            universe.approval_facts(&runtime),
            None,
            saya_agent::CancellationToken::new(),
        ),
    );
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedWrite {
            answered_tool_call: AtomicBool::new(false),
        }),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "write the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &RecordingSink(std::sync::Mutex::new(Vec::new())),
        saya_agent::CancellationToken::new(),
        None,
        Some(decider),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    match output {
        Ok(_) => {}
        Err(error) => {
            panic!("the granted workspace_write call must run, never be unsupported: {error:?}")
        }
    }
    assert!(
        project.join("grant-headless.txt").exists(),
        "the granted call wrote through the workspace seam"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The closed shape at the process level: the same headless ask turn with
/// NO grant keeps the family hidden — the model's `workspace_write` call
/// fails validation as an unknown tool and is fed back as a tool result,
/// never executed, with the workspace untouched. No grant, no
/// advertisement, no execution.
#[tokio::test]
async fn a_headless_ask_session_without_the_grant_refuses_the_workspace_write_call() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;

    struct ScriptedWrite;
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedWrite {
        fn name(&self) -> &str {
            "scripted-write"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            let mut message = saya_agent::ChatMessage::text("assistant", "");
            message.tool_calls = vec![saya_agent::ToolCall {
                id: "call-1".into(),
                name: "workspace_write".into(),
                arguments: serde_json::json!({
                    "path": "grant-headless.txt",
                    "content": "must not be written\n"
                }),
            }];
            Ok(saya_agent::ChatResponse::new(message))
        }
    }
    struct RecordingSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);
    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for RecordingSink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let project = worktree("grant-process-closed");
    let state = temp_dir("grant-process-closed-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = saya_agent::SessionPolicy::new(ApprovalPolicy::Ask);
    universe.bind_grants(policy.grants());
    let decider: Arc<dyn saya_agent::ApprovalDecider> = Arc::new(
        crate::prompt_approval::TerminalApproval::from_session_with_cancellation(
            policy,
            false,
            universe.primary.clone(),
            universe.approval_facts(&runtime),
            None,
            saya_agent::CancellationToken::new(),
        ),
    );
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedWrite),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "write the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        Some(decider),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    // The turn itself completes: an unadvertised call is fed back as a
    // failed tool result, never executed.
    output.expect("the closed turn completes; the call was refused, not run");
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                saya_agent::AgentEvent::ToolCompleted { name, .. }
                    if name.as_str() == "workspace_write"
            )),
            "the refused call was seen and fed back as a tool result: {events:?}"
        );
    }
    assert!(
        !project.join("grant-headless.txt").exists(),
        "the workspace was never written"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

// ---------------------------------------------------------------------------
// the remaining grant families — scratch, fetch, runner, command (POST-03 G02)
// ---------------------------------------------------------------------------

/// The headless-ask shape for the scratch family: a held `scratch` grant
/// advertises `scratch_sql` — and `scratch_import` exactly where a
/// workspace root binds, the same composition facts `grant_token` reads
/// before offering the token. No other family moves: one token opens one
/// family, and the ungrantable effects (`render_chart`, the fan-out)
/// stay on the approval-surface rule alone.
#[test]
fn a_headless_ask_session_advertises_the_scratch_family_under_its_grant() {
    let project = worktree("grant-scratch");
    let state = temp_dir("grant-scratch-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["scratch"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["scratch_sql", "scratch_import"] {
        assert!(
            names.contains(&tool.to_string()),
            "the held scratch grant advertises {tool} under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "http_fetch",
        "http_download",
        "run_command",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a scratch grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The ungrantable fan-out is a read-surface tool: its advertisement is
    // unchanged by the grant — present exactly where `allow_query_data` put
    // it, widened by nothing.
    assert!(
        names.contains(&"bounded_sql_query_all".to_string()),
        "the fan-out rides the read surface as today, untouched by the grant: {names:?}"
    );
    // The member-composed half: no workspace root, no contained import —
    // scratch_sql still rides the grant, scratch_import does not.
    let plain = temp_dir("grant-scratch-plain");
    let plain_state = temp_dir("grant-scratch-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["scratch"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "scratch needs no root: {names:?}"
    );
    assert!(
        !names.contains(&"scratch_import".to_string()),
        "no root, no contained import, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}

/// The fetch family: a held `fetch:` token advertises `http_fetch` — and
/// `http_download` exactly where a workspace root binds (the download needs
/// a destination, the same condition `grant_token` mirrors). The chart and
/// the fan-out stay hidden: no grant token exists for them, and a fetch
/// grant must never stand in for the approval surface they ride.
#[test]
fn a_headless_ask_session_advertises_the_fetch_family_under_its_grant() {
    let project = worktree("grant-fetch");
    let state = temp_dir("grant-fetch-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["fetch:https+docs.example"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    for tool in ["http_fetch", "http_download"] {
        assert!(
            names.contains(&tool.to_string()),
            "the held fetch grant advertises {tool} under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "run_command",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a fetch grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // No root, no download destination — fetch alone rides the grant.
    let plain = temp_dir("grant-fetch-plain");
    let plain_state = temp_dir("grant-fetch-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["fetch:https+docs.example"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"http_fetch".to_string()),
        "fetch needs no root: {names:?}"
    );
    assert!(
        !names.contains(&"http_download".to_string()),
        "no root, no download destination, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}

/// The host-command lane: a held `command:` token advertises `run_command`
/// where the lane composed — and nothing where it did not (no workspace
/// root, no lane, structural). On Windows the lane is unavailable and stays
/// hidden, grant or no grant.
#[test]
fn a_headless_ask_session_advertises_run_command_only_where_the_lane_composed() {
    let project = worktree("grant-command");
    let state = temp_dir("grant-command-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["command:cargo"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    if cfg!(windows) {
        assert!(
            !names.contains(&"run_command".to_string()),
            "Windows keeps the unavailable lane hidden under a grant: {names:?}"
        );
    } else {
        assert!(
            names.contains(&"run_command".to_string()),
            "the held command grant advertises the composed lane under headless ask: {names:?}"
        );
    }
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_program",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a command grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // No root, no lane: the grant gates nothing.
    let plain = temp_dir("grant-command-plain");
    let plain_state = temp_dir("grant-command-plain-state");
    let unbound = SessionUniverse::compose(
        &session_runtime(None),
        None,
        None,
        true,
        &plain,
        &plain_state,
    )
    .expect("composition succeeds without a root");
    granted(&unbound, &["command:cargo"]);
    let names = advertised(&unbound, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"run_command".to_string()),
        "no root, no lane, grant or no grant: {names:?}"
    );
    let _ = (
        fs::remove_dir_all(&project),
        fs::remove_dir_all(&state),
        fs::remove_dir_all(&plain),
        fs::remove_dir_all(&plain_state),
    );
}

/// The runner family, where a runner composed: a `runner:` token for an
/// allowlisted program — or an `interpreter:` token for a staged one —
/// advertises `run_program` under headless ask. Anywhere the runner did
/// not compose (any platform, no program_dir), the same grants advertise
/// nothing: a capability the composition could not construct is never
/// conjured by a grant.
#[cfg(target_os = "macos")]
#[test]
fn a_headless_ask_session_advertises_run_program_under_runner_and_interpreter_grants() {
    let project = worktree("grant-runner");
    let state = temp_dir("grant-runner-state");
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
    granted(&universe, &["runner:touch"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"run_program".to_string()),
        "the held runner grant advertises the composed runner under headless ask: {names:?}"
    );
    for tool in [
        "workspace_write",
        "workspace_edit",
        "scratch_sql",
        "scratch_import",
        "http_fetch",
        "http_download",
        "run_command",
        "render_chart",
    ] {
        assert!(
            !names.contains(&tool.to_string()),
            "a runner grant widens no other family: {tool} stays hidden: {names:?}"
        );
    }
    // The interpreter door rides `run_program`'s advertisement: a staged
    // interpreter's token opens the same single tool.
    granted(&universe, &["interpreter:python3"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"run_program".to_string()),
        "the interpreter token opens the same composed runner tool: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

#[test]
fn a_runner_grant_advertises_nothing_where_no_runner_composed() {
    let project = worktree("grant-runner-absent");
    let state = temp_dir("grant-runner-absent-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(&universe, &["runner:touch", "interpreter:python3"]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"run_program".to_string()),
        "no composed runner, no run_program — grant or no grant: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// Every family grant at once cannot widen a restrictive posture:
/// read-only and never never consult grants, and Plan denies before the
/// approval match — every family stays hidden under each. The `sql:`
/// token rides along to pin that the SQL family behaves exactly as today
/// (it widens none of these families; its own tools ride the read surface).
#[test]
fn no_family_grant_widens_read_only_never_or_plan() {
    let project = worktree("grant-restrictive-families");
    let state = temp_dir("grant-restrictive-families-state");
    let universe = compose(&session_runtime(None), &project, &state);
    granted(
        &universe,
        &[
            "workspace-write",
            "scratch",
            "fetch:https+docs.example",
            "runner:touch",
            "interpreter:python3",
            "command:cargo",
            "sql:docs",
        ],
    );
    for (agent_mode, mode, can_obtain_approval, label) in [
        (
            AgentMode::Build,
            ApprovalPolicy::ReadOnly,
            true,
            "read-only",
        ),
        (AgentMode::Build, ApprovalPolicy::Never, true, "never"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            false,
            "plan, headless",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "plan, with a surface",
        ),
    ] {
        let names = advertised(&universe, agent_mode, mode, can_obtain_approval);
        for tool in [
            "workspace_write",
            "workspace_edit",
            "scratch_sql",
            "scratch_import",
            "http_fetch",
            "http_download",
            "run_program",
            "run_command",
            "render_chart",
        ] {
            assert!(
                !names.contains(&tool.to_string()),
                "{label} stays closed under every family grant: {tool} hidden: {names:?}"
            );
        }
    }
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// A grant recorded after the turn (by this turn's decider, through the
/// same live store the surfaces bind) recomputes into the next turn's
/// definitions: the availability decision reads the live store per turn,
/// never a copy taken once.
#[test]
fn a_grant_recorded_after_the_turn_recomputes_into_the_next_definitions() {
    let project = worktree("grant-recompute");
    let state = temp_dir("grant-recompute-state");
    let universe = compose(&session_runtime(None), &project, &state);
    let policy = granted(&universe, &[]);
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        !names.contains(&"scratch_sql".to_string()),
        "no grant yet: the headless ask stays closed: {names:?}"
    );
    // The decider records the grant mid-session; the store is the one the
    // view is bound to, so the next turn's definitions see it.
    policy.grants().grant("scratch");
    let names = advertised(&universe, AgentMode::Build, ApprovalPolicy::Ask, false);
    assert!(
        names.contains(&"scratch_sql".to_string()),
        "the recorded grant recomputes into the next turn's definitions: {names:?}"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}

/// The turn-boundary confirmation: a headless ask session whose live
/// policy holds `fetch:https+a.example` runs its turn with `http_download`
/// advertised, but a call naming a different host is denied at call time —
/// the engine builds the call's own token from the actual arguments and
/// only an exactly granted one pre-answers — with no prompt (headless
/// surfaces cannot read one) and the destination never written. The
/// granted host's call, by contrast, resolves `Allow` at the decider.
#[tokio::test]
async fn a_headless_ask_session_denies_a_foreign_host_call_under_its_fetch_grant() {
    use crate::agent::runtime::run_prompt_with_inputs;
    use crate::agent::turn_inputs::TurnInputs;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    struct ScriptedDownload {
        issued: AtomicBool,
    }
    #[async_trait::async_trait]
    impl saya_agent::ChatProvider for ScriptedDownload {
        fn name(&self) -> &str {
            "scripted-download"
        }
        async fn complete(
            &self,
            _request: saya_agent::ChatRequest,
        ) -> Result<saya_agent::ChatResponse, saya_agent::ProviderError> {
            if !self.issued.swap(true, Ordering::SeqCst) {
                let mut message = saya_agent::ChatMessage::text("assistant", "");
                message.tool_calls = vec![saya_agent::ToolCall {
                    id: "call-1".into(),
                    name: "http_download".into(),
                    arguments: serde_json::json!({
                        "url": "https://b.example/denied.bin",
                        "destination": "denied.bin"
                    }),
                }];
                Ok(saya_agent::ChatResponse::new(message))
            } else {
                Ok(saya_agent::ChatResponse::new(
                    saya_agent::ChatMessage::text("assistant", "done"),
                ))
            }
        }
    }
    struct RecordingSink(std::sync::Mutex<Vec<saya_agent::AgentEvent>>);
    #[async_trait::async_trait]
    impl saya_agent::AgentEventSink for RecordingSink {
        async fn emit(&self, event: saya_agent::AgentEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    let project = worktree("grant-fetch-process");
    let state = temp_dir("grant-fetch-process-state");
    let runtime = session_runtime(None);
    let universe = Arc::new(compose(&runtime, &project, &state));
    let policy = granted(&universe, &["fetch:https+a.example"]);
    let definitions = universe.definitions(
        AgentMode::Build,
        ApprovalPolicy::Ask,
        false,
        true,
        false,
        false,
    );
    let download = definitions
        .iter()
        .find(|definition| definition.name == "http_download")
        .expect("the held fetch grant advertises http_download with a workspace bound");
    // The turn's real decider, unable to prompt — the headless shape.
    let decider: Arc<dyn saya_agent::ApprovalDecider> = Arc::new(
        crate::prompt_approval::TerminalApproval::from_session_with_cancellation(
            policy,
            false,
            universe.primary.clone(),
            universe.approval_facts(&runtime),
            None,
            saya_agent::CancellationToken::new(),
        ),
    );
    // The granted host's call pre-answers; the foreign host's does not —
    // one fetch token authorizes one destination, judged per call.
    assert!(
        decider
            .approve(
                download,
                &serde_json::json!({"url": "https://a.example/ok.bin", "destination": "ok.bin"}),
            )
            .await,
        "the granted destination's call is allowed at the decider"
    );
    assert!(
        !decider
            .approve(
                download,
                &serde_json::json!({
                    "url": "https://b.example/denied.bin",
                    "destination": "denied.bin"
                }),
            )
            .await,
        "a different host under the same family grant is denied at the decider"
    );
    let inputs = TurnInputs {
        ai: runtime.resolved.ai.clone(),
        provider: Box::new(ScriptedDownload {
            issued: AtomicBool::new(false),
        }),
        registry: crate::connection::ConnectionRegistry::new(""),
        failures: Vec::new(),
    };
    let sink = RecordingSink(std::sync::Mutex::new(Vec::new()));
    let output = run_prompt_with_inputs(
        &runtime,
        inputs,
        "download the file",
        ApprovalPolicy::Ask,
        false,
        false,
        Vec::new(),
        &sink,
        saya_agent::CancellationToken::new(),
        None,
        Some(Arc::clone(&decider)),
        None,
        Some(Arc::clone(&universe)),
        AgentMode::Build,
        None,
        None,
    )
    .await;
    output.expect("the turn completes: the foreign call was denied, not run");
    {
        let events = sink.0.lock().unwrap();
        assert!(
            events.iter().any(|event| matches!(
                event,
                saya_agent::AgentEvent::ToolDenied { name, .. }
                    if name.as_str() == "http_download"
            )),
            "the foreign-host call was denied at the call boundary: {events:?}"
        );
    }
    assert!(
        !project.join("denied.bin").exists(),
        "the denied call never wrote its destination"
    );
    let _ = (fs::remove_dir_all(&project), fs::remove_dir_all(&state));
}
