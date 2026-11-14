use super::CodingError;
use super::host_root::resolve_in_place;
use crate::agent::tools::consent::Operator;
use crate::cli::RootMode;
use crate::config::runtime::RuntimeConfig;
use saya_agent::{ApprovalPolicy, CancellationToken};
use saya_harness::coding::{
    git::{GitError, GitRepository, GitService},
    local::LocalRoot,
    snapshot::SnapshotArtifacts,
};
use saya_store::SqliteStateStore;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The one root a coding application works over: a Git repository's working
/// tree (today's managed-checkout mode) or a plain folder with a captured
/// local baseline. The variant decides every Git-only operation; an in-place
/// root refuses it rather than silently creating a checkout.
pub(super) enum CodingRoot {
    Git(GitRepository),
    InPlace(LocalRoot),
}

impl CodingRoot {
    fn path(&self) -> &Path {
        match self {
            Self::Git(repository) => repository.root(),
            Self::InPlace(local) => local.path(),
        }
    }

    fn git(&self) -> Option<&GitRepository> {
        match self {
            Self::Git(repository) => Some(repository),
            Self::InPlace(_) => None,
        }
    }

    fn local(&self) -> Option<&LocalRoot> {
        match self {
            Self::Git(_) => None,
            Self::InPlace(local) => Some(local),
        }
    }
}

/// Trusted host composition. Program directory and state are selected by the
/// user/application, never by repository contents or model tool arguments.
pub struct CodingApplication {
    pub(super) runtime: RuntimeConfig,
    pub(super) store: SqliteStateStore,
    pub(super) service: GitService,
    pub(super) root: CodingRoot,
    pub(super) artifacts: SnapshotArtifacts,
    pub(super) state_root: PathBuf,
    pub(super) git: PathBuf,
    pub(super) program_dir: PathBuf,
    /// The managed-only refusal a start fell back over, when it did.
    pub(super) managed_fallback: Option<&'static str>,
    pub(super) personal_instruction_dir: Option<PathBuf>,
    /// The operator who answers an egressing call — a `shell` run outside the
    /// sandbox and a `web_fetch`, the root's and a helper's — when
    /// one is present. Absent by default: a headless run creates jobs whose
    /// host runs fall back to the sandbox under `ask` and whose fetches are
    /// refused, rather than either being put to anyone.
    pub(super) operator: Option<Arc<dyn Operator>>,
    /// The approval mode the root's `shell` and `web_fetch` read when they
    /// decide whether a call must be put to the operator. `Ask` — the
    /// fail-safe default — unless the building surface states the live mode.
    pub(super) approval_policy: ApprovalPolicy,
    /// The current job's lifecycle timeline. It starts at application open so
    /// the proposal and approval phases are measured; `create` takes it into
    /// the job and leaves a fresh one for the next job.
    pub(super) lifecycle: std::sync::Mutex<Arc<super::lifecycle::LifecycleTimeline>>,
}
impl CodingApplication {
    /// Opens the application over `repository`, falling back to an in-place
    /// root when the path is a plain folder. A Git repository keeps today's
    /// checkout mode; any Git failure other than "not a repository" (a bare
    /// repository, a risky local config, a changed identity) is refused rather
    /// than silently treated as a plain folder.
    pub async fn open(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        Self::open_with_mode(
            runtime,
            store,
            repository,
            state_root,
            git,
            program_dir,
            RootMode::Managed,
            token,
        )
        .await
    }

    /// Opens the application in place over the stated folder, even when it
    /// sits inside a Git working tree.
    pub async fn open_in_place(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        Self::open_with_mode(
            runtime,
            store,
            repository,
            state_root,
            git,
            program_dir,
            RootMode::InPlace,
            token,
        )
        .await
    }

    /// Opens the application over `mode`. `InPlace` never asks Git to open the
    /// repository; `Managed` falls back to an in-place root only when the path
    /// is not a repository at all.
    #[allow(clippy::too_many_arguments)]
    pub async fn open_with_mode(
        mut runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        mode: RootMode,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        super::database::bind_local_profiles(&mut runtime)?;
        let canonical = fs::canonicalize(repository).map_err(CodingError::owner)?;
        let state_root = private_directory(state_root)?;
        if state_root.starts_with(&canonical) {
            return Err(CodingError::StateDirectory);
        }
        let service = GitService::new(git, &state_root).map_err(CodingError::owner)?;
        let root = if mode == RootMode::InPlace {
            CodingRoot::InPlace(resolve_in_place(repository, &canonical, &state_root)?)
        } else {
            match service.open_repository(&canonical, token).await {
                Ok(repository) => CodingRoot::Git(repository),
                Err(GitError::NotRepository) => {
                    CodingRoot::InPlace(resolve_in_place(repository, &canonical, &state_root)?)
                }
                Err(error) => return Err(CodingError::owner(error)),
            }
        };
        let artifacts = SnapshotArtifacts::open(&private_directory(&state_root.join("snapshots"))?)
            .map_err(CodingError::owner)?;
        let program_dir = fs::canonicalize(program_dir).map_err(CodingError::owner)?;
        if !program_dir.is_dir() {
            return Err(CodingError::InvalidRequest);
        }
        Ok(Arc::new(Self {
            runtime,
            store,
            service,
            root,
            artifacts,
            state_root,
            git: git.to_path_buf(),
            program_dir,
            managed_fallback: None,
            personal_instruction_dir: None,
            operator: None,
            approval_policy: ApprovalPolicy::Ask,
            lifecycle: std::sync::Mutex::new(super::lifecycle::LifecycleTimeline::new()),
        }))
    }
    pub async fn open_default(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        Self::open_default_with_mode(
            runtime,
            store,
            repository,
            state_root,
            git,
            program_dir,
            RootMode::Managed,
            token,
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn open_default_with_mode(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        mode: RootMode,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let personal_instruction_dir = crate::config::sources::user_config_dir();
        Self::open_with_mode(
            runtime,
            store,
            repository,
            state_root,
            git,
            program_dir,
            mode,
            token,
        )
        .await?
        .with_personal_instruction_dir(Some(&personal_instruction_dir))
    }
    /// The canonical root this application works over: a Git working tree or
    /// an in-place folder.
    pub fn root_path(&self) -> &Path {
        self.root.path()
    }
    /// Whether this application works in place over a plain folder.
    pub fn is_in_place(&self) -> bool {
        self.root.local().is_some()
    }
    /// The root mode this application opened over, as the result and status
    /// surfaces state it.
    pub fn root_mode_name(&self) -> &'static str {
        if self.is_in_place() {
            "in_place"
        } else {
            "managed"
        }
    }
    /// The Git repository behind a Git-mode root, or the typed refusal naming
    /// the operation that needs one.
    pub(crate) fn git_repository(
        &self,
        operation: &'static str,
    ) -> Result<&GitRepository, CodingError> {
        self.root
            .git()
            .ok_or(CodingError::InPlaceRequiresGit { operation })
    }
    pub(crate) fn local_root(&self) -> Option<&LocalRoot> {
        self.root.local()
    }
    pub fn with_personal_instruction_dir(
        mut self: Arc<Self>,
        path: Option<&Path>,
    ) -> Result<Arc<Self>, CodingError> {
        let application = Arc::get_mut(&mut self).ok_or_else(|| {
            CodingError::owner(std::io::Error::other(
                "personal instruction path must be selected before host publication",
            ))
        })?;
        application.personal_instruction_dir = path.map(Path::to_path_buf);
        Ok(self)
    }
    /// Attaches the operator that answers an egressing call — a `shell` run
    /// outside the sandbox, or a `web_fetch`. A shared host keeps the operator
    /// unset rather than mutating under another owner: the failure is
    /// fail-closed, and such a call is denied unasked.
    pub(crate) fn with_operator(mut self: Arc<Self>, operator: Arc<dyn Operator>) -> Arc<Self> {
        if let Some(application) = Arc::get_mut(&mut self) {
            application.operator = Some(operator);
        }
        self
    }
    /// States the approval mode the root's `shell` reads. Like the operator,
    /// a shared host keeps the fail-safe default rather than mutating under
    /// another owner.
    pub fn with_approval_policy(mut self: Arc<Self>, policy: ApprovalPolicy) -> Arc<Self> {
        if let Some(application) = Arc::get_mut(&mut self) {
            application.approval_policy = policy;
        }
        self
    }
    pub fn store(&self) -> &SqliteStateStore {
        &self.store
    }
}

pub(super) fn private_directory(path: &Path) -> Result<PathBuf, CodingError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        // Missing ancestors are created too (private, like the leaf): on a
        // fresh machine saya's own state directory does not exist yet. An
        // existing directory is never altered; only the leaf is then checked.
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        match builder.create(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(CodingError::owner(error)),
        }
        let metadata = fs::symlink_metadata(path).map_err(CodingError::owner)?;
        // SAFETY: geteuid reads the process identity and has no pointer arguments.
        let owner_uid = unsafe { libc::geteuid() };
        if !metadata.is_dir() || metadata.mode() & 0o077 != 0 || metadata.uid() != owner_uid {
            return Err(CodingError::StateDirectory);
        }
        fs::canonicalize(path).map_err(CodingError::owner)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(CodingError::StateDirectory)
    }
}

impl CodingApplication {
    pub(crate) async fn session(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        mode: RootMode,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let (root, program_dir) = super::root_mode::session_inputs(&runtime)?;
        Self::open_default_with_mode(
            runtime,
            store,
            repository,
            &root,
            &super::cli::find_git()?,
            &program_dir,
            mode,
            token,
        )
        .await
    }
}

#[cfg(all(test, unix))]
#[path = "host_tests.rs"]
mod host_tests;
