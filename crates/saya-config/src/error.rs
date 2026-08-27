use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid TOML: {0}")]
    Parse(String),
    #[error("invalid environment value for {name}: {reason}")]
    InvalidEnvironment { name: String, reason: String },
    #[error("no connection profile was selected")]
    MissingProfile,
    #[error("connection profile {0:?} was not found")]
    UnknownProfile(String),
    #[error("database environment variable {name} is required")]
    MissingDatabaseField { name: &'static str },
    #[error("database type {0:?} is not supported in environment configuration")]
    UnsupportedDatabaseType(String),
    #[error("secret reference {0} could not be resolved")]
    MissingSecret(String),
    #[error("keyring secret references are unavailable in this runtime")]
    KeyringUnavailable,
    #[error("could not read secret file: {0}")]
    SecretFile(String),
    #[error("memory setting {field} = {value} must be within {min}..={max}")]
    MemoryRange {
        field: &'static str,
        value: u32,
        min: u32,
        max: u32,
    },
    /// A non-memory numeric setting is below its floor. Sibling to
    /// [`ConfigError::MemoryRange`] for settings that are not `[memory]` (e.g.
    /// `[ai] context_byte_budget`), which have a minimum but no useful ceiling
    /// — reporting one would mean printing `usize::MAX` at the user.
    #[error("setting {field} = {value} must be at least {min}")]
    SettingBelowMinimum {
        field: &'static str,
        value: usize,
        min: usize,
    },
}
