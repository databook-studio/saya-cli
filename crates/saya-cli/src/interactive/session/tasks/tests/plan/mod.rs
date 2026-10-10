//! Slice 3+4 red tests, part two: universe advertisement, the user-turn
//! context block, and the prefix-cache property.

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::interactive::session::tasks::SessionTasks;
use crate::interactive::session::tasks::render::{
    TASKS_BLOCK_LABEL, TASKS_RENDER_CEILING, render_tasks_block, tasks_set_definition,
};

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    use saya_config::{
        AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
        ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs,
        ResolvedMemory, ResolvedRunnerJobs, ThemeChoice,
    };
    crate::config::runtime::RuntimeConfig {
        resolved: ResolvedConfig {
            coding: Default::default(),
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
                retry_delays_ms: vec![250, 500, 1000],
                compaction: saya_config::CompactionMode::Auto,
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
        cache_scope: std::path::PathBuf::from("/tmp/saya-tasks-tests"),
        investigations_root: crate::config::runtime::temp_investigations_root(),
        secret_values: Default::default(),
    }
}

#[cfg(test)]
#[path = "advertisement.rs"]
mod advertisement;
#[cfg(test)]
#[path = "context_render.rs"]
mod context_render;
