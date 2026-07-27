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
    #[error("secret reference {0} could not be resolved")]
    MissingSecret(String),
    #[error("keyring secret references are unavailable in this runtime")]
    KeyringUnavailable,
    #[error("could not read secret file: {0}")]
    SecretFile(String),
}
