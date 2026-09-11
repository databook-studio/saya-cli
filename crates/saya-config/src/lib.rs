//! Configuration contracts and resolution for SAYA CLI.

mod context_window;
mod diagnostics;
mod env_file;
mod error;
mod input;
mod jobs;
mod layers;
mod memory;
mod model;
mod postgres_env;
mod profile_env;
mod resolve;
mod secret;
mod values;

pub use context_window::context_window_tokens;
pub use diagnostics::{RedactedDiagnostics, ResolvedDiagnostics};
pub use env_file::parse_explicit_env_file;
pub use error::ConfigError;
pub use input::{CliOverrides, ResolutionInput};
pub use jobs::{ResolvedFetchJobs, ResolvedJobs};
pub use memory::ResolvedMemory;
pub use model::{ConfigFile, ConnectionsFile, FetchJobsFile, JobsFile};
pub use resolve::{ResolvedAi, ResolvedConfig, resolve};
pub use saya_types::SecretRef;
pub use secret::{MapSecretResolver, ResolvedSecret, SecretResolver};
pub use values::{AiProvider, ColorChoice, MemoryMode, OutputFormat, ThemeChoice};
