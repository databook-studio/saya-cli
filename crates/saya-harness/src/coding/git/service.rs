use std::{
    fmt, fs,
    path::Path,
};

use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, ManagedCheckoutPath};
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::Instant;

static NEXT_SERVICE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

use super::{
    command::{GitError, GitOperation, GitRepository, require_success},
    environment::GitEnvironment,
    process::{self, GitOutput},
    process_journal::GitProcessJournal,
    process_observer::{GitProcessObserver, GitProcessSnapshot},
    repository_check,
    request::GitRequest,
};

pub struct GitService {
    pub(super) environment: GitEnvironment,
    pub(super) observer: std::sync::Arc<GitProcessObserver>,
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
            let environment = GitEnvironment::new(executable, state_root.as_ref())?;
            let identity = format!(
                "git-service-{}-{}",
                std::process::id(),
                NEXT_SERVICE_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            );
            let (journal, journal_state) = GitProcessJournal::open(&environment.root, &identity)
                .map_err(|_| GitError::InvalidStateRoot)?;
            Ok(Self {
                environment,
                observer: GitProcessObserver::new(identity, journal, journal_state),
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

    pub fn process_observations(&self) -> GitProcessSnapshot {
        self.observer.snapshot()
    }

    pub async fn git_version(&self, cancellation: &CancellationToken) -> Result<String, GitError> {
        let _guard = self.operation_guard().await;
        self.read_version(cancellation).await
    }

    async fn read_version(&self, cancellation: &CancellationToken) -> Result<String, GitError> {
        self.read_version_with_deadline(cancellation, None).await
    }

    async fn read_version_with_deadline(
        &self,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<String, GitError> {
        let output = match deadline {
            Some(deadline) => {
                self.run_request_until(
                    &self.environment.root,
                    GitRequest::Version,
                    cancellation,
                    deadline,
                )
                .await?
            }
            None => {
                self.run_request(&self.environment.root, GitRequest::Version, cancellation)
                    .await?
            }
        };
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

    pub(super) async fn ensure_supported_until(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), GitError> {
        self.read_version_with_deadline(cancellation, Some(deadline))
            .await
            .map(|_| ())
    }

    pub(super) async fn operation_guard(&self) -> MutexGuard<'_, ()> {
        self.operation_lock.lock().await
    }

    pub(super) async fn operation_guard_until(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<MutexGuard<'_, ()>, GitError> {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(GitError::Cancelled),
            result = tokio::time::timeout_at(deadline, self.operation_guard()) => {
                result.map_err(|_| GitError::TimedOut)
            }
        }
    }

    pub(super) async fn run_request(
        &self,
        current_dir: &Path,
        request: GitRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<GitOutput, GitError> {
        process::execute(
            &self.environment,
            current_dir,
            request,
            cancellation,
            &self.observer,
        )
        .await
    }

    pub(super) async fn run_request_until(
        &self,
        current_dir: &Path,
        request: GitRequest<'_>,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<GitOutput, GitError> {
        process::execute_until(
            &self.environment,
            current_dir,
            request,
            cancellation,
            deadline,
            &self.observer,
        )
        .await
    }
}

impl Drop for GitService {
    fn drop(&mut self) {
        self.observer.close();
    }
}
