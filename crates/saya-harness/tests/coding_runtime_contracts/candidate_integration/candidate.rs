//! Candidate tests: the content-only candidate
//! source digest over one verified managed checkout, and the CandidateIdentity
//! carried forward from the verified baseline.

use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{
    CandidateError, CandidateIdentity, DEFAULT_DELTA_DEADLINE, DeltaError, candidate_source_digest,
    operator_project_source_digest,
};
use saya_harness::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;
use std::{fs, path::Path, path::PathBuf, time::Duration};

use super::support_active::{Active, active};

struct TempProject(PathBuf);

impl TempProject {
    fn new(_label: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("op{}-{stamp:x}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_operator_git_refused(root: &Path) {
    assert!(matches!(
        operator_project_source_digest(
            root,
            &IgnoredPaths::none(),
            &saya_agent::CancellationToken::new(),
            DEFAULT_DELTA_DEADLINE
        ),
        Err(CandidateError::Walk(DeltaError::NestedRepository))
    ));
}

#[tokio::test]
async fn candidate_digest_is_deterministic() {
    let a = active("w08-det-a", "checkout-w08-det-a").await;
    let b = active("w08-det-b", "checkout-w08-det-b").await;
    let first = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let second = candidate_source_digest(
        &b.root,
        &IgnoredPaths::none(),
        &b.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(first, second);
    fs::write(b.root.join("extra.txt"), b"extra\n").unwrap();
    let changed = candidate_source_digest(
        &b.root,
        &IgnoredPaths::none(),
        &b.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_ne!(first, changed);
    fs::remove_file(b.root.join("extra.txt")).unwrap();
    let restored = candidate_source_digest(
        &b.root,
        &IgnoredPaths::none(),
        &b.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(first, restored);
}

#[tokio::test]
async fn candidate_digest_ignores_write_order() {
    let a = active("w08-order-a", "checkout-w08-order-a").await;
    let b = active("w08-order-b", "checkout-w08-order-b").await;
    fs::write(a.root.join("x.txt"), b"x bytes\n").unwrap();
    fs::write(a.root.join("y.txt"), b"y bytes\n").unwrap();
    fs::write(b.root.join("y.txt"), b"y bytes\n").unwrap();
    fs::write(b.root.join("x.txt"), b"x bytes\n").unwrap();
    let first = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let second = candidate_source_digest(
        &b.root,
        &IgnoredPaths::none(),
        &b.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn candidate_digest_tracks_content_and_exec_bit() {
    let a = active("w08-sense", "checkout-w08-sense").await;
    let baseline = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();
    let after_content = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_ne!(baseline, after_content);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = a.root.join("a/one.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        let after_exec = candidate_source_digest(
            &a.root,
            &IgnoredPaths::none(),
            &a.token,
            DEFAULT_DELTA_DEADLINE,
        )
        .unwrap();
        assert_ne!(after_content, after_exec);
    }
}

#[tokio::test]
async fn candidate_digest_carries_relative_symlinks_and_skips_unsafe_ones_and_nested_git() {
    let a = active("w08-refuse", "checkout-w08-refuse").await;
    use std::os::unix::fs::symlink;

    symlink("tracked.txt", a.root.join("link")).unwrap();
    let digest = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(
        digest,
        candidate_source_digest(
            &a.root,
            &IgnoredPaths::none(),
            &a.token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap(),
        "a relative symlink is content, and its digest is deterministic"
    );
    fs::remove_file(a.root.join("link")).unwrap();
    symlink("/etc/passwd", a.root.join("link")).unwrap();
    let without_link = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    fs::remove_file(a.root.join("link")).unwrap();
    assert_eq!(
        candidate_source_digest(
            &a.root,
            &IgnoredPaths::none(),
            &a.token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap(),
        without_link,
        "an unsafe symlink is reported, not included in the digest"
    );
    fs::create_dir_all(a.root.join("sub/.git")).unwrap();
    assert_eq!(
        candidate_source_digest(
            &a.root,
            &IgnoredPaths::none(),
            &a.token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap(),
        without_link,
        "a nested repository is reported, not included in the digest"
    );
}

#[tokio::test]
async fn candidate_digest_skips_oversize_instead_of_rejecting() {
    let a = active("w08-big", "checkout-w08-big").await;
    let big = a.root.join("big.bin");
    fs::File::create(&big)
        .unwrap()
        .set_len(MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let with_big = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    fs::remove_file(&big).unwrap();
    assert_eq!(
        candidate_source_digest(
            &a.root,
            &IgnoredPaths::none(),
            &a.token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap(),
        with_big,
        "an oversize file is reported, not included in the digest"
    );
}

#[tokio::test]
async fn candidate_digest_honours_cancellation_and_deadline() {
    let a = active("w08-cancel", "checkout-w08-cancel").await;
    a.token.cancel();
    let err = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .err()
    .unwrap();
    assert!(matches!(err, CandidateError::Walk(DeltaError::Cancelled)));
    let b = active("w08-deadline", "checkout-w08-deadline").await;
    let err = candidate_source_digest(&b.root, &IgnoredPaths::none(), &b.token, Duration::ZERO)
        .err()
        .unwrap();
    assert!(matches!(
        err,
        CandidateError::Walk(DeltaError::DeadlineExceeded)
    ));
}

#[test]
fn operator_project_digest_accepts_only_the_exact_root_git_directory() {
    let project = TempProject::new("root-git-dir");
    fs::create_dir(project.0.join(".git")).unwrap();
    fs::write(project.0.join(".git/config"), b"first metadata").unwrap();
    fs::write(project.0.join("src.rs"), b"project bytes").unwrap();

    let candidate = candidate_source_digest(
        &project.0,
        &IgnoredPaths::none(),
        &saya_agent::CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("candidate mode reports root Git metadata as not included");
    let before = operator_project_source_digest(
        &project.0,
        &IgnoredPaths::none(),
        &saya_agent::CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("operator mode accepts only this root metadata directory");
    assert_eq!(
        candidate, before,
        "both modes exclude the root metadata directory"
    );
    fs::write(project.0.join(".git/config"), b"changed metadata").unwrap();
    let after = operator_project_source_digest(
        &project.0,
        &IgnoredPaths::none(),
        &saya_agent::CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(
        before, after,
        "repository metadata is excluded from source identity"
    );
}

#[test]
fn operator_project_digest_keeps_the_existing_root_git_pointer_behavior() {
    let project = TempProject::new("root-git-pointer");
    fs::write(
        project.0.join(".git"),
        b"gitdir: ../.git/worktrees/project\n",
    )
    .unwrap();
    fs::write(project.0.join("src.rs"), b"project bytes").unwrap();
    let token = saya_agent::CancellationToken::new();
    assert_eq!(
        candidate_source_digest(
            &project.0,
            &IgnoredPaths::none(),
            &token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap(),
        operator_project_source_digest(
            &project.0,
            &IgnoredPaths::none(),
            &token,
            DEFAULT_DELTA_DEADLINE
        )
        .unwrap()
    );
}

#[cfg(unix)]
#[test]
fn operator_project_digest_refuses_nested_casefolded_and_non_directory_git_entries() {
    use std::os::unix::{fs::symlink, net::UnixListener};

    let nested = TempProject::new("nested-git");
    fs::create_dir_all(nested.0.join("sub/.git")).unwrap();
    assert_operator_git_refused(&nested.0);

    let casefold = TempProject::new("casefold-git");
    fs::create_dir(casefold.0.join(".GIT")).unwrap();
    assert_operator_git_refused(&casefold.0);

    let linked = TempProject::new("linked-git");
    fs::create_dir(linked.0.join("target")).unwrap();
    symlink("target", linked.0.join(".git")).unwrap();
    assert_operator_git_refused(&linked.0);

    let special = TempProject::new("special-git");
    let socket = UnixListener::bind(special.0.join(".git")).unwrap();
    assert_operator_git_refused(&special.0);
    drop(socket);
}

#[tokio::test]
async fn candidate_identity_copies_the_verified_baseline_and_hides_paths() {
    let a: Active = active("w08-identity", "checkout-w08-identity").await;
    let identity = CandidateIdentity::from_verified(&a.fixture.job, &a.verified);
    assert_eq!(identity.job_id(), &a.fixture.job);
    assert_eq!(identity.checkout_id(), a.verified.checkout_id());
    assert_eq!(identity.snapshot_id(), a.verified.snapshot_id());
    assert_eq!(identity.snapshot_sha256(), a.verified.snapshot_sha256());
    assert_eq!(identity.plan_sha256(), a.verified.plan_sha256());
    let debug = format!("{identity:?}");
    assert!(debug.contains("checkout-w08-identity"));
    assert!(!debug.contains(a.root.to_string_lossy().as_ref()));
}
