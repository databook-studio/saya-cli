//! Tests for the direct-mode root working checkout: a job with write paths gets
//! exactly one agent-less managed checkout at the job's base commit, named in a
//! bounded private record and reused on resume, while a job without write paths
//! creates nothing.

use super::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::{NewCodingChild, SqliteStateStore};
use saya_types::{AgentId, InvocationId, JobId};
use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

const RECORD_NAME: &str = "root-checkout.json";
const MAX_RECORD_BYTES: usize = 1024;

#[cfg(unix)]
#[tokio::test]
async fn direct_mode_creates_one_root_checkout_and_reuses_it_on_resume() {
    let fixture = fixture("direct", vec!["tracked.txt".into()]).await;
    let job_id = fixture.job.job_id().clone();
    let first = fixture
        .job
        .ensure_root_checkout()
        .await
        .unwrap()
        .expect("a direct job with write paths has a root checkout");
    assert!(first.as_str().starts_with("work-"));
    // The id is deterministic over the job id, so a resume recomputes it.
    let identity = super::create::digest(format!("{}:root", job_id.as_str()).as_bytes()).unwrap();
    assert_eq!(first.as_str(), format!("work-{}", &identity.as_str()[..32]));
    // The store holds exactly one agent-less checkout, under that id, at the
    // job's base commit.
    let page = fixture
        .store
        .list_managed_checkouts(&job_id, None, 32)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        Some(page.items[0].ownership.managed_checkout_id()),
        Some(&first)
    );
    assert!(page.items[0].ownership.agent_id().is_none());
    assert_eq!(
        page.items[0].ownership.base_commit(),
        fixture.job.plan.git().unwrap().base_commit()
    );
    // The bounded private record names the checkout.
    let bytes = fs::read(fixture.job.records.join(RECORD_NAME)).unwrap();
    assert!(bytes.len() <= MAX_RECORD_BYTES);
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], json!(1));
    assert_eq!(value["checkout_id"], json!(first.as_str()));
    // Resuming re-ensures the same checkout and creates no second one.
    let again = fixture
        .job
        .ensure_root_checkout()
        .await
        .unwrap()
        .expect("the resumed job reuses its recorded root checkout");
    assert_eq!(again, first);
    assert_eq!(
        fixture
            .store
            .list_managed_checkouts(&job_id, None, 32)
            .await
            .unwrap()
            .items
            .len(),
        1
    );
}

#[cfg(unix)]
#[tokio::test]
async fn a_job_with_no_write_paths_creates_no_root_checkout() {
    let fixture = fixture("no-paths", vec![]).await;
    let ensured = fixture.job.ensure_root_checkout().await.unwrap();
    assert_eq!(ensured, None);
    assert_no_root_checkout(&fixture).await;
}

#[cfg(unix)]
#[tokio::test]
async fn a_resume_with_the_checkout_present_but_no_record_replays_to_one_checkout() {
    let fixture = fixture("resume-no-record", vec!["tracked.txt".into()]).await;
    let job_id = fixture.job.job_id().clone();
    let first = fixture
        .job
        .ensure_root_checkout()
        .await
        .unwrap()
        .expect("a direct job with write paths has a root checkout");
    // The checkout exists and is Active; the private record is lost (e.g. a
    // partial cleanup). Resume must reuse the existing checkout, not create a
    // second one, and must re-record its id.
    fs::remove_file(fixture.job.records.join(RECORD_NAME)).unwrap();
    let again = fixture
        .job
        .ensure_root_checkout()
        .await
        .unwrap()
        .expect("the existing Active checkout is reused when the record is lost");
    assert_eq!(again, first);
    let page = fixture
        .store
        .list_managed_checkouts(&job_id, None, 32)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1, "exactly one checkout, not a duplicate");
    // The record is re-written so a further resume reads it.
    assert!(fixture.job.records.join(RECORD_NAME).exists());
}

/// COMPAT (redundancy-ledger row 32): a job an older binary wrote — its
/// `request.json` carries no coordinator_mode, and it had already delegated a
/// child before the upgrade — still resumes, and `ensure_root_checkout` creates
/// the root working checkout W that binary never made, under the existing
/// safety rules, without disturbing the child.
#[cfg(unix)]
#[tokio::test]
async fn a_resume_of_a_job_that_already_has_children_creates_its_root_checkout() {
    let fixture = fixture("resume-with-children", vec!["tracked.txt".into()]).await;
    let job_id = fixture.job.job_id().clone();
    // The bytes an older binary wrote for this job: no coordinator_mode.
    let legacy = br#"{"job_id":"root-checkout-resume-with-children","task":"exercise the root working checkout","write_paths":["tracked.txt"],"programs":[],"database_profiles":[],"database_identities":{}}"#;
    fs::write(fixture.job.records.join("request.json"), legacy).unwrap();
    // A delegated child the job already had before the upgrade.
    fixture
        .store
        .create_coding_child(NewCodingChild {
            job_id: job_id.clone(),
            agent_id: AgentId::parse("child-a").unwrap(),
            current_invocation_id: InvocationId::parse("inv-child-a").unwrap(),
        })
        .await
        .unwrap();
    // That binary never created W, so no record names one.
    assert!(!fixture.job.records.join(RECORD_NAME).exists());

    // Resuming replays the mode-less request and tolerates the existing child.
    let approval = CodingAuthorization::scoped(
        &fixture
            .job
            .application
            .propose_resume(&job_id)
            .await
            .unwrap(),
    )
    .unwrap();
    let resumed = fixture
        .job
        .application
        .resume(&job_id, &approval, CancellationToken::new())
        .await
        .expect("a job whose request has no mode resumes as a direct job");
    assert_eq!(resumed.request.job_id(), &job_id);

    // The resumed direct job creates W under the existing safety rules.
    let checkout = resumed
        .ensure_root_checkout()
        .await
        .unwrap()
        .expect("a direct job with write paths has a root checkout");
    assert!(checkout.as_str().starts_with("work-"));
    let page = fixture
        .store
        .list_managed_checkouts(&job_id, None, 32)
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1, "exactly one root checkout");
    assert!(page.items[0].ownership.agent_id().is_none());
    // The pre-existing child is untouched.
    let children = fixture
        .store
        .list_coding_children(&job_id, None, 32)
        .await
        .unwrap();
    assert_eq!(children.items.len(), 1);
    assert_eq!(children.items[0].agent_id.as_str(), "child-a");
}

async fn assert_no_root_checkout(fixture: &Fixture) {
    assert!(
        fixture
            .store
            .list_managed_checkouts(fixture.job.job_id(), None, 32)
            .await
            .unwrap()
            .items
            .is_empty()
    );
    assert!(!fixture.job.records.join(RECORD_NAME).exists());
}

/// Shared with the sibling test modules that need a real direct-mode job with
/// its working checkout, the same way the delegation fixtures are shared.
pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) store: SqliteStateStore,
    pub(super) job: Arc<NativeCodingJob>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) async fn fixture(case: &str, write_paths: Vec<String>) -> Fixture {
    fixture_with(case, write_paths, false).await
}

/// The same fixture, started plan-first: the job carries a closed plan gate,
/// which an operator answer opens. The fixture's operator approves, so the
/// job's own start approval is the only gate this test shape needs.
pub(super) async fn plan_fixture(case: &str, write_paths: Vec<String>) -> Fixture {
    fixture_with(case, write_paths, true).await
}

struct ApprovingOperator;

#[async_trait::async_trait]
impl crate::agent::tools::consent::Operator for ApprovingOperator {
    async fn ask(&self, _subject: &str, _arguments: &serde_json::Value) -> bool {
        true
    }
}

async fn fixture_with(case: &str, write_paths: Vec<String>, plan: bool) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-root-checkout-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = root.join("repo");
    fs::create_dir(&repository).unwrap();
    fs::create_dir(root.join("programs")).unwrap();
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .unwrap()
        .canonicalize()
        .unwrap();
    for args in [
        &["init", "--quiet"][..],
        &["config", "user.name", "Root checkout proof"],
        &["config", "user.email", "root-checkout@example.invalid"],
    ] {
        git_run(&git, &repository, args);
    }
    fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
    git_run(&git, &repository, &["add", "tracked.txt"]);
    git_run(&git, &repository, &["commit", "--quiet", "-m", "baseline"]);
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        &repository,
        &root,
        BTreeMap::new(),
    )
    .unwrap();
    let store = SqliteStateStore::new(root.join("state.sqlite3"));
    let application = CodingApplication::open(
        runtime,
        store.clone(),
        &repository,
        &root.join("state"),
        &git,
        &root.join("programs"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let application = if plan {
        application.with_operator(Arc::new(ApprovingOperator))
    } else {
        application
    };
    let request = CodingJobRequest::new(
        JobId::parse(&format!("root-checkout-{case}")).unwrap(),
        "exercise the root working checkout".into(),
        write_paths,
    )
    .unwrap()
    .with_plan(plan)
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    Fixture { root, store, job }
}

fn git_run(git: &Path, repository: &Path, args: &[&str]) {
    let output = Command::new(git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
