//! The root decision: a Git repository keeps today's checkout mode, a plain
//! folder opens in place, `--in-place` forces the folder even inside a Git
//! tree, and the roots an in-place baseline must never capture are refused.

use super::super::{CodingAuthorization, CodingJobRequest};
use super::{CodingApplication, CodingError};
use crate::{GlobalOptions, load_with_sources};
use saya_agent::CancellationToken;
use saya_store::SqliteStateStore;
use saya_types::JobId;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    root: PathBuf,
    repository: PathBuf,
    programs: PathBuf,
    git: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture(case: &str) -> Fixture {
    use std::os::unix::fs::PermissionsExt;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "saya-coding-root-{case}-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).unwrap();
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
    let root = root.canonicalize().unwrap();
    let repository = root.join("folder");
    fs::create_dir(&repository).unwrap();
    let programs = root.join("programs");
    fs::create_dir(&programs).unwrap();
    let git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|path| path.join("git"))
        .find(|path| path.is_file())
        .expect("git is on PATH")
        .canonicalize()
        .unwrap();
    Fixture {
        root,
        repository,
        programs,
        git,
    }
}

fn git(fixture: &Fixture, args: &[&str]) {
    let output = Command::new(&fixture.git)
        .args(["-c", "core.hooksPath=/dev/null", "-C"])
        .arg(&fixture.repository)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git fixture: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn open(
    fixture: &Fixture,
    mode: crate::cli::RootMode,
) -> Result<std::sync::Arc<CodingApplication>, CodingError> {
    let runtime = load_with_sources(
        &GlobalOptions::default(),
        &fixture.repository,
        &fixture.root,
        BTreeMap::new(),
    )
    .unwrap();
    let store = SqliteStateStore::new(fixture.root.join("state.sqlite3"));
    CodingApplication::open_with_mode(
        runtime,
        store,
        &fixture.repository,
        &fixture.root.join("state"),
        &fixture.git,
        &fixture.programs,
        mode,
        &CancellationToken::new(),
    )
    .await
}

#[tokio::test]
async fn plain_folder_opens_in_place() {
    let fixture = fixture("plain");
    let application = open(&fixture, crate::cli::RootMode::Managed).await.unwrap();
    assert!(application.is_in_place());
    assert_eq!(application.root_path(), fixture.repository);
    assert!(application.local_root().is_some());
    assert!(application.git_repository("test").is_err());
}

#[tokio::test]
async fn git_repo_still_uses_checkout() {
    let fixture = fixture("git");
    git(&fixture, &["init", "--quiet"]);
    git(&fixture, &["config", "user.name", "Root proof"]);
    git(&fixture, &["config", "user.email", "root@example.invalid"]);
    fs::write(fixture.repository.join("tracked.txt"), b"original\n").unwrap();
    git(&fixture, &["add", "tracked.txt"]);
    git(&fixture, &["commit", "--quiet", "-m", "baseline"]);
    let application = open(&fixture, crate::cli::RootMode::Managed).await.unwrap();
    assert!(!application.is_in_place());
    assert!(application.local_root().is_none());
    assert!(application.git_repository("test").is_ok());
}

#[tokio::test]
async fn in_place_flag_on_git_repo() {
    let fixture = fixture("forced");
    git(&fixture, &["init", "--quiet"]);
    let application = open(&fixture, crate::cli::RootMode::InPlace).await.unwrap();
    assert!(application.is_in_place());
    assert_eq!(application.root_path(), fixture.repository);
}

#[tokio::test]
async fn dangerous_root_refused() {
    let fixture = fixture("dangerous");
    let state_root = fixture.root.join("state");
    fs::create_dir(&state_root).unwrap();
    assert!(matches!(
        crate::commands::coding::host_root::refuse_dangerous_root(Path::new("/"), &state_root),
        Err(CodingError::InPlaceUnsupportedRoot { .. })
    ));
    let home = crate::commands::coding::host_root::home_directory().expect("HOME is set");
    assert!(matches!(
        crate::commands::coding::host_root::refuse_dangerous_root(&home, &state_root),
        Err(CodingError::InPlaceUnsupportedRoot { .. })
    ));
    assert!(matches!(
        crate::commands::coding::host_root::refuse_dangerous_root(&state_root, &state_root),
        Err(CodingError::InPlaceUnsupportedRoot { .. })
    ));
}

#[tokio::test]
async fn preferences_use_local_path() {
    let fixture = fixture("preferences");
    let application = open(&fixture, crate::cli::RootMode::Managed).await.unwrap();
    let request = CodingJobRequest::new(
        JobId::parse("job-preferences-local").unwrap(),
        "inspect only".into(),
        vec![],
    )
    .unwrap();
    let approval =
        CodingAuthorization::scoped(&application.propose(&request).await.unwrap()).unwrap();
    let job = application
        .create(request, &approval, CancellationToken::new())
        .await
        .unwrap();
    let canonical = fixture.repository.canonicalize().unwrap();
    let from_root = crate::commands::preferences::worktree_id_for(application.root_path()).unwrap();
    assert_eq!(
        from_root,
        crate::commands::preferences::worktree_id_for(&canonical).unwrap()
    );
    assert_eq!(
        application
            .continuation_request(job.job_id())
            .unwrap()
            .job_id(),
        job.job_id()
    );
}

#[tokio::test]
async fn state_overlap_refused() {
    let fixture = fixture("overlap");
    let state_root = fixture.repository.join("state");
    fs::create_dir(&state_root).unwrap();
    let canonical = fs::canonicalize(&fixture.repository).unwrap();
    let canonical_state = fs::canonicalize(&state_root).unwrap();
    assert!(matches!(
        super::resolve_in_place(&fixture.repository, &canonical, &canonical_state),
        Err(CodingError::InPlaceRoot { .. })
    ));
}
