//! W05c2 create fixtures (authored, not executed): one real repository, a
//! planned and captured snapshot, a running store job matching the capture,
//! and the budget ledger's sequencer.

use std::{
    fs,
    num::NonZeroU64,
    path::{Path, PathBuf},
    time::Duration,
};

use saya_agent::CancellationToken;
use saya_harness::coding::budget::{CountLimits, JobBudgetLedger, JobEventSequencer};
use saya_harness::coding::git::{GitRepository, GitService};
use saya_harness::coding::snapshot::{
    CapturedSnapshot, SnapshotArtifacts, SnapshotManifest, SnapshotOmissionPolicy,
    SnapshotSelection,
};
use saya_harness::coding::worktree::CreateCheckoutInput;
use saya_harness::workspace::Workspace;
use saya_store::{ManagedCheckoutRecord, NewCodingJob, SqliteStateStore};
use saya_types::{
    CodingEventKind, CodingEventV1, GitObjectId, JobId, JobState, ManagedCheckoutId, SnapshotId,
};

use super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

/// One `create_managed_checkout` fixture: the repository, its snapshot
/// artifacts, the matching running job, and the ledger sequencer.
pub(super) struct CreateCheckoutFixture {
    pub(super) temp: TempRoot,
    pub(super) git: PathBuf,
    pub(super) repo_root: PathBuf,
    pub(super) service: GitService,
    pub(super) repository: GitRepository,
    pub(super) artifacts: SnapshotArtifacts,
    pub(super) captured: CapturedSnapshot,
    pub(super) plan: SnapshotManifest,
    pub(super) store: SqliteStateStore,
    pub(super) sequencer: JobEventSequencer,
    pub(super) job: JobId,
    pub(super) base_commit: GitObjectId,
}

pub(super) fn limits() -> CountLimits {
    CountLimits {
        logical_requests: 8,
        transport_attempts: 8,
        tool_calls: 8,
        model_slots: 4,
        child_slots: 4,
        deadline: Duration::from_secs(30),
    }
}

fn running_event(job: &JobId) -> CodingEventV1 {
    let kind = CodingEventKind::StateChanged {
        state: JobState::Running,
    };
    CodingEventV1::new(NonZeroU64::new(1).unwrap(), job.clone(), None, None, kind).unwrap()
}

fn head_commit(git: &Path, fixture_root: &Path, repository_root: &Path) -> GitObjectId {
    let output = run_git(
        git,
        fixture_root,
        git_args(repository_root, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    GitObjectId::parse(String::from_utf8(output).unwrap().trim()).unwrap()
}

impl CreateCheckoutFixture {
    /// Builds one fixture: a real repository with uncommitted working
    /// content, plan and capture through the public snapshot producer, a
    /// running job whose registered pair matches the inventory identity and
    /// the capture, and the ledger sequencer the create flow drains.
    pub(super) async fn new(label: &str) -> Self {
        let temp = TempRoot::new(label);
        let git = git_executable();
        let repo_root = repository(temp.path(), &git);
        fs::write(repo_root.join("tracked.txt"), b"modified tracked bytes\n").unwrap();
        fs::create_dir_all(repo_root.join("a")).unwrap();
        fs::write(repo_root.join("a/one.txt"), b"nested one bytes\n").unwrap();
        let service = GitService::new(&git, state_root(temp.path())).unwrap();
        let cancellation = CancellationToken::new();
        let repository = service
            .open_repository(&repo_root, &cancellation)
            .await
            .unwrap();
        let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
        let plan = service
            .plan_snapshot(
                &repository,
                SnapshotId::parse(label).unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await
            .unwrap();
        let artifacts = SnapshotArtifacts::open(&temp.path().join("artifacts")).unwrap();
        let workspace = Workspace::open(repository.root()).unwrap();
        let captured = service
            .capture_snapshot(&repository, &workspace, &plan, &artifacts, &cancellation)
            .await
            .unwrap();
        let base_commit = head_commit(&git, temp.path(), &repo_root);
        let store = SqliteStateStore::new(&temp.path().join("state.sqlite3"));
        let job = JobId::parse("job-a").unwrap();
        let identity = service
            .inventory_repository(&repository, &cancellation)
            .await
            .unwrap();
        store
            .create_coding_job(NewCodingJob {
                job_id: job.clone(),
                repository_id: identity.identity().repository_id().clone(),
                snapshot_id: plan.snapshot_id().clone(),
            })
            .await
            .unwrap();
        store
            .append_coding_event(running_event(&job))
            .await
            .unwrap();
        let ledger = JobBudgetLedger::new(store.clone(), job.clone(), limits())
            .await
            .unwrap();
        let sequencer = ledger.sequencer();
        Self {
            temp,
            git,
            repo_root,
            service,
            repository,
            artifacts,
            captured,
            plan,
            store,
            sequencer,
            job,
            base_commit,
        }
    }

    /// The durable record for `checkout_id`, if any.
    pub(super) async fn record(
        &self,
        checkout_id: &ManagedCheckoutId,
    ) -> Option<ManagedCheckoutRecord> {
        self.store
            .get_managed_checkout(&self.job, checkout_id)
            .await
            .unwrap()
    }

    /// One trimmed Git output from `root` in the fixture environment.
    pub(super) fn git_output(&self, root: &Path, args: &[&str]) -> String {
        String::from_utf8(run_git(&self.git, self.temp.path(), git_args(root, args)))
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Flips one byte of the first captured entry's stored object, keeping
    /// its length, and returns the object path plus the original bytes.
    pub(super) fn corrupt_first_object(&self) -> (PathBuf, Vec<u8>) {
        let entry = self.captured.entries().first().expect("captured entries");
        let object = self
            .temp
            .path()
            .join("artifacts")
            .join(self.plan.snapshot_id().as_str())
            .join("objects")
            .join(entry.content_sha256().as_str());
        let original = fs::read(&object).unwrap();
        let mut corrupted = original.clone();
        corrupted[0] ^= 0xFF;
        fs::write(&object, &corrupted).unwrap();
        (object, original)
    }

    /// The fixed create input for `checkout_id`, with no owning child.
    pub(super) fn input<'a>(
        &'a self,
        checkout_id: &'a ManagedCheckoutId,
    ) -> CreateCheckoutInput<'a> {
        CreateCheckoutInput {
            store: &self.store,
            sequencer: &self.sequencer,
            service: &self.service,
            repository: &self.repository,
            artifacts: &self.artifacts,
            plan: &self.plan,
            job_id: &self.job,
            agent_id: None,
            checkout_id,
            base_commit: &self.base_commit,
        }
    }

    /// The create input for `checkout_id` with an explicit base commit, for
    /// retry-mismatch proofs.
    pub(super) fn input_with_base<'a>(
        &'a self,
        checkout_id: &'a ManagedCheckoutId,
        base_commit: &'a GitObjectId,
    ) -> CreateCheckoutInput<'a> {
        CreateCheckoutInput {
            base_commit,
            ..self.input(checkout_id)
        }
    }
}
