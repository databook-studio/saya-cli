use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, ManagedCheckoutPath};
use tokio::sync::{Mutex, MutexGuard};

use super::{
    command::{GitError, GitOperation, GitRepository, require_success},
    environment::GitEnvironment,
    process::{self, GitOutput},
    repository_check,
    request::GitRequest,
};

pub struct GitService {
    pub(super) environment: GitEnvironment,
    operation_lock: Mutex<()>,
}

impl fmt::Debug for GitService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("GitService { .. }")
    }
}

impl GitService {
    /// Construct this service only at the trusted composition root with the
    /// installed Git path; never derive the executable from project or model input.
    pub fn new(
        executable: impl AsRef<Path>,
        state_root: impl AsRef<Path>,
    ) -> Result<Self, GitError> {
        #[cfg(not(unix))]
        {
            let _ = (executable, state_root);
            return Err(GitError::UnsupportedPlatform);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            let executable = executable.as_ref();
            if !executable.is_absolute() {
                return Err(GitError::InvalidGitExecutable);
            }
            let executable =
                fs::canonicalize(executable).map_err(|_| GitError::InvalidGitExecutable)?;
            let metadata = fs::metadata(&executable).map_err(|_| GitError::InvalidGitExecutable)?;
            if !metadata.is_file() || metadata.mode() & 0o111 == 0 {
                return Err(GitError::InvalidGitExecutable);
            }
            Ok(Self {
                environment: GitEnvironment::new(executable, state_root.as_ref())?,
                operation_lock: Mutex::new(()),
            })
        }
    }

    pub fn managed_path(
        &self,
        checkout_id: &ManagedCheckoutId,
    ) -> Result<ManagedCheckoutPath, GitError> {
        let path = self.environment.worktrees.join(checkout_id.as_str());
        let path = path.to_str().ok_or(GitError::InvalidManagedPath)?;
        ManagedCheckoutPath::new(path.to_owned()).map_err(|_| GitError::InvalidManagedPath)
    }

    pub async fn git_version(&self, cancellation: &CancellationToken) -> Result<String, GitError> {
        let _guard = self.operation_guard().await;
        self.read_version(cancellation).await
    }

    async fn read_version(&self, cancellation: &CancellationToken) -> Result<String, GitError> {
        let output = self
            .run_request(&self.environment.root, GitRequest::Version, cancellation)
            .await?;
        let output = require_success(output, GitOperation::Read)
            .map_err(|_| GitError::UnsupportedGitVersion)?;
        let version = repository_check::single_line(&output.stdout)?;
        if !version.starts_with("git version ") {
            return Err(GitError::InvalidGitVersion);
        }
        Ok(version)
    }

    pub async fn open_repository(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<GitRepository, GitError> {
        let _guard = self.operation_guard().await;
        self.read_version(cancellation).await?;
        repository_check::inspect(self, path, false, cancellation).await
    }

    pub(super) async fn ensure_supported(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        self.read_version(cancellation).await.map(|_| ())
    }

    pub(super) async fn operation_guard(&self) -> MutexGuard<'_, ()> {
        self.operation_lock.lock().await
    }

    pub(super) async fn run_request(
        &self,
        current_dir: &Path,
        request: GitRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<GitOutput, GitError> {
        process::execute(&self.environment, current_dir, request, cancellation).await
    }
}
