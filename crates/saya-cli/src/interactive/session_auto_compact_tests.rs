//! The headless automatic-compaction adapter: the decision's numerator is the
//! LAST answering request's input tokens (never the summed run usage, never a
//! failed attempt's report, never an invented zero), every suppression gate
//! the TUI reads also gates the plain REPL, and a pending clarification or a
//! stop never authorises automatic work. The end-to-end firing through the
//! real REPL loop is the next packet (C4-3b); these tests pin the decision
//! and its gating.

use super::headless_auto_compact_input;
use super::settle_auto_compact;
use crate::interactive::auto_compact::should_auto_compact;
use crate::interactive::session_runtime::SessionRuntime;
use crate::interactive::session_state::SessionState;
use saya_agent::ApprovalPolicy;
use saya_agent::{AgentEvent, AgentOutput, ProviderContinuationPhase, TokenUsage, UsageCall};
use std::path::PathBuf;

use saya_config::{
    AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

/// The explicit runtime the decision reads (the `session_headless_receipt_tests`
/// pattern, so this module mounts at the interactive level like its neighbours):
/// auto mode, no profile, the stated window — a struct literal so no config,
/// env, or connection file is touched.
fn runtime_with_window(window: Option<u64>) -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
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
                context_window_tokens: window,
                show_thinking: false,
                compaction: CompactionMode::Auto,
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
        cache_scope: PathBuf::from("/tmp/saya-c43a-decision"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// A settled run output with the given events, summed usage, and salvage flag.
fn output(events: Vec<AgentEvent>, usage: TokenUsage, truncated: bool) -> AgentOutput {
    AgentOutput {
        answer: "the answer".into(),
        events,
        used_bounded_sql_query: false,
        tool_metadata: Vec::new(),
        usage,
        learning_usage: None,
        truncated,
        answer_sql: None,
    }
}

/// One answering round's usage report, as `receive` emits it mid-turn.
fn answering_report(input: u64) -> AgentEvent {
    AgentEvent::usage(UsageCall::Answer, TokenUsage::new(input, 20))
}

/// The decision table over the headless input derivation and the shared
/// predicate: row (a) fires; rows (b)–(h) each suppress for their own reason.
#[test]
fn the_headless_decision_table_matches_the_shared_predicate() {
    let runtime = runtime_with_window(Some(1000));
    let state = SessionState::new("c43a", None, "test-model");

    // (a) The last Answer usage 960 of an explicit window 1000 is 96%: fires.
    let fires = output(vec![answering_report(960)], TokenUsage::new(960, 20), false);
    let input = headless_auto_compact_input(&runtime, &state, &fires);
    assert_eq!(
        input.answering_input,
        Some(960),
        "(a) the numerator rides the input"
    );
    assert_eq!(
        input.window,
        Some(1000),
        "(a) the explicit window rides the input"
    );
    assert!(
        !input.continued && !input.terminal_stop && !input.auto_failed,
        "(a) a normally completed turn carries no suppression gate"
    );
    assert!(
        should_auto_compact(&input),
        "(a) 960 of 1000 at the last answering request must fire"
    );

    // (b) The same turn shape with a larger SUMMED usage and a smaller last
    // request: the numerator is the LAST request's input, so 400 of 1000 is
    // 40% and never fires — occupancy is not the paid total.
    let summed = output(
        vec![answering_report(400)],
        TokenUsage::new(5000, 20),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &summed);
    assert_eq!(
        input.answering_input,
        Some(400),
        "(b) the numerator is the last request's input, not the summed run usage"
    );
    assert!(
        !should_auto_compact(&input),
        "(b) 400 of 1000 (the last request) is 40%: the summed 5000 must never decide"
    );

    // (c) A failed attempt's huge report is never the numerator either: only
    // a successful Answer report measures occupancy.
    let mixed = output(
        vec![
            AgentEvent::failed_attempt_usage(TokenUsage::new(120_000, 8)),
            answering_report(400),
        ],
        TokenUsage::new(120_400, 28),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &mixed);
    assert_eq!(
        input.answering_input,
        Some(400),
        "(c) a FailedAttemptUsage never becomes the numerator"
    );
    assert!(
        !should_auto_compact(&input),
        "(c) the failed attempt's huge input must not fire"
    );

    // (d) No Answer usage event at all: the numerator is absent, not zero.
    let silent = output(
        vec![AgentEvent::failed_attempt_usage(TokenUsage::new(
            120_000, 8,
        ))],
        TokenUsage::new(120_000, 8),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &silent);
    assert_eq!(input.answering_input, None, "(d) absent is not zero");
    assert!(
        !should_auto_compact(&input),
        "(d) no answering report never fires"
    );

    // (e) Unknown window: no percentage, no fire — 96% of nothing is a fiction.
    let unknown = runtime_with_window(None);
    let input = headless_auto_compact_input(&unknown, &state, &fires);
    assert_eq!(input.window, None, "(e) the unknown window stays absent");
    assert!(
        !should_auto_compact(&input),
        "(e) an unknown window never fires, whatever the numerator"
    );

    // (f) An admitted continuation (typed Continuing, truncated false): the
    // resume anchors live in earlier tool results, so this never compacts.
    let continued = output(
        vec![
            AgentEvent::provider_continuation(ProviderContinuationPhase::Continuing, 1, 3),
            answering_report(960),
        ],
        TokenUsage::new(960, 20),
        false,
    );
    let input = headless_auto_compact_input(&runtime, &state, &continued);
    assert!(
        input.continued,
        "(f) the typed transition is the continued gate"
    );
    assert!(
        !input.terminal_stop,
        "(f) a continuation is never read as a salvage stop"
    );
    assert!(
        !should_auto_compact(&input),
        "(f) a continued turn never compacts"
    );

    // (g) A budget-salvage stop: suppressed through its own gate, named apart
    // from the continuation.
    let salvage = output(vec![answering_report(960)], TokenUsage::new(960, 20), true);
    let input = headless_auto_compact_input(&runtime, &state, &salvage);
    assert!(
        input.terminal_stop,
        "(g) the salvage flag is the terminal-stop gate"
    );
    assert!(
        !input.continued,
        "(g) a salvage stop is never read as a continuation"
    );
    assert!(
        !should_auto_compact(&input),
        "(g) a salvaged turn never compacts"
    );

    // (h) A prior automatic failure this session latches the trigger off.
    let mut latched = SessionState::new("c43a-latched", None, "test-model");
    latched.auto_compact_failed = true;
    let input = headless_auto_compact_input(&runtime, &latched, &fires);
    assert!(input.auto_failed, "(h) the session latch rides the input");
    assert!(
        !should_auto_compact(&input),
        "(h) a latched session never fires again"
    );
}

/// (i) A pending clarification or a cancelled/stopped turn is never authority
/// for automatic work: `settle_auto_compact` returns `None` without running
/// the compaction — the runtime here is the unused one, so a None result
/// cannot involve any provider call — and the session is untouched.
#[test]
fn a_pending_clarification_or_a_stop_never_settles_automatic_work() {
    let root = std::env::temp_dir().join(format!("saya-c43a-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the gate-test root");
    let runtime = runtime_with_window(Some(1000));
    let mut state = SessionState::new("c43a-gate", None, "test-model");
    let session = &mut SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "c43a-gate",
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the gate-test session acquires");
    // A turn that WOULD fire on the pure predicate (960 of 1000) but ended on
    // a clarification, and the same turn stopped instead of settling.
    let would_fire = output(vec![answering_report(960)], TokenUsage::new(960, 20), false);
    assert!(
        settle_auto_compact(&runtime, &mut state, session, &would_fire, true, false).is_none(),
        "(i) a pending clarification is not authority for automatic work"
    );
    assert!(
        settle_auto_compact(&runtime, &mut state, session, &would_fire, false, true).is_none(),
        "(i) a cancelled/stopped turn never compacts merely because output exists"
    );
    assert!(
        !state.auto_compact_failed,
        "(i) a gated settlement leaves the no-retry latch alone"
    );
    let _ = std::fs::remove_dir_all(&root);
}
