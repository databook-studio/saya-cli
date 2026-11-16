//! The in-place root's ownership proof: the canonical folder, its
//! domain-separated id, and the captured baseline it was fixed from are
//! re-checked at use time. The proof is never a managed checkout — it has no
//! Git base commit and no durable checkout row — and every helper shares the
//! one root rather than deriving its own.

use std::fs;
use std::path::{Path, PathBuf};

use saya_agent::CancellationToken;
use saya_harness::coding::local::{LocalBaseline, LocalRoot};
use saya_harness::coding::snapshot::SnapshotArtifacts;
#[cfg(target_os = "macos")]
use saya_harness::coding::worktree::remove_in_place_scratch;
use saya_harness::coding::worktree::{
    InPlaceRootInput, OwnershipError, ScratchScope, confine_checkout, verify_in_place_root,
};
use saya_types::{Sha256Digest, SnapshotId};

use crate::support::TempRoot;

struct Fixture {
    _temp: TempRoot,
    root: LocalRoot,
    baseline: LocalBaseline,
    artifacts: SnapshotArtifacts,
    state: PathBuf,
}

impl Fixture {
    async fn new(label: &str) -> Self {
        let temp = TempRoot::new(label);
        let folder = temp.path().join("folder");
        fs::create_dir(&folder).unwrap();
        fs::write(folder.join("notes.txt"), b"baseline\n").unwrap();
        fs::create_dir(folder.join("nested")).unwrap();
        fs::write(folder.join("nested/plan.md"), b"plan\n").unwrap();
        let state = temp.path().join("state");
        fs::create_dir(&state).unwrap();
        let root = LocalRoot::open(&folder, &state).unwrap();
        let cancellation = CancellationToken::new();
        let baseline =
            LocalBaseline::edit_time(&root, SnapshotId::parse("job-inplace").unwrap()).unwrap();
        let artifacts = SnapshotArtifacts::open(&temp.path().join("artifacts")).unwrap();
        baseline.capture(&artifacts, &cancellation).await.unwrap();
        Self {
            _temp: temp,
            root,
            baseline,
            artifacts,
            state,
        }
    }

    fn input(&self) -> InPlaceRootInput<'_> {
        InPlaceRootInput {
            artifacts: &self.artifacts,
            state_root: &self.state,
            root_id: self.baseline.repository_id(),
            root_path: Path::new(self.baseline.root_path()),
            snapshot_id: self.baseline.snapshot_id(),
            plan_sha256: self.baseline.manifest_sha256(),
        }
    }

    fn folder(&self) -> &Path {
        self.root.path()
    }
}

/// A moved folder and a symlink standing in for it both refuse: the id is the
/// canonical path. A baseline that no longer carries the recorded plan digest
/// refuses too, and a root id the baseline does not name is not this root.
#[tokio::test]
async fn verify_in_place_root_rechecks_path_and_baseline() {
    let fixture = Fixture::new("w05-inplace-verify").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    assert_eq!(verified.path().as_str(), fixture.folder().to_str().unwrap());
    assert_eq!(verified.snapshot_id(), fixture.baseline.snapshot_id());
    assert_eq!(
        verified.in_place_baseline(),
        Some(fixture.baseline.manifest_sha256())
    );

    let moved = fixture.folder().with_extension("moved");
    fs::rename(fixture.folder(), &moved).unwrap();
    assert!(matches!(
        verify_in_place_root(&fixture.input()),
        Err(OwnershipError::InPlaceRoot(_))
    ));

    std::os::unix::fs::symlink(&moved, fixture.folder()).unwrap();
    assert!(matches!(
        verify_in_place_root(&fixture.input()),
        Err(OwnershipError::InPlaceRoot(_))
    ));
    fs::remove_file(fixture.folder()).unwrap();
    fs::rename(&moved, fixture.folder()).unwrap();

    let wrong_digest = Sha256Digest::parse(&"0".repeat(64)).unwrap();
    let mut tampered = fixture.input();
    tampered.plan_sha256 = &wrong_digest;
    assert!(matches!(
        verify_in_place_root(&tampered),
        Err(OwnershipError::SnapshotMismatch)
    ));

    let other_id = saya_types::RepositoryId::parse(&format!("loc-{}", "1".repeat(64))).unwrap();
    let mut other_root = fixture.input();
    other_root.root_id = &other_id;
    assert!(matches!(
        verify_in_place_root(&other_root),
        Err(OwnershipError::RootMismatch)
    ));
}

/// The verified in-place root is not a managed checkout: it carries no Git
/// base commit, and its checkout id is derived from the root id alone, so it
/// can never collide with a durable managed-checkout record.
#[tokio::test]
async fn in_place_root_not_a_managed_checkout() {
    let fixture = Fixture::new("w05-inplace-not-managed").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    assert!(verified.base().is_in_place());
    assert!(verified.base_commit().is_none());
    let root_hex = fixture
        .baseline
        .repository_id()
        .as_str()
        .strip_prefix("loc-")
        .unwrap();
    assert_eq!(
        verified.checkout_id().as_str(),
        format!("inplace-{root_hex}")
    );
    assert_eq!(verified.path().as_str(), fixture.folder().to_str().unwrap());
}

/// A helper re-verifies the same folder and receives the same root: the
/// proof carries no per-agent path, so no helper ever owns a second root.
#[tokio::test]
async fn helper_shares_in_place_root() {
    let fixture = Fixture::new("w05-inplace-helper").await;
    let root = verify_in_place_root(&fixture.input()).unwrap();
    let helper = verify_in_place_root(&fixture.input()).unwrap();
    assert_eq!(root.path(), helper.path());
    assert_eq!(root.checkout_id(), helper.checkout_id());
    assert_eq!(root.in_place_baseline(), helper.in_place_baseline());
    // The id carries the root's content address, not an agent: two helpers
    // re-verifying the same folder can only ever name the same checkout.
    assert!(
        helper.checkout_id().as_str().ends_with(
            fixture
                .baseline
                .repository_id()
                .as_str()
                .strip_prefix("loc-")
                .unwrap()
        )
    );
}

/// The in-place sandbox grants exactly the folder: one file-system root, the
/// child's cwd, and no second root for the state tree.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn in_place_sandbox_root_is_the_folder() {
    let fixture = Fixture::new("w05-inplace-sandbox").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root).unwrap();

    assert_eq!(
        confined.fs_roots(),
        [fixture.folder().to_path_buf()].as_slice(),
        "the sandbox has exactly the folder as its root"
    );
    assert_eq!(
        confined.command(Path::new("/bin/true")).get_current_dir(),
        Some(fixture.folder())
    );
}

/// In-place scratch is private, one level below the project's `.saya`, and
/// excluded from the baseline. Cleanup removes only the known layout.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn in_place_scratch_private_and_excluded() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = Fixture::new("w05-inplace-scratch").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root).unwrap();

    let base = fixture.folder().join(".saya").join("inplace");
    let (home, tmp) = confined.scratch();
    assert_eq!(home, base.join("home"));
    assert_eq!(tmp, base.join("tmp"));
    for path in [&base, home, tmp] {
        let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700, "{} is not private", path.display());
    }

    let baseline = LocalBaseline::edit_time(
        &fixture.root,
        SnapshotId::parse("job-inplace-scratch").unwrap(),
    )
    .unwrap();
    let value: serde_json::Value = serde_json::from_slice(baseline.canonical_bytes()).unwrap();
    assert!(
        value.get("entries").is_none(),
        "the edit-time marker holds no inventory, so scratch cannot reach it"
    );

    remove_in_place_scratch(fixture.folder()).unwrap();
    assert!(!base.exists(), "the known scratch layout was removed");
    assert!(
        !fixture.folder().join(".saya").exists(),
        "the empty scratch parent is removed too"
    );
}

/// The parent `.saya` is only removed when it is empty: a user file there is
/// never touched, and a `.saya` that held it survives the job.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn in_place_scratch_cleanup_leaves_a_user_saya_directory() {
    let fixture = Fixture::new("w05-inplace-user-saya").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root).unwrap();
    fs::write(fixture.folder().join(".saya/config.toml"), b"user\n").unwrap();

    remove_in_place_scratch(fixture.folder()).unwrap();

    assert!(!fixture.folder().join(".saya/inplace").exists());
    assert!(fixture.folder().join(".saya/config.toml").exists());
}

/// The Linux counterpart: the host cannot prove confinement, so the
/// confinement step refuses with its typed error rather than handing out an
/// unconfined command. No scratch is created and no shell runs.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn in_place_confinement_is_refused_when_the_host_cannot_prove_it() {
    let fixture = Fixture::new("w05-inplace-refused").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    assert!(matches!(
        confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root),
        Err(saya_harness::coding::worktree::ConfinementError::Unsupported)
    ));
    assert!(
        !fixture.folder().join(".saya").exists(),
        "a refusal leaves no scratch behind"
    );
}

/// A confined in-place shell cannot reach the state root beside the folder,
/// while the folder itself stays writable under the same confinement.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn in_place_shell_cannot_reach_state() {
    let fixture = Fixture::new("w05-inplace-state").await;
    let verified = verify_in_place_root(&fixture.input()).unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin"), ScratchScope::Root).unwrap();

    let state_canary = fixture.state.join("w05-canary");
    let denied = confined
        .command(Path::new("/bin/sh"))
        .args([
            "-c",
            "echo x > \"$1\"",
            "sh",
            state_canary.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        !denied.status.success(),
        "the shell wrote the state root: {state_canary:?}"
    );
    assert!(!state_canary.exists());

    let folder_canary = fixture.folder().join("w05-canary");
    let allowed = confined
        .command(Path::new("/bin/sh"))
        .args([
            "-c",
            "echo x > \"$1\"",
            "sh",
            folder_canary.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(allowed.status.success(), "the folder write failed");
    assert!(folder_canary.exists());
}
