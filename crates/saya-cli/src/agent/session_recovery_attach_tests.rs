//! The attach seam's wiring tests: a real acquired session with a bound
//! workspace carries one handle — the acquired id, flag off — and the
//! per-turn tools built through `attach_recovery` capture only when the
//! session's own flag is on. An unbound universe exposes no handle and
//! attaches nothing; the one-shot `ask` path (`None`) attaches nothing.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_config::{
    AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
    ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
};
use saya_types::CheckpointState;

use super::attach_recovery;
use crate::agent::tools::DatabaseTools;
use crate::interactive::session::lifecycle::recovery::RecoveryHandle;
use crate::interactive::session::runtime::SessionRuntime;
use crate::interactive::session::universe::{SessionComposition, SessionUniverse};

fn temp_dir(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-attach-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            hooks: Default::default(),
            tools: Default::default(),
            web_search: Default::default(),
            mcp_servers: Default::default(),
            permissions: Default::default(),
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
        cache_scope: PathBuf::from("/tmp/saya-session-attach"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

/// The per-turn shape: fresh tools over the session's workspace, then the
/// attach. Exactly what `run_prompt` builds per turn.
fn attached_tools(universe: &SessionUniverse) -> DatabaseTools {
    attach_recovery(
        DatabaseTools::new(None, 100, true).with_workspace(universe.workspace()),
        Some(universe),
    )
}

#[cfg(test)]
#[path = "session_recovery_attach_tests/attachment_guards.rs"]
mod attachment_guards;
#[cfg(test)]
#[path = "session_recovery_attach_tests/flag_behavior.rs"]
mod flag_behavior;
