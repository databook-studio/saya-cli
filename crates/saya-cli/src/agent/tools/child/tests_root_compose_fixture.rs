//! Real-checkout fixture for the root workspace composition tests: one git
//! repository, a captured snapshot, a running store job, the agent-less
//! working checkout W, a resolved runtime, and a `NativeCodingJob` for the
//! child-id rejection pin. Built from public
//! harness/store APIs so it stays in `agent::tools::child` (the coding
//! application's fields are private to `commands::coding`).
#![cfg(any(target_os = "macos", target_os = "linux"))]

use std::{
    collections::BTreeMap, fs, num::NonZeroU64, path::Path, path::PathBuf, process::Command,
    sync::Arc, time::Duration, time::SystemTime, time::UNIX_EPOCH,
};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    budget::{CountLimits, JobBudgetLedger},
    git::{GitRepository, GitService},
    snapshot::{SnapshotArtifacts, SnapshotOmissionPolicy, SnapshotSelection},
    worktree::{CreateCheckoutInput, OwnershipInput, create_managed_checkout},
};
use saya_harness::workspace::Workspace;
use saya_store::{NewCodingJob, SqliteStateStore};
use saya_types::{
    CodingEventKind, CodingEventV1, JobId, JobState, ManagedCheckoutId, RepositoryId, SnapshotId,
};

use crate::commands::coding::{CodingApplication, CodingAuthorization, CodingJobRequest};
use crate::{GlobalOptions, load_with_sources};

pub(crate) struct Fixture {
    root: PathBuf,
    pub(crate) store: SqliteStateStore,
    pub(crate) service: GitService,
    pub(crate) repository: GitRepository,
    pub(crate) artifacts: SnapshotArtifacts,
    pub(crate) repository_id: RepositoryId,
    pub(crate) snapshot_id: SnapshotId,
    pub(crate) job: JobId,
    pub(crate) checkout: ManagedCheckoutId,
    pub(crate) checkout_path: PathBuf,
    pub(crate) runtime: crate::config::runtime::RuntimeConfig,
    pub(crate) delegate_job: Arc<crate::commands::coding::NativeCodingJob>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git_run(git: &Path, repo: &Path, args: &[&str]) {
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(repo)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn running_event(job: &JobId) -> CodingEventV1 {
    CodingEventV1::new(
        NonZeroU64::new(1).unwrap(),
        job.clone(),
        None,
        None,
        CodingEventKind::StateChanged {
            state: JobState::Running,
        },
    )
    .unwrap()
}

/// Sets `path` to owner-private 0700. The fixture module is gated to macOS/Linux
/// (both `unix`), so the `PermissionsExt` import needs no separate cfg guard.
fn private(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}

impl Fixture {
    /// One fixture: a real repo with a committed file, a captured snapshot, a
    /// running job, the agent-less working checkout W, a resolved runtime, and
    /// a `NativeCodingJob` whose `validate_delegate_tasks` pins the
    /// "coordinator" child-id rejection.
    pub(crate) async fn new(label: &str, write_paths: &[&str]) -> Self {
        Self::with_directory_name(label, write_paths, &format!("saya-root-compose-{label}")).await
    }

    /// The same fixture under a chosen directory name, so a test can put the
    /// checkout root outside a host facility's safe text class.
    pub(crate) async fn with_directory_name(
        label: &str,
        write_paths: &[&str],
        directory: &str,
    ) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{directory}-{}-{stamp}", std::process::id()));
        fs::create_dir(&root).unwrap();
        private(&root);
        let root = root.canonicalize().unwrap();
        let repo = root.join("repo");
        let program_dir = root.join("programs");
        let harness_state = root.join("harness-state");
        fs::create_dir(&repo).unwrap();
        fs::create_dir(&program_dir).unwrap();
        fs::create_dir(&harness_state).unwrap();
        private(&harness_state);
        let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .find_map(|path| path.join("git").canonicalize().ok())
            .expect("git on PATH");
        for args in [
            &["init", "--quiet"][..],
            &["config", "user.name", "root compose proof"][..],
            &["config", "user.email", "root-compose@example.invalid"][..],
        ] {
            git_run(&git, &repo, args);
        }
        fs::write(repo.join("tracked.txt"), b"original\n").unwrap();
        git_run(&git, &repo, &["add", "tracked.txt"]);
        git_run(&git, &repo, &["commit", "--quiet", "-m", "baseline"]);

        let cancellation = CancellationToken::new();
        let service = GitService::new(&git, &harness_state).unwrap();
        let repository = service.open_repository(&repo, &cancellation).await.unwrap();
        let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
        let job = JobId::parse(&format!("root-compose-{label}")).unwrap();
        let snapshot_id = SnapshotId::parse(job.as_str()).unwrap();
        let plan = service
            .plan_snapshot(
                &repository,
                snapshot_id.clone(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await
            .unwrap();
        let artifacts = SnapshotArtifacts::open(&root.join("artifacts")).unwrap();
        let workspace = Workspace::open(repository.root()).unwrap();
        let _captured = service
            .capture_snapshot(&repository, &workspace, &plan, &artifacts, &cancellation)
            .await
            .unwrap();
        let base_commit = plan.base_commit().clone();
        let store = SqliteStateStore::new(root.join("state.sqlite3"));
        let identity = service
            .inventory_repository(&repository, &cancellation)
            .await
            .unwrap();
        let repository_id = identity.identity().repository_id().clone();
        store
            .create_coding_job(NewCodingJob {
                job_id: job.clone(),
                repository_id: repository_id.clone(),
                snapshot_id: plan.snapshot_id().clone(),
            })
            .await
            .unwrap();
        store
            .append_coding_event(running_event(&job))
            .await
            .unwrap();
        let ledger = JobBudgetLedger::new(
            store.clone(),
            job.clone(),
            CountLimits {
                logical_requests: 8,
                transport_attempts: 8,
                tool_calls: 8,
                model_slots: 4,
                child_slots: 4,
                deadline: Duration::from_secs(300),
            },
        )
        .await
        .unwrap();
        let checkout =
            ManagedCheckoutId::parse(&format!("work-{}{}", label.to_lowercase(), "0".repeat(20)))
                .unwrap();
        let created = create_managed_checkout(
            &CreateCheckoutInput {
                store: &store,
                sequencer: &ledger.sequencer(),
                service: &service,
                repository: &repository,
                artifacts: &artifacts,
                plan: &plan,
                job_id: &job,
                agent_id: None,
                checkout_id: &checkout,
                base_commit: &base_commit,
            },
            &cancellation,
        )
        .await
        .unwrap();
        // W's filesystem path: the agent-less working checkout root the root
        // toolset is composed over, so tests can assert file existence there.
        let checkout_path = PathBuf::from(created.path().as_str());

        let runtime =
            load_with_sources(&GlobalOptions::default(), &repo, &root, BTreeMap::new()).unwrap();

        // A `NativeCodingJob` for the child-id rejection pin. The rejection is
        // mode-independent, so the configured mode (the resolved default) is
        // carried into the request exactly as the start path does and we never
        // read the (private) application runtime afterwards.
        let application = CodingApplication::open(
            runtime.clone(),
            store.clone(),
            &repo,
            &root.join("app-state"),
            &git,
            &program_dir,
            &cancellation,
        )
        .await
        .unwrap();
        let delegate_id = JobId::parse(&format!("root-compose-delegate-{label}")).unwrap();
        let request = CodingJobRequest::new(
            delegate_id,
            "pin the coordinator child-id rejection".into(),
            write_paths.iter().map(ToString::to_string).collect(),
        )
        .unwrap();
        let approval =
            CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
        let delegate_job = application
            .create(request, &approval, CancellationToken::new())
            .await
            .unwrap();

        Self {
            root,
            store,
            service,
            repository,
            artifacts,
            repository_id,
            snapshot_id,
            job,
            checkout,
            checkout_path,
            runtime,
            delegate_job,
        }
    }

    pub(crate) fn ownership(&self) -> OwnershipInput<'_> {
        OwnershipInput {
            store: &self.store,
            service: &self.service,
            repository: &self.repository,
            artifacts: &self.artifacts,
            job_id: &self.job,
            checkout_id: &self.checkout,
        }
    }
}
