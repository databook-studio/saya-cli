use super::CodingError;
use super::host_root::{resolve_in_place, resolve_session_root};
use crate::agent::tools::consent::Operator;
use crate::agent::tools::shell_policy::ModeCell;
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

pub(super) mod mode;
mod private;
mod root;
mod workspace;
use private::io_context;
pub(super) use private::private_directory;
use root::CodingRoot;

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
    /// Whether this surface can obtain a per-call approval at all: the fact the
    /// tool advertisement reads, never the stdio one. `true` — every surface
    /// that states nothing keeps today's advertisement — unless the surface
    /// composing this application says otherwise: a chat whose front has nobody
    /// to ask cannot reach the calls a write-shaped tool would put to a person.
    /// A cell rather than a plain flag because the surface states it through the
    /// shared host it already holds — the same way it states its mode.
    pub(super) can_obtain_approval: std::sync::atomic::AtomicBool,
    /// Whether a run of this host inherits the job's continuation context: the
    /// block that states what a *continuing* run already knows. `true` for
    /// every surface whose job outlives its turn, and stated `false` by a
    /// one-turn composition, whose run answers one question and inherits
    /// nothing — a question asked twice must send the provider the same bytes.
    pub(super) continuation_context: std::sync::atomic::AtomicBool,
    /// The job's one permission-mode cell: the root's `shell` reads it on every
    /// call, so the mode stated here is the mode the next call resolves. Shared
    /// by handle, so a host two `Arc`s hold still answers with its current mode.
    /// `Ask` — the fail-safe default — unless the building surface states one.
    pub(super) mode: ModeCell,
    /// The grants the launching surface stated for this session (`--allow`): a
    /// seed pre-answers the call the mode would have asked about, so the family
    /// it covers is advertised. A cell because the surface states it through the
    /// shared host, like the mode; empty unless a surface states one, which is
    /// every job surface.
    pub(super) launch_grants: std::sync::Mutex<saya_agent::SessionGrants>,
    /// The current job's lifecycle timeline. It starts at application open so
    /// the proposal and approval phases are measured; `create` takes it into
    /// the job and leaves a fresh one for the next job.
    pub(super) lifecycle: std::sync::Mutex<Arc<super::lifecycle::LifecycleTimeline>>,
    /// Whether this host's runs have a workspace at all: whether the folder they
    /// were opened over is one they may read and search. `true` — every surface
    /// whose folder can root an in-place job — and `false` only where a session
    /// states it: a run over a folder that cannot root a job (the home
    /// directory, a dangerous root, a folder that would hold the job's private
    /// state root) is offered no file, shell or write tool.
    pub(super) workspace: std::sync::atomic::AtomicBool,
    /// Whether this host composes one-turn runs: one question, one turn, one
    /// answer, and no outcome view recorded afterwards. `false` for every
    /// surface whose job outlives its turn.
    pub(super) one_turn: std::sync::atomic::AtomicBool,
    /// The session this application's job is a job of, when the surface opening
    /// it states one: the folder's lease names that session, so a job it opens
    /// over its own folder is admitted alongside its run instead of refused.
    pub(super) session_parent: std::sync::Mutex<Option<String>>,
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
    /// is not a repository at all. `NoWorkspace` is a session over a folder that
    /// cannot root an in-place job: the folder is the run's label and its own
    /// identity, nothing is captured for it, and the run it composes is offered
    /// no workspace tool.
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
        let canonical = fs::canonicalize(repository).map_err(|error| {
            CodingError::owner(io_context("resolve repository", repository, error))
        })?;
        let state_root = private_directory(state_root)?;
        // A run over a folder that cannot root an in-place job keeps its private
        // state where it was configured even when that is inside the folder:
        // the folder is only the run's label there, and no file, shell or write
        // tool is composed over it, so nothing can reach the state.
        if mode != RootMode::NoWorkspace && state_root.starts_with(&canonical) {
            return Err(CodingError::StateDirectory);
        }
        let service = GitService::new(git, &state_root).map_err(CodingError::owner)?;
        let root = match mode {
            RootMode::InPlace => {
                CodingRoot::InPlace(resolve_in_place(repository, &canonical, &state_root)?)
            }
            RootMode::NoWorkspace => {
                CodingRoot::InPlace(resolve_session_root(repository, &state_root)?)
            }
            RootMode::Managed => match service.open_repository(&canonical, token).await {
                Ok(repository) => CodingRoot::Git(repository),
                Err(GitError::NotRepository) => {
                    CodingRoot::InPlace(resolve_in_place(repository, &canonical, &state_root)?)
                }
                Err(error) => return Err(CodingError::owner(error)),
            },
        };
        let artifacts = SnapshotArtifacts::open(&private_directory(&state_root.join("snapshots"))?)
            .map_err(CodingError::owner)?;
        let program_dir = fs::canonicalize(program_dir).map_err(|error| {
            CodingError::owner(io_context("resolve program directory", program_dir, error))
        })?;
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
            can_obtain_approval: std::sync::atomic::AtomicBool::new(true),
            continuation_context: std::sync::atomic::AtomicBool::new(true),
            mode: ModeCell::new(ApprovalPolicy::Ask),
            launch_grants: std::sync::Mutex::new(saya_agent::SessionGrants::default()),
            lifecycle: std::sync::Mutex::new(super::lifecycle::LifecycleTimeline::new()),
            workspace: std::sync::atomic::AtomicBool::new(mode != RootMode::NoWorkspace),
            one_turn: std::sync::atomic::AtomicBool::new(false),
            session_parent: std::sync::Mutex::new(None),
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
    /// The private state root this application's runs record under, and where
    /// an in-place folder's lease is looked for.
    pub(crate) fn state_root(&self) -> &Path {
        &self.state_root
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
    pub fn store(&self) -> &SqliteStateStore {
        &self.store
    }
    /// Whether the artifacts root already holds a snapshot under `id`: a job id
    /// a run has taken. A session that opens a run again — a resumed session's
    /// next process — would otherwise be refused by the artifacts root for
    /// naming the snapshot the run before it wrote.
    pub(crate) fn snapshot_taken(&self, id: &str) -> bool {
        let Ok(id) = saya_types::SnapshotId::parse(id) else {
            // An id the snapshot grammar refuses is `create`'s to refuse, with
            // its own error: this method only reports what is already there.
            return false;
        };
        // Anything but "no snapshot under that id" is treated as taken: a
        // second run under the same id is the failure worth avoiding, and an
        // unreadable snapshot is not evidence that the id is free.
        !matches!(
            self.artifacts.read_snapshot(&id),
            Err(saya_harness::coding::snapshot::SnapshotError::SnapshotNotFound)
        )
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

    /// The same session over a state root the surface stated: a one-shot ask
    /// keeps its job's records outside the folder it asks about, so it names the
    /// root they go under instead of taking the configured one.
    pub(crate) async fn session_over(
        runtime: RuntimeConfig,
        store: SqliteStateStore,
        repository: &Path,
        mode: RootMode,
        state_root: &Path,
        token: &CancellationToken,
    ) -> Result<Arc<Self>, CodingError> {
        let (root, program_dir) = super::root_mode::session_inputs_over(&runtime, state_root)?;
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
