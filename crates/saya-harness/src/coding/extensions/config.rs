use crate::runner::{Credential, SharedCredentialSource, sandbox::RunnerSpawn};
use std::path::PathBuf;

/// Trusted application configuration. Never constructed from server/model text.
pub struct ExtensionConfig {
    pub(super) spawn: RunnerSpawn,
    pub(super) executable: PathBuf,
    pub(super) argv: Vec<String>,
    pub(super) credentials: Vec<Credential>,
    pub(super) resolver: SharedCredentialSource,
}
impl ExtensionConfig {
    pub fn new(
        spawn: RunnerSpawn,
        executable: PathBuf,
        argv: Vec<String>,
    ) -> Result<Self, ExtensionError> {
        let canonical = executable
            .canonicalize()
            .map_err(|_| ExtensionError::Configuration)?;
        if !executable.is_absolute()
            || canonical != executable
            || !canonical.is_file()
            || canonical.parent() != Some(spawn.program_dir())
            || argv.len() > 64
            || argv
                .iter()
                .any(|arg| arg.contains('\0') || arg.len() > 8192)
            || argv.iter().map(String::len).sum::<usize>() > 32768
        {
            return Err(ExtensionError::Configuration);
        }
        Ok(Self {
            spawn,
            executable,
            argv,
            credentials: Vec::new(),
            resolver: std::sync::Arc::new(crate::runner::StaticCredentialSource::new([])),
        })
    }
    pub fn with_credentials(
        mut self,
        credentials: Vec<Credential>,
        resolver: SharedCredentialSource,
    ) -> Result<Self, ExtensionError> {
        if credentials.len() > 16 {
            return Err(ExtensionError::Configuration);
        }
        self.credentials = credentials;
        self.resolver = resolver;
        Ok(self)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionError {
    #[error("invalid configured extension executable, arguments, or credentials")]
    Configuration,
    #[error("extension operation is unsupported on this platform")]
    Unsupported,
    #[error("extension process or pipe failed")]
    Process,
    #[error("extension protocol frame or message is invalid")]
    Protocol,
    #[error("extension response exceeded its bound")]
    Limit,
    #[error("extension operation timed out")]
    Timeout,
    #[error("extension operation was cancelled")]
    Cancelled,
    #[error("extension tool requires explicit effect approval")]
    ApprovalRequired,
    #[error("extension tool is not explicitly trusted")]
    UntrustedTool,
}
