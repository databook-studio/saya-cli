//! The application carries the operator that answers a `shell` run outside
//! the sandbox into the job it creates. The panel attaches one; a
//! headless run does not, so the job it creates has nobody to ask — and a host
//! run under `ask` falls back to the sandbox or is refused unasked rather than
//! run.
#![cfg(unix)]

use super::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use crate::agent::tools::consent::Operator;
use crate::{GlobalOptions, load_with_sources};
use async_trait::async_trait;
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::JobId;
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

/// An operator that answers nothing: the created job only has to carry it.
struct Refusing;

#[async_trait]
impl Operator for Refusing {
    async fn ask(&self, _subject: &str, _arguments: &Value) -> bool {
        false
    }
}

struct Fixture {
    root: PathBuf,
    application: Arc<CodingApplication>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// One private state root with a fresh repository and a coding runtime; the
/// operator, when given, is attached before any job is created.
async fn fixture(case: &str, operator: Option<Arc<dyn Operator>>) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-shell-operator-{case}-{}-{stamp}",
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
        &["config", "user.name", "Shell operator proof"],
        &["config", "user.email", "shell-operator@example.invalid"],
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
    let application = CodingApplication::open(
        runtime,
        SqliteStateStore::new(root.join("state.sqlite3")),
        &repository,
        &root.join("state"),
        &git,
        &root.join("programs"),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    let application = match operator {
        Some(operator) => application.with_operator(operator),
        None => application,
    };
    Fixture { root, application }
}

async fn create(fixture: &Fixture, job: &str) -> Arc<NativeCodingJob> {
    let request = CodingJobRequest::new(
        JobId::parse(job).unwrap(),
        "carry the panel's shell operator into the job".into(),
        vec!["tracked.txt".into()],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&fixture.application.propose(&request).await.unwrap()).unwrap();
    fixture
        .application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn an_application_with_an_operator_hands_it_to_the_job() {
    let fixture = fixture("present", Some(Arc::new(Refusing))).await;
    let job = create(&fixture, "shell-operator-present").await;
    assert!(job.operator().is_some());
}

#[tokio::test]
async fn an_application_without_an_operator_leaves_the_job_without_one() {
    let fixture = fixture("absent", None).await;
    let job = create(&fixture, "shell-operator-absent").await;
    assert!(job.operator().is_none());
}

/// The same operator is the surface that answers a clarification question: a
/// job created by a panel that attached one advertises the ask even with no
/// database profile selected, and answers it without a connection.
#[tokio::test]
async fn a_job_with_an_operator_advertises_the_ask_without_a_profile() {
    let fixture = fixture("ask-present", Some(Arc::new(Refusing))).await;
    let job = create(&fixture, "ask-present").await;
    let databases = job
        .databases()
        .expect("an operator surface can answer an ask");
    let definitions = databases.definitions(None);
    let advertised: Vec<&str> = definitions
        .iter()
        .map(|definition| definition.name.as_str())
        .collect();
    assert_eq!(advertised, vec!["request_clarification"]);
}

/// A headless job has nobody to answer, so it advertises no ask at all: it
/// must never stop and wait for a question no one can answer.
#[tokio::test]
async fn a_headless_job_advertises_no_ask() {
    let fixture = fixture("ask-absent", None).await;
    let job = create(&fixture, "ask-absent").await;
    assert!(
        job.databases().is_none(),
        "a headless job has no surface to answer an ask"
    );
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
