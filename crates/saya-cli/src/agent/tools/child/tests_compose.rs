//! Composition boundary tests for host-owned child run records.

#[cfg(unix)]
use std::os::unix::fs::symlink;
use std::{fs, path::PathBuf};
use std::{path::Path, sync::Arc, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::runner::sandbox::{RunSandbox, SandboxProvision};
use saya_harness::runner::{RunProgram, StaticCredentialSource};
use saya_types::RunnerScope;
use saya_types::{JobId, ManagedCheckoutId, SnapshotId};

use super::super::tests::{Sandbox, checkout, implementer_view};
use super::{
    ChildComposeError, bind_invocation_cancellation, ensure_record_dir_outside_checkout,
    validate_view_ownership,
};

#[test]
fn record_directory_inside_checkout_is_refused() {
    let root = fixture_root("record-inside");
    let checkout = root.join("checkout");
    fs::create_dir_all(&checkout).unwrap();
    let records = checkout.join("records");
    fs::create_dir_all(&records).unwrap();
    // The production contract hands in an already-canonical record dir (the
    // caller's `trusted_record_dir`); the macOS temp dir sits behind a
    // `/var/folders` symlink, so the fixture canonicalizes like the sibling
    // test does.
    let records = records.canonicalize().unwrap();

    let error = ensure_record_dir_outside_checkout(&records, &checkout).unwrap_err();

    assert!(matches!(
        error,
        ChildComposeError::RecordDirectoryInsideCheckout
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn canonical_record_directory_outside_checkout_is_returned() {
    let root = fixture_root("record-canonical");
    let checkout = root.join("checkout");
    let records = root.join("trusted-records");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&records).unwrap();

    let records = records.canonicalize().unwrap();
    let canonical = ensure_record_dir_outside_checkout(&records, &checkout).unwrap();

    assert_eq!(canonical, records);
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn checkout_symlink_cannot_redirect_host_records_outside() {
    let root = fixture_root("record-symlink");
    let checkout = root.join("checkout");
    let records = root.join("trusted-records");
    fs::create_dir_all(&checkout).unwrap();
    fs::create_dir_all(&records).unwrap();
    let link = checkout.join("records");
    symlink(&records, &link).unwrap();

    let error = ensure_record_dir_outside_checkout(&link, &checkout).unwrap_err();

    assert!(matches!(
        error,
        ChildComposeError::RecordDirectoryNotCanonical
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg(unix)]
fn outside_path_with_checkout_symlink_hop_is_refused() {
    let root = fixture_root("record-hop");
    let checkout_dir = root.join("checkout");
    let records = root.join("trusted-records");
    fs::create_dir_all(&checkout_dir).unwrap();
    fs::create_dir_all(&records).unwrap();
    symlink(&records, checkout_dir.join("records")).unwrap();
    let outside_hop = root.join("outside-hop");
    symlink(&checkout_dir, &outside_hop).unwrap();

    let error = ensure_record_dir_outside_checkout(&outside_hop.join("records"), &checkout_dir)
        .unwrap_err();

    assert!(matches!(
        error,
        ChildComposeError::RecordDirectoryNotCanonical
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn authority_view_must_match_verified_job_checkout_and_snapshot() {
    let checkout_id = checkout("owned");
    let view = implementer_view(&checkout_id, &["a/ok.txt"]);
    let job_id = JobId::parse("job").unwrap();
    let snapshot_id = SnapshotId::parse("snapshot").unwrap();

    assert!(validate_view_ownership(&view, &checkout_id, &job_id, &snapshot_id).is_ok());
    assert!(
        validate_view_ownership(
            &view,
            &ManagedCheckoutId::parse("other").unwrap(),
            &job_id,
            &snapshot_id,
        )
        .is_err()
    );
    assert!(
        validate_view_ownership(
            &view,
            &checkout_id,
            &JobId::parse("other-job").unwrap(),
            &snapshot_id,
        )
        .is_err()
    );
    assert!(
        validate_view_ownership(
            &view,
            &checkout_id,
            &job_id,
            &SnapshotId::parse("other-snapshot").unwrap(),
        )
        .is_err()
    );
}

#[tokio::test]
async fn invocation_cancellation_stops_the_confined_runner() {
    let sandbox = Sandbox::new("compose-cancellation");
    let records = sandbox.outer.join("records");
    fs::create_dir_all(&records).unwrap();
    let run_sandbox = RunSandbox::new([sandbox.ws_root()], std::iter::empty()).unwrap();
    let SandboxProvision::Proven { spawn, .. } = run_sandbox.prepare(Path::new("/bin")).unwrap()
    else {
        return;
    };
    let records = records.canonicalize().unwrap();
    let records = ensure_record_dir_outside_checkout(&records, &sandbox.ws_root()).unwrap();
    let runner = RunProgram::for_step(
        spawn,
        Some(RunnerScope::new(vec!["sleep".into()]).unwrap()),
        None,
        Duration::from_secs(10),
        Arc::new(StaticCredentialSource::new([])),
    )
    .with_record_dir(records);
    let cancellation = CancellationToken::new();
    let runner = bind_invocation_cancellation(runner, &cancellation);
    let task = tokio::spawn(async move { runner.run("sleep", &["10".into()], None).await });

    tokio::time::sleep(Duration::from_millis(50)).await;
    cancellation.cancel();
    let outcome = tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    assert!(outcome.cancelled);
}

fn fixture_root(name: &str) -> PathBuf {
    let root =
        std::env::temp_dir().join(format!("saya-child-compose-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    root
}
