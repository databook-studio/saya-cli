//! Configuration contracts and resolution for SAYA CLI.

mod diagnostics;
mod env_file;
mod error;
mod input;
mod layers;
mod model;
mod postgres_env;
mod profile_env;
mod resolve;
mod secret;
mod values;

pub use diagnostics::{RedactedDiagnostics, ResolvedDiagnostics};
pub use env_file::parse_explicit_env_file;
pub use error::ConfigError;
pub use input::{CliOverrides, ResolutionInput};
pub use model::{ConfigFile, ConnectionsFile};
pub use resolve::{ResolvedConfig, resolve};
pub use saya_types::SecretRef;
pub use secret::{MapSecretResolver, ResolvedSecret, SecretResolver};
pub use values::{AiProvider, ColorChoice, OutputFormat};
