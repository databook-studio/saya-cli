//! E2-4b H1: the headless REPL keeps no last query and no capture, so the
//! receipt actions are explicitly UNAVAILABLE there, never synthesized. The
//! proof runs each receipt command through the REAL per-turn entry
//! (`handle_line_verbatim` — the same parse → apply → `action_outcome` →
//! `emit_action` chain the piped loop and `--turn-file` run) and asserts the
//! turn ends `Errored`, the outcome half pinned for `/report` alone by
//! `report_outcome_tests` (session_loop.rs:1037). The refusal TEXT itself is
//! emitted by `emit_action`'s own private `print!` (session_emit.rs:130),
//! which no in-process seam captures — asserting its bytes would need a
//! production seam, so the outcome through the real entry is the proof here.

use super::*;
use saya_agent::ApprovalPolicy;
use std::path::PathBuf;

use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs, ResolvedMemory,
    ResolvedRunnerJobs, ThemeChoice,
};

/// The explicit runtime every interactive test module builds (the
/// `session_runtime_question_tests` pattern): no profile is configured, so
/// the receipt refusals must fire before any connector is ever built.
fn receipt_runtime() -> RuntimeConfig {
    RuntimeConfig {
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
            provenance: Default::default(),
            endpoints: Default::default(),
        },
        connections: Default::default(),
        config_path: None,
        connections_path: None,
        cache_scope: PathBuf::from("/tmp/saya-h1-receipts"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// Each receipt command, and the save-without-SQL investigation refusal
/// ("the headless REPL has no last query to save from"), ends the turn
/// errored through the real entry — never completed, never a synthesized
/// receipt. A throwaway root with no database profile: the refusals must
/// fire before any connector is ever built.
#[test]
fn headless_turn_over_each_receipt_action_ends_errored() {
    let root = std::env::temp_dir().join(format!("saya-h1-receipts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("the receipt-turn root");
    let runtime = receipt_runtime();
    let store = FsSessionStore::new(root.join("session-store"));
    let state_db = SqliteStateStore::new(PathBuf::new());
    let state = &mut SessionState::new("h1-receipts", None, "test-model");
    let session = &mut SessionRuntime::acquire(
        &runtime,
        Some(&root),
        true,
        None,
        "h1-receipts",
        ApprovalPolicy::Ask,
        &root.join("session-dirs"),
    )
    .expect("the receipt-turn session acquires");
    for line in [
        "/export out.csv",
        "/report out.md",
        "/chart out.png",
        "/explain SELECT 1",
        "/investigation save",
    ] {
        let mut ctx = TurnContext {
            state: &mut *state,
            runtime: &runtime,
            store: &store,
            state_db: &state_db,
            format: RenderFormat::Text,
            terminal: false,
            session: &mut *session,
            trusted_echo: None,
        };
        let outcome = handle_line_verbatim(line, &mut ctx).expect("the turn runs");
        assert!(
            matches!(outcome, TurnOutcome::Errored),
            "{line} must end errored — the receipt is unavailable, never synthesized"
        );
    }
    let _ = std::fs::remove_dir_all(&root);
}
