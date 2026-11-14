use std::{fmt, path::Path};

#[cfg(unix)]
use std::fs;

use saya_agent::CancellationToken;
use saya_types::{ManagedCheckoutId, ManagedCheckoutPath};
use tokio::sync::{Mutex, MutexGuard};
use tokio::time::Instant;

#[cfg(unix)]
static NEXT_SERVICE_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

use super::{
    command::{GitError, GitOperation, GitRepository, require_success},
    compatibility::{GitCompatibility, GitVersionInfo},
    environment::GitEnvironment,
    process::{self, GitOutput},
    process_observer::{GitProcessObserver, GitProcessSnapshot},
    repository_check,
    request::GitRequest,
};

#[cfg(unix)]
use super::process_journal::GitProcessJournal;

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
            Err(GitError::UnsupportedPlatform)
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
        let info = self.read_version(cancellation).await?;
        Ok(info.raw().to_owned())
    }

    /// The bounded `git --version` line and the compatibility paths it
    /// selects. The probe always runs so diagnostics and receipts stay
    /// truthful; internal detection caches the result.
    pub async fn git_version_info(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<GitVersionInfo, GitError> {
        let _guard = self.operation_guard().await;
        let info = self.read_version(cancellation).await?;
        self.environment.set_compatibility(info.compatibility());
        Ok(info)
    }

    /// The detected compatibility, probing once and caching it for this
    /// service (one service is created per process).
    pub async fn git_compatibility(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<GitCompatibility, GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_compatibility(cancellation).await
    }

    async fn ensure_compatibility(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<GitCompatibility, GitError> {
        if let Some(compatibility) = self.environment.cached_compatibility() {
            return Ok(compatibility);
        }
        let info = self.read_version(cancellation).await?;
        let compatibility = info.compatibility();
        self.environment.set_compatibility(compatibility);
        Ok(compatibility)
    }

    async fn ensure_compatibility_until(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<GitCompatibility, GitError> {
        if let Some(compatibility) = self.environment.cached_compatibility() {
            return Ok(compatibility);
        }
        let info = self
            .read_version_with_deadline(cancellation, Some(deadline))
            .await?;
        let compatibility = info.compatibility();
        self.environment.set_compatibility(compatibility);
        Ok(compatibility)
    }

    async fn read_version(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<GitVersionInfo, GitError> {
        self.read_version_with_deadline(cancellation, None).await
    }

    async fn read_version_with_deadline(
        &self,
        cancellation: &CancellationToken,
        deadline: Option<Instant>,
    ) -> Result<GitVersionInfo, GitError> {
        let output = match deadline {
            Some(deadline) => {
                self.run_version_request_until(cancellation, deadline)
                    .await?
            }
            None => self.run_version_request(cancellation).await?,
        };
        let output = require_success(output, GitOperation::Read)
            .map_err(|_| GitError::UnsupportedGitVersion)?;
        let version = repository_check::single_line(&output.stdout)?;
        let info = GitVersionInfo::parse(version);
        let compatibility = info.compatibility();
        // An unparseable version is conservatively the minimum; only a
        // reported version below it is refused, at this first probe.
        if compatibility.version_known()
            && compatibility.version() < GitCompatibility::minimum_supported_version()
        {
            return Err(GitError::UnsupportedGitVersionTooOld {
                found: compatibility.version(),
                minimum: GitCompatibility::minimum_supported_version(),
            });
        }
        Ok(info)
    }

    pub async fn open_repository(
        &self,
        path: &Path,
        cancellation: &CancellationToken,
    ) -> Result<GitRepository, GitError> {
        let _guard = self.operation_guard().await;
        self.ensure_compatibility(cancellation).await?;
        repository_check::inspect(self, path, false, cancellation).await
    }

    pub(super) async fn ensure_supported(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<(), GitError> {
        self.ensure_compatibility(cancellation).await.map(|_| ())
    }

    pub(super) async fn ensure_supported_until(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<(), GitError> {
        self.ensure_compatibility_until(cancellation, deadline)
            .await
            .map(|_| ())
    }

    /// The worktree listing request this Git version supports.
    pub(super) fn worktree_list_request(&self) -> GitRequest<'static> {
        if self
            .environment
            .compatibility()
            .worktree_list_zero_terminated()
        {
            GitRequest::WorktreeListPorcelainZ
        } else {
            GitRequest::WorktreeListPorcelain
        }
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

    /// The version probe itself: it cannot be guarded by the compatibility it
    /// is about to determine, and every guarded request calls it through the
    /// cache first.
    async fn run_version_request(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<GitOutput, GitError> {
        process::execute(
            &self.environment,
            &self.environment.root,
            GitRequest::Version,
            cancellation,
            &self.observer,
        )
        .await
    }

    async fn run_version_request_until(
        &self,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<GitOutput, GitError> {
        process::execute_until(
            &self.environment,
            &self.environment.root,
            GitRequest::Version,
            cancellation,
            deadline,
            &self.observer,
        )
        .await
    }

    pub(super) async fn run_request(
        &self,
        current_dir: &Path,
        request: GitRequest<'_>,
        cancellation: &CancellationToken,
    ) -> Result<GitOutput, GitError> {
        // Every real request probes the version first, so the command carries
        // `--no-lazy-fetch` whenever the installed Git supports it. The probe
        // itself cannot: its answer is what selects the option.
        if !matches!(request, GitRequest::Version) {
            self.ensure_compatibility(cancellation).await?;
        }
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
        if !matches!(request, GitRequest::Version) {
            self.ensure_compatibility_until(cancellation, deadline)
                .await?;
        }
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
