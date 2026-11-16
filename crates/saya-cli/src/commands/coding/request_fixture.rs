//! Shared fixture for the persisted-`request.json` tests: one approved job in
//! a real repository, with its private records on disk.
//!
//! Both the wire-shape tests (`request_mode_tests`) and the `[tools] allow`
//! tests (`tools_allow_tests`) need a job whose `request.json` was written by
//! the real creation path, so the bytes under test are the bytes production
//! writes rather than a hand-built shape.

use super::{CodingApplication, CodingAuthorization, CodingJobRequest, NativeCodingJob};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::JobId;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const JOB: &str = "request-mode-legacy-resume";

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) application: Arc<CodingApplication>,
    pub(super) job: Arc<NativeCodingJob>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) async fn fixture(case: &str) -> Fixture {
    fixture_with(case, |request| request).await
}

/// The same fixture, with the request shaped by the caller before it is
/// proposed and created — so a test can start a job that states a `[tools]`
/// list and read back the `request.json` the real path wrote for it.
pub(super) async fn fixture_with(
    case: &str,
    shape: impl FnOnce(CodingJobRequest) -> CodingJobRequest,
) -> Fixture {
    build(case, &[], shape).await
}

/// The same fixture, launched under a stated `--tools` list: the session's
/// effective set, which a `/start --tools` may only narrow. The list is
/// stated the way a launch states it — as the CLI layer's override — so the
/// resolved runtime the application holds is the one a session would hold.
pub(super) async fn fixture_launched_with(case: &str, tools: &[&str]) -> Fixture {
    build(case, tools, |request| request).await
}

async fn build(
    case: &str,
    tools: &[&str],
    shape: impl FnOnce(CodingJobRequest) -> CodingJobRequest,
) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-request-mode-{case}-{}-{stamp}",
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
        &["config", "user.name", "Mode proof"],
        &["config", "user.email", "mode@example.invalid"],
    ] {
        git_run(&git, &repository, args);
    }
    fs::write(repository.join("tracked.txt"), b"original\n").unwrap();
    git_run(&git, &repository, &["add", "tracked.txt"]);
    git_run(&git, &repository, &["commit", "--quiet", "-m", "baseline"]);
    let options = GlobalOptions {
        tools: tools.iter().map(|tool| (*tool).to_owned()).collect(),
        ..GlobalOptions::default()
    };
    let runtime = load_with_sources(&options, &repository, &root, BTreeMap::new()).unwrap();
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
    let request = shape(
        CodingJobRequest::new(
            JobId::parse(JOB).unwrap(),
            "resume a request written by an older binary".into(),
            vec!["tracked.txt".into()],
        )
        .unwrap(),
    );
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    Fixture {
        root,
        application,
        job,
    }
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
