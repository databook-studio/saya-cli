use super::CodingError;
use crate::config::runtime::RuntimeConfig;
use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::{GitRepository, GitService},
    snapshot::SnapshotArtifacts,
};
use saya_store::SqliteStateStore;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

/// Trusted host composition. Program directory and state are selected by the
/// user/application, never by repository contents or model tool arguments.
pub struct CodingApplication {
    pub(super) runtime: RuntimeConfig,
    pub(super) store: SqliteStateStore,
    pub(super) service: GitService,
    pub(super) repository: GitRepository,
    pub(super) artifacts: SnapshotArtifacts,
    pub(super) state_root: PathBuf,
    pub(super) program_dir: PathBuf,
}
impl CodingApplication {
    pub async fn open(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        state_root: &Path,
        git: &Path,
        program_dir: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let repository = fs::canonicalize(repository).map_err(CodingError::owner)?;
        let state_root = private_directory(state_root)?;
        if state_root.starts_with(&repository) {
            return Err(CodingError::StateDirectory);
        }
        let service = GitService::new(git, &state_root).map_err(CodingError::owner)?;
        let repository = service
            .open_repository(&repository, token)
            .await
            .map_err(CodingError::owner)?;
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
            repository,
            artifacts,
            state_root,
            program_dir,
        }))
    }
    pub fn store(&self) -> &SqliteStateStore {
        &self.store
    }
}

pub(super) fn private_directory(path: &Path) -> Result<PathBuf, CodingError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
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
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let database = crate::state_path::state_db_path();
        let root = database
            .parent()
            .ok_or(CodingError::StateDirectory)?
            .join("coding");
        private_directory(&root)?;
        let program_dir = runtime
            .resolved
            .jobs
            .runner
            .program_dir
            .clone()
            .unwrap_or_else(|| PathBuf::from("/usr/bin"));
        Self::open(
            runtime,
            store,
            repository,
            &root,
            &super::cli::find_git()?,
            &program_dir,
            token,
        )
        .await
    }
}
