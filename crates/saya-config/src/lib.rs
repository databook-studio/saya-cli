//! Configuration contracts and resolution for SAYA CLI.

mod agents;
mod bounds;
mod coding;
mod context_window;
mod diagnostics;
mod endpoints;
mod env_file;
mod error;
mod hooks;
mod input;
mod instructions;
mod jobs;
mod layers;
mod limits;
mod mcp;
mod memory;
mod model;
mod permissions;
mod postgres_env;
mod profile_env;
mod provenance;
mod resolve;
mod secret;
mod tools;
mod values;
mod web_search;

pub use agents::{EXPLORE_AGENT, GENERAL_AGENT, HelperAgent, HelperAgents, ResolvedAgents};
pub use coding::{ExtensionProfile, ExtensionProtocol, ExtensionToolEffect, ResolvedCoding};
pub use context_window::context_window_tokens;
pub use diagnostics::{EndpointDiagnostics, RedactedDiagnostics, ResolvedDiagnostics};
pub use endpoints::{MAX_ENDPOINT_STRING_CHARS, ORCHESTRATOR_ROLE, ResolvedEndpoint};
pub use env_file::parse_explicit_env_file;
pub use error::ConfigError;
pub use hooks::{
    DEFAULT_HOOK_TIMEOUT_MS, HookCommandFile, HookEntryFile, HookEvent, HooksFile,
    MAX_HOOK_ARG_BYTES, MAX_HOOK_ARGV, MAX_HOOK_COMMAND_BYTES, MAX_HOOK_TIMEOUT_MS,
    MAX_HOOKS_PER_EVENT, ResolvedHook, ResolvedHookCommand, ResolvedHooks,
};
pub use input::{CliOverrides, ResolutionInput};
pub use instructions::{
    DEFAULT_PERSONAL_INSTRUCTION_BYTES, DEFAULT_PROJECT_INSTRUCTION_BYTES,
    DEFAULT_TOTAL_INSTRUCTION_BYTES, MIN_INSTRUCTION_BYTES, PERSONAL_INSTRUCTIONS_FILE,
    PROJECT_INSTRUCTIONS_FILE, ResolvedInstructions, personal_instructions_path,
};
pub use jobs::{ResolvedFetchJobs, ResolvedHostCommands, ResolvedJobs, ResolvedSessionDeny};
pub use limits::{
    BudgetLimit, ResolvedAnsweringLimits, ResolvedJobLimits, ResolvedLimits, ResolvedRunLimits,
};
pub use mcp::{
    DEFAULT_MCP_CALL_TIMEOUT_MS, DEFAULT_MCP_STARTUP_TIMEOUT_MS, MAX_MCP_ARG_BYTES, MAX_MCP_ARGS,
    MAX_MCP_CALL_TIMEOUT_MS, MAX_MCP_COMMAND_BYTES, MAX_MCP_ENV_NAME_CHARS, MAX_MCP_ENV_VARS,
    MAX_MCP_SERVER_NAME_CHARS, MAX_MCP_SERVERS, MAX_MCP_STARTUP_TIMEOUT_MS, McpServerFile,
    McpServersFile, ResolvedMcpServer, ResolvedMcpServers, is_valid_server_name,
};
pub use memory::ResolvedMemory;
pub use model::{
    AgentFile, AgentRole, AgentsFile, ConfigFile, ConnectionsFile, EndpointFile, FetchJobsFile,
    HostCommandsFile, JobFile, JobLimitsFile, JobRootMode, JobsFile, MAX_CONFIG_FILE_BYTES,
    SessionCommandsFile, ToolsFile,
};
pub use permissions::{
    CommandDecision, CommandRule, CommandRules, MAX_COMMAND_RULE_BYTES, MAX_COMMAND_RULES,
    PermissionsFile, ResolvedPermissions, decide,
};
pub use provenance::{SettingOrigin, SettingProvenance, SettingSource, user_facing_keys};
pub use resolve::{ResolvedAi, ResolvedConfig, resolve};
pub use saya_types::{CodingBudgetMode, SecretRef};
pub use secret::{MapSecretResolver, ResolvedSecret, SecretResolver};
pub use tools::{
    DATABASE_TOOLS, GOVERNED_TOOLS, MAX_MCP_TOOL_NAME_CHARS, MCP_TOOL_PREFIX, ResolvedTools,
    ToolsAllow, ToolsAllowError, allow_list_name, canonical_tool_name, database_tool_names,
    governed_tool_names, is_database_tool, is_governed_tool, is_mcp_allow_name,
    is_mcp_tool_call_name, is_valid_mcp_tool_segment, mcp_server_of,
};
pub use values::{
    AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ReasoningEffort, ThemeChoice,
};
pub use web_search::{MAX_ENDPOINT_BYTES, ResolvedWebSearch, WebSearchBackend, WebSearchFile};
