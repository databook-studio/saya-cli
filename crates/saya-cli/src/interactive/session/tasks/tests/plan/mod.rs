//! Universe advertisement, the user-turn context block, and the prefix-cache
//! property of the task list.

use saya_agent::{AgentMode, ApprovalPolicy};

use crate::interactive::session::tasks::SessionTasks;
use crate::interactive::session::tasks::render::{
    TASKS_BLOCK_LABEL, TASKS_RENDER_CEILING, checklist_definition, render_tasks_block,
};

fn test_runtime() -> crate::config::runtime::RuntimeConfig {
    use saya_config::{
        AiProvider, ColorChoice, MemoryMode, OutputFormat, ResolvedAi, ResolvedConfig,
        ResolvedHostCommands, ResolvedJobs, ResolvedMemory, ThemeChoice,
    };
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
                retry_delays_ms: vec![250, 500, 1000],
                compaction: saya_config::CompactionMode::Auto,
                compaction_trigger_percent: 50,
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
#[cfg(test)]
#[path = "delta.rs"]
mod delta;
