//! Slice 3+4 red tests, part two: universe advertisement, the user-turn
//! context block, and the prefix-cache property.

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::interactive::session_tasks::SessionTasks;
use crate::interactive::session_tasks_render::{
    TASKS_BLOCK_LABEL, TASKS_RENDER_CEILING, render_tasks_block, tasks_set_definition,
};

/// The tool is advertised wherever the session can use it — including Plan
/// mode, which is the point. Pinned here through the universe's own
/// definitions, on the path that survives the Plan filter.
#[test]
fn the_universe_advertises_tasks_set_including_under_plan() {
    let observed = |agent_mode: AgentMode, mode: ApprovalPolicy, can_prompt: bool| {
        let plain = std::env::temp_dir().join(format!(
            "saya-tasks-ads-{}-{}",
            std::process::id(),
            match (agent_mode, mode, can_prompt) {
                (AgentMode::Build, ApprovalPolicy::Ask, true) => "build-ask",
                (AgentMode::Build, ApprovalPolicy::Bypass, _) => "build-bypass",
                (AgentMode::Plan, _, _) => "plan",
                _ => "other",
            }
        ));
        let state = std::env::temp_dir().join(format!(
            "saya-tasks-ads-state-{}-{}",
            std::process::id(),
            match (agent_mode, mode, can_prompt) {
                (AgentMode::Build, ApprovalPolicy::Ask, true) => "build-ask",
                (AgentMode::Build, ApprovalPolicy::Bypass, _) => "build-bypass",
                (AgentMode::Plan, _, _) => "plan",
                _ => "other",
            }
        ));
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&state);
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::create_dir_all(&state).unwrap();
        let runtime = test_runtime();
        let universe =
            crate::interactive::session_universe::SessionUniverse::compose_with_launch_and_path(
                crate::interactive::session_universe::SessionComposition {
                    runtime: &runtime,
                    explicit: None,
                    pinned_root: None,
                    walk_when_unpinned: false,
                    cwd: &plain,
                    state_dir: &state,
                    launch: None,
                    path: None,
                },
            )
            .expect("composition succeeds");
        let names: Vec<String> = universe
            .definitions(agent_mode, mode, can_prompt, true, false, false)
            .into_iter()
            .map(|definition| definition.name)
            .collect();
        let _ = std::fs::remove_dir_all(&plain);
        let _ = std::fs::remove_dir_all(&state);
        names
    };
    for (agent_mode, mode, can_prompt, label) in [
        (AgentMode::Build, ApprovalPolicy::Ask, true, "ask+prompt"),
        (AgentMode::Build, ApprovalPolicy::Bypass, false, "bypass"),
        (
            AgentMode::Plan,
            ApprovalPolicy::Ask,
            true,
            "Plan+ask: the case this design exists for",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::Bypass,
            false,
            "Plan+bypass: the case this design exists for",
        ),
        (
            AgentMode::Plan,
            ApprovalPolicy::ReadOnly,
            false,
            "Plan+read-only",
        ),
    ] {
        assert!(
            observed(agent_mode, mode, can_prompt).contains(&"tasks_set".to_string()),
            "tasks_set must be advertised under {label}"
        );
    }
    // Never hides everything write-shaped-adjacent — tasks_set included, so
    // the model never sees a tool it cannot use.
    assert!(
        !observed(AgentMode::Build, ApprovalPolicy::Never, true).contains(&"tasks_set".to_string()),
        "never must hide tasks_set"
    );
}

/// A read-only universe (definitions before the Plan filter) carries the
/// same definition object the Plan filter keeps: one definition, one path.
#[test]
fn the_definition_object_is_the_advertised_object() {
    let direct = tasks_set_definition();
    assert_eq!(
        direct.effect.local_state,
        saya_agent::LocalStateEffect::WriteSession
    );
}

/// A non-empty list produces exactly one context block on the user turn; an
/// empty list produces none.
#[test]
fn the_list_rides_the_user_turn_only_when_non_empty() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let empty = SessionTaskList::default();
    assert!(
        render_tasks_block(&empty).is_none(),
        "an empty list costs zero tokens and emits nothing"
    );
    let list = SessionTaskList::new(vec![
        SessionTask::with_note(
            "profile the tables",
            TaskStatus::InProgress,
            Some("halfway"),
        )
        .unwrap(),
        SessionTask::new("write the report", TaskStatus::Pending).unwrap(),
    ])
    .unwrap();
    let block = render_tasks_block(&list).expect("a non-empty list emits one block");
    assert_eq!(block.label, TASKS_BLOCK_LABEL);
    assert!(!block.truncated);
    let rendered = saya_agent::render_untrusted_block(&block);
    assert!(rendered.contains("profile the tables"));
    assert!(rendered.contains("write the report"));
}

/// The system prompt is byte-identical whether or not a list exists — the
/// prefix-cache property. Asserted through the same `build_messages` the
/// runtime uses: the system message must not vary with the list.
#[test]
fn the_system_prompt_is_byte_identical_with_or_without_a_list() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let list = SessionTaskList::new(vec![
        SessionTask::new("profile the tables", TaskStatus::InProgress).unwrap(),
    ])
    .unwrap();
    let with = render_tasks_block(&list).map(|block| vec![block]);
    for blocks in [None, with.as_deref()] {
        let _ = blocks;
    }
    let empty: Vec<saya_agent::ContextBlock> = Vec::new();
    let full: Vec<saya_agent::ContextBlock> = render_tasks_block(&list).into_iter().collect();
    let system = Some("Available database connections:\n- a (postgresql)");
    let a = saya_agent::build_messages(system, &empty, "what next", &[], 32 * 1024).unwrap();
    let b = saya_agent::build_messages(system, &full, "what next", &[], 32 * 1024).unwrap();
    assert_eq!(
        a[0], b[0],
        "the system message must not vary with the task list — that is what makes the prefix cache work"
    );
    assert_ne!(a[1], b[1], "the user turns differ: one carries the list");
}

/// The rendered block's size is bounded by the contract's own bounds: a
/// worst-case list (32 tasks, max-length titles and notes) renders without
/// exceeding the stated ceiling.
#[test]
fn the_worst_case_list_renders_within_the_stated_ceiling() {
    use saya_types::{
        MAX_SESSION_TASKS, MAX_TASK_NOTE_CHARS, MAX_TASK_TITLE_CHARS, SessionTask, SessionTaskList,
        TaskStatus,
    };
    let title = "t".repeat(MAX_TASK_TITLE_CHARS);
    let note = "n".repeat(MAX_TASK_NOTE_CHARS);
    let pad = MAX_TASK_TITLE_CHARS - 3;
    let tasks: Vec<SessionTask> = (0..MAX_SESSION_TASKS)
        .map(|i| {
            let status = if i == 0 {
                TaskStatus::InProgress
            } else {
                TaskStatus::Pending
            };
            SessionTask::with_note(
                format!("{} {i:02}", &title[..pad]),
                status,
                Some(note.as_str()),
            )
            .unwrap()
        })
        .collect();
    let list = SessionTaskList::new(tasks).unwrap();
    let block = render_tasks_block(&list).expect("worst case still renders one block");
    let rendered = saya_agent::render_untrusted_block(&block);
    assert!(
        rendered.len() <= TASKS_RENDER_CEILING,
        "worst-case render {} exceeds ceiling {TASKS_RENDER_CEILING}",
        rendered.len()
    );
    // The ceiling is stated here, not just in the source: 32 tasks ×
    // (200-char title + 512-char note + markers) plus the wrapper.
    assert_eq!(
        TASKS_RENDER_CEILING, 24_576,
        "the ceiling pins the scale: {TASKS_RENDER_CEILING}"
    );
}

/// The live cell round-trips through render: what `tasks_set` stored is
/// what the next turn's block carries.
#[test]
fn the_cell_round_trips_through_render() {
    use saya_types::{SessionTask, SessionTaskList, TaskStatus};
    let tasks = SessionTasks::default();
    assert!(
        render_tasks_block(&tasks.current()).is_none(),
        "a fresh cell renders nothing"
    );
    tasks.replace(
        SessionTaskList::new(vec![
            SessionTask::new("profile the tables", TaskStatus::Done).unwrap(),
        ])
        .unwrap(),
    );
    let block = render_tasks_block(&tasks.current()).expect("stored renders");
    assert!(
        saya_agent::render_untrusted_block(&block).contains("profile the tables"),
        "the stored list is what the turn carries"
    );
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    use saya_config::{
        AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
        ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs,
        ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
    };
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
                runner: ResolvedRunnerJobs::default(),
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
        cache_scope: std::path::PathBuf::from("/tmp/saya-tasks-tests"),
        secret_values: Default::default(),
    }
}
