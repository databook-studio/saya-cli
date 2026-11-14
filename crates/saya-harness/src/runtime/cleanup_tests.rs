use std::{fs, time::Instant};

use super::{private_root, process_fixture};

#[tokio::test]
async fn cleanup_does_not_treat_engine_exit_125_as_container_absence() {
    let root = private_root("cleanup-exit-125");
    let executable = process_fixture::compile_fake_podman(&root);
    let authfile = root.join("registry-auth.json");
    fs::write(&authfile, br#"{"auths":{}}"#).expect("fixture authfile written");
    fs::write(root.join("fake-plan"), "exists_exit=125\n").expect("fake plan written");
    let cidfile = root.join("container-id");
    let ticket = "fixture-ticket";
    let name = format!("saya-node-probe-{ticket}");
    fs::write(
        &cidfile,
        "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
    )
    .expect("owned container id is available for cleanup");

    let cancellation = saya_agent::CancellationToken::new();
    let context = super::super::podman_cleanup::CleanupContext::new(
        &executable,
        None,
        &authfile,
        Instant::now() + super::super::podman_cleanup::CLEANUP_BUDGET,
        &cancellation,
    );
    let status =
        super::super::podman_cleanup::cleanup(&context, &cidfile, &name, ticket, false).await;

    assert_eq!(status, super::super::CleanupStatus::Unknown);
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[tokio::test]
async fn cleanup_finds_an_owned_container_when_podman_did_not_write_the_cidfile() {
    let root = private_root("cleanup-missing-cid");
    let executable = process_fixture::compile_fake_podman(&root);
    let authfile = root.join("registry-auth.json");
    fs::write(&authfile, br#"{"auths":{}}"#).expect("fixture authfile written");
    fs::write(root.join("fake-plan"), "exists_exit=1\n").expect("fake plan written");
    let ticket = "fixture-ticket";
    let name = format!("saya-node-probe-{ticket}");
    process_fixture::write_container_record(&root, ticket, &name);
    let cidfile = root.join("cidfile-not-created");

    let cancellation = saya_agent::CancellationToken::new();
    let context = super::super::podman_cleanup::CleanupContext::new(
        &executable,
        None,
        &authfile,
        Instant::now() + super::super::podman_cleanup::CLEANUP_BUDGET,
        &cancellation,
    );
    let status =
        super::super::podman_cleanup::cleanup(&context, &cidfile, &name, ticket, true).await;

    assert_eq!(status, super::super::CleanupStatus::Removed);
    assert!(!process_fixture::container_record_path(&root).exists());
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[tokio::test]
async fn cleanup_never_removes_a_container_with_a_different_ticket() {
    let root = private_root("cleanup-mismatched-ticket");
    let executable = process_fixture::compile_fake_podman(&root);
    let authfile = root.join("registry-auth.json");
    fs::write(&authfile, br#"{"auths":{}}"#).expect("fixture authfile written");
    let expected_ticket = "expected-ticket";
    let name = format!("saya-node-probe-{expected_ticket}");
    process_fixture::write_container_record(&root, "different-ticket", &name);
    let cancellation = saya_agent::CancellationToken::new();
    let context = super::super::podman_cleanup::CleanupContext::new(
        &executable,
        None,
        &authfile,
        Instant::now() + super::super::podman_cleanup::CLEANUP_BUDGET,
        &cancellation,
    );

    let status = super::super::podman_cleanup::cleanup(
        &context,
        &root.join("missing-cidfile"),
        &name,
        expected_ticket,
        false,
    )
    .await;

    assert_eq!(status, super::super::CleanupStatus::Unknown);
    assert!(process_fixture::container_record_path(&root).exists());
    let calls = fs::read_to_string(root.join("calls.log")).expect("engine calls recorded");
    assert!(!calls.lines().any(|line| line.contains("args=[\"rm\"")));
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[tokio::test]
async fn cleanup_does_not_remove_when_inspect_fails_after_container_exists() {
    let root = private_root("cleanup-inspect-125");
    let executable = process_fixture::compile_fake_podman(&root);
    let authfile = root.join("registry-auth.json");
    fs::write(&authfile, br#"{"auths":{}}"#).expect("fixture authfile written");
    fs::write(root.join("fake-plan"), "container_inspect_exit=125\n")
        .expect("inspection failure is controlled");
    let ticket = "fixture-ticket";
    let name = format!("saya-node-probe-{ticket}");
    process_fixture::write_container_record(&root, ticket, &name);
    let cancellation = saya_agent::CancellationToken::new();
    let context = super::super::podman_cleanup::CleanupContext::new(
        &executable,
        None,
        &authfile,
        Instant::now() + super::super::podman_cleanup::CLEANUP_BUDGET,
        &cancellation,
    );

    let status = super::super::podman_cleanup::cleanup(
        &context,
        &root.join("missing-cidfile"),
        &name,
        ticket,
        false,
    )
    .await;

    assert_eq!(status, super::super::CleanupStatus::Unknown);
    assert!(process_fixture::container_record_path(&root).exists());
    let calls = fs::read_to_string(root.join("calls.log")).expect("engine calls recorded");
    assert!(!calls.lines().any(|line| line.contains("args=[\"rm\"")));
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[tokio::test]
async fn incomplete_create_with_no_observed_container_retains_its_recovery_ticket() {
    let root = private_root("cleanup-incomplete-create");
    let state_root = root.join("state");
    let registry = super::super::Registry::open(&state_root).expect("registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    fs::write(
        registry.root().join("fake-plan"),
        "run_exit=125\nrun_no_create=1\n",
    )
    .expect("fake process models an interrupted create with no observation");
    let podman =
        super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
            .expect("compiled fake process passes concrete path checks");

    let result = podman
        .probe_node(&registry, &saya_agent::CancellationToken::new())
        .await;

    assert!(matches!(
        result,
        Err(super::super::podman::PodmanError::CleanupUnknown { .. })
    ));
    assert_eq!(
        registry
            .pending_probe_tickets()
            .expect("uncertain operation remains recoverable")
            .len(),
        1
    );
    assert!(
        !registry
            .has_ready()
            .expect("uncertain create cannot publish")
    );
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}
