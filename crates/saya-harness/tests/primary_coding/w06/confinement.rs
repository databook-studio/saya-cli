//! W06c `confine_checkout` real-process battery (authored, not executed): the
//! first failing proof that a confined child cannot write outside the single
//! owned root, the owned-write counter-proof, the missing program dir
//! refusal, and the Debug shape.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::worktree::{
    ConfinementError, OwnershipInput, confine_checkout, create_managed_checkout,
    verify_owned_checkout,
};
use saya_types::{JobId, ManagedCheckoutId};

use super::create_support::CreateCheckoutFixture;

/// The checkout under confinement in every test.
const A_ID: &str = "checkout-w06-a";
/// The sibling checkout created next to the confined one in the deny test.
const B_ID: &str = "checkout-w06-b";

/// One ownership input over the fixture for the given asking job.
fn ownership_input<'a>(
    fixture: &'a CreateCheckoutFixture,
    job_id: &'a JobId,
    checkout_id: &'a ManagedCheckoutId,
) -> OwnershipInput<'a> {
    OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id,
        checkout_id,
    }
}

/// A confined child of checkout A cannot write the common Git metadata, the
/// sibling checkout B, the original workspace, or a file next to A's own
/// directory: every write fails and leaves no canary behind.
#[tokio::test]
async fn child_command_cannot_write_shared_git_or_sibling() {
    let fixture = CreateCheckoutFixture::new("w06-confine-deny").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let b = ManagedCheckoutId::parse(B_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    create_managed_checkout(&fixture.input(&b), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();

    let a_path = fixture.service.managed_path(&a).unwrap();
    let b_path = fixture.service.managed_path(&b).unwrap();
    let targets = [
        format!("{}/.git/w06-canary", fixture.repo_root.display()),
        format!("{}/w06-canary", b_path.as_str()),
        format!("{}/w06-canary", fixture.repo_root.display()),
        format!(
            "{}/w06-canary",
            Path::new(a_path.as_str()).parent().unwrap().display()
        ),
    ];
    for target in targets {
        let output = confined
            .command(Path::new("/bin/sh"))
            .args(["-c", "echo x > \"$1\"", "sh", target.as_str()])
            .output()
            .unwrap();
        assert!(!output.status.success(), "the child wrote {target}");
        assert!(!Path::new(&target).exists(), "the canary {target} exists");
        if cfg!(target_os = "macos") {
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("Operation not permitted"),
                "the macOS denial is missing for {target}: {stderr}"
            );
        }
    }
}

/// Under the same confinement a child of checkout A writes inside the owned
/// checkout, and its working directory is the checkout itself.
#[tokio::test]
async fn owned_checkout_write_succeeds_under_same_confinement() {
    let fixture = CreateCheckoutFixture::new("w06-owned-write").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();
    let managed = fixture.service.managed_path(&a).unwrap();

    let ok = format!("{}/w06-ok", managed.as_str());
    let output = confined
        .command(Path::new("/bin/sh"))
        .args(["-c", "echo x > \"$1\"", "sh", ok.as_str()])
        .output()
        .unwrap();
    assert!(output.status.success(), "the owned write failed: {ok}");
    assert_eq!(
        fs::read_to_string(&ok).unwrap(),
        "x\n",
        "the owned canary content is wrong: {ok}"
    );

    let cwd_output = confined
        .command(Path::new("/bin/sh"))
        .args(["-c", "pwd -P > cwd.txt"])
        .output()
        .unwrap();
    assert!(cwd_output.status.success());
    let cwd_file = Path::new(managed.as_str()).join("cwd.txt");
    let cwd = fs::read_to_string(&cwd_file).unwrap();
    let expected = fs::canonicalize(managed.as_str()).unwrap();
    assert_eq!(cwd.trim(), expected.to_string_lossy().trim());
}

/// Confinement refuses a program directory that does not exist: the sandbox
/// build fails as `ConfinementError::Sandbox`.
#[tokio::test]
async fn confinement_refuses_a_missing_program_dir() {
    let fixture = CreateCheckoutFixture::new("w06-missing-dir").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/nonexistent-w06-program-dir"));
    assert!(matches!(confined, Err(ConfinementError::Sandbox(_))));
}

/// The Debug shape of a confined checkout carries the checkout id and never
/// a path.
#[tokio::test]
async fn confined_debug_carries_the_checkout_id_and_no_path() {
    let fixture = CreateCheckoutFixture::new("w06-debug-shape").await;
    let a = ManagedCheckoutId::parse(A_ID).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&a), &token)
        .await
        .unwrap();
    let input = ownership_input(&fixture, &fixture.job, &a);
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let confined = confine_checkout(&verified, Path::new("/bin")).unwrap();
    let debug = format!("{confined:?}");
    assert!(debug.contains(A_ID), "the debug shape is {debug}");
    let managed = fixture.service.managed_path(&a).unwrap();
    assert!(
        !debug.contains(managed.as_str()),
        "the debug shape leaks the path: {debug}"
    );
}
