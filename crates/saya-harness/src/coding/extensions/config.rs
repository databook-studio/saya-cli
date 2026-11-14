use crate::runner::{Credential, SharedCredentialSource, sandbox::RunnerSpawn};
use std::path::PathBuf;

/// Trusted application configuration. Never constructed from server/model text.
pub struct ExtensionConfig {
    pub(super) spawn: RunnerSpawn,
    #[cfg(unix)]
    pub(super) executable: PathBuf,
    #[cfg(unix)]
    pub(super) argv: Vec<String>,
    pub(super) credentials: Vec<Credential>,
    pub(super) observer: Option<std::sync::Arc<dyn super::ExtensionProcessObserver>>,
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
            #[cfg(unix)]
            executable,
            #[cfg(unix)]
            argv,
            credentials: Vec::new(),
            observer: None,
            resolver: std::sync::Arc::new(crate::runner::StaticCredentialSource::new([])),
        })
    }
    pub fn with_observer(
        mut self,
        observer: std::sync::Arc<dyn super::ExtensionProcessObserver>,
    ) -> Self {
        self.observer = Some(observer);
        self
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ExtensionError {
    #[error("invalid configured extension executable, arguments, or credentials")]
    Configuration,
    #[error("extension operation is unsupported on this platform")]
    Unsupported,
    #[error("extension process or pipe failed")]
    Process,
    /// A mandatory spawned/settled process record could not be durably
    /// published. This is deliberately distinct from the protocol exchange.
    #[error("mandatory extension process observation could not be recorded")]
    Observer,
    /// The bounded protocol exchange failed and its mandatory observer
    /// settlement also failed. No payload is retained here.
    #[error("extension exchange and mandatory observer settlement failed")]
    ExchangeAndObserver,
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

/// Preserves a primary protocol exchange while making a later mandatory
/// observer failure visible to the owner as one safe composite class.
pub(crate) fn exchange_with_cleanup(
    exchange: ExtensionError,
    cleanup: Result<(), ExtensionError>,
) -> ExtensionError {
    if matches!(cleanup, Err(ExtensionError::Observer)) {
        ExtensionError::ExchangeAndObserver
    } else {
        exchange
    }
}
