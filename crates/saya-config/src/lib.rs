//! Configuration contracts and resolution for SAYA CLI.

mod context_window;
mod diagnostics;
mod endpoints;
mod env_file;
mod error;
mod input;
mod instructions;
mod investigation_budget;
mod jobs;
mod layers;
mod memory;
mod model;
mod postgres_env;
mod profile_env;
mod provenance;
mod resolve;
mod secret;
mod values;

pub use context_window::context_window_tokens;
pub use diagnostics::{EndpointDiagnostics, RedactedDiagnostics, ResolvedDiagnostics};
pub use endpoints::{MAX_ENDPOINT_STRING_CHARS, ORCHESTRATOR_ROLE, ResolvedEndpoint};
pub use env_file::parse_explicit_env_file;
pub use error::ConfigError;
pub use input::{CliOverrides, ResolutionInput};
pub use instructions::{
    DEFAULT_PERSONAL_INSTRUCTION_BYTES, DEFAULT_PROJECT_INSTRUCTION_BYTES,
    DEFAULT_TOTAL_INSTRUCTION_BYTES, MIN_INSTRUCTION_BYTES, PERSONAL_INSTRUCTIONS_FILE,
    PROJECT_INSTRUCTIONS_FILE, ResolvedInstructions, personal_instructions_path,
};
pub use investigation_budget::{BudgetLimit, ResolvedInvestigationBudgets};
pub use jobs::{
    ResolvedFetchJobs, ResolvedHostCommands, ResolvedInterpreterJobs, ResolvedJobs,
    ResolvedRunnerJobs, ResolvedSessionDeny,
};
pub use memory::ResolvedMemory;
pub use model::{
    ConfigFile, ConnectionsFile, EndpointFile, FetchJobsFile, HostCommandsFile, JobsFile,
    SessionCommandsFile,
};
pub use provenance::{SettingOrigin, SettingProvenance, SettingSource, user_facing_keys};
pub use resolve::{ResolvedAi, ResolvedConfig, resolve};
pub use saya_types::SecretRef;
pub use secret::{MapSecretResolver, ResolvedSecret, SecretResolver};
pub use values::{AiProvider, ColorChoice, CompactionMode, MemoryMode, OutputFormat, ThemeChoice};
