//! The startup trust prompt and the bypass × trust intersection, exercised
//! through the real `permissions::trust::*` API.

use std::fs;
use std::path::{Path, PathBuf};

use saya_agent::ApprovalPolicy;
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
};

use crate::interactive::session::permissions::trust::{
    BYPASS_UNBOUND_NO_LANE, TrustAnswer, TrustPromptContext, ask_trust, bypass_no_lane_note,
    parse_trust_answer, resolve_trusted_dir, should_prompt, trust_prompt, trusted_root_line,
};
use crate::interactive::session::universe::SessionUniverse;

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

fn temp_dir(label: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-session-trust-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn session_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            hooks: Default::default(),
            web_search: Default::default(),
            mcp_servers: Default::default(),
            permissions: Default::default(),
            tools: Default::default(),
            coding: Default::default(),
            agents: Default::default(),
            instructions: Default::default(),
            profile_name: None,
            profile: None,
            limits: Default::default(),
            ai: ResolvedAi {
                app_attribution: true,
                provider: AiProvider::Ollama,
                model: "test-model".into(),
                base_url: None,
                api_key: None,
                allow_data_sharing: true,
                temperature: 0.0,
                timeout_seconds: 60,
                idle_timeout_seconds: 90,
                request_timeout_seconds: 180,
                max_output_tokens: None,
                reasoning_effort: None,
                context_byte_budget: 256 * 1024,
                context_window_tokens: None,
                show_thinking: false,
                compaction: saya_config::CompactionMode::Auto,
                compaction_trigger_percent: 50,
                retry_delays_ms: vec![250, 500, 1000],
            },
            max_rows: 100,
            read_only: true,
            candidates: 1,
            jobs: ResolvedJobs::default(),
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
        cache_scope: PathBuf::from("/tmp/saya-session-trust"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// A fresh unbound launch: no `--workspace`, no pin, outside any worktree.
fn unbound_terminal() -> TrustPromptContext {
    TrustPromptContext {
        is_terminal: true,
        fresh: true,
        has_explicit: false,
        has_pin: false,
        root_bound: false,
        turn_file: false,
    }
}
#[path = "trust_tests/bypass.rs"]
mod bypass;
#[path = "trust_tests/persistence.rs"]
mod persistence;
#[path = "trust_tests/prompt.rs"]
mod prompt;
#[path = "trust_tests/trusted_paths.rs"]
mod trusted_paths;
