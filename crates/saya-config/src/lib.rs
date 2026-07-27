//! Configuration contracts and resolution for SAYA CLI.

mod diagnostics;
mod env_file;
mod error;
mod input;
mod model;
mod resolve;
mod secret;

pub use diagnostics::{RedactedDiagnostics, ResolvedDiagnostics};
pub use env_file::parse_explicit_env_file;
pub use error::ConfigError;
pub use input::{CliOverrides, ResolutionInput};
pub use model::{AiProvider, ConfigFile, ConnectionsFile, OutputFormat};
pub use resolve::{ResolvedConfig, resolve};
pub use saya_types::SecretRef;
pub use secret::{MapSecretResolver, ResolvedSecret, SecretResolver};
