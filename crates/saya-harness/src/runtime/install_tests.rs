use std::{
    fs,
    time::{Duration, Instant},
};

use super::{own_process, private_root, process_fixture};

#[test]
fn installer_rejects_unproved_image_node_and_npm_before_ready_publication() {
    own_process::run(
        module_path!(),
        "installer_rejects_unproved_image_node_and_npm_before_ready_publication",
        true,
        async {
            let root = private_root("identity-proofs");
            let executable = process_fixture::compile_fake_podman(&root);
            for (label, plan) in [
                ("image", "digest=sha256:wrong\n"),
                ("node", "node=v0.0.0\n"),
                ("npm", "npm=0.0.0\n"),
            ] {
                let state_root = root.join(label);
                let registry = super::super::Registry::open(&state_root).expect("registry opens");
                fs::write(registry.root().join("fake-plan"), plan).expect("fake plan written");
                let podman = super::super::podman::Podman::from_candidate(
                    executable.clone(),
                    Some(registry.authfile()),
                )
                .expect("concrete process wrapper accepts the compiled fixture");

                let result = super::super::install_with(
                    registry,
                    podman,
                    &saya_agent::CancellationToken::new(),
                )
                .await;

                let error = result.expect_err("unproved identities cannot publish a ready record");
                let expected_error = match (label, error) {
                    ("image", super::super::NodeInstallError::ImageUnproved { observed, .. }) => {
                        observed == "sha256:wrong"
                    }
                    (
                        "node",
                        super::super::NodeInstallError::NodeVersionUnproved { observed, .. },
                    ) => observed == "v0.0.0",
                    (
                        "npm",
                        super::super::NodeInstallError::NpmVersionUnproved { observed, .. },
                    ) => observed == "0.0.0",
                    (_, other) => panic!("the owner returned the wrong identity error: {other}"),
                };
                assert!(expected_error, "the owner reports the unproved identity");
                let registry =
                    super::super::Registry::open(&state_root).expect("registry can be inspected");
                assert!(!registry.has_ready().expect("no ready record was published"));
                drop(registry);
            }
            fs::remove_dir_all(root).expect("test-owned root removed");
        },
    );
}

#[tokio::test]
async fn installer_publishes_only_after_probes_then_reuses_the_exact_ready_record() {
    let root = private_root("identity-ready");
    let executable = process_fixture::compile_fake_podman(&root);
    let state_root = root.join("state");
    let registry = super::super::Registry::open(&state_root).expect("registry opens");
    let authfile = registry.authfile();
    let podman =
        super::super::podman::Podman::from_candidate(executable.clone(), Some(authfile.clone()))
            .expect("concrete process wrapper accepts the compiled fixture");

    let first = super::super::install_with(registry, podman, &saya_agent::CancellationToken::new())
        .await
        .expect("the compiled exact image and versions install");
    assert!(matches!(
        first,
        super::super::NodeInstallReceipt::Installed { .. }
    ));

    let registry = super::super::Registry::open(&state_root).expect("ready registry reopens");
    assert!(registry.has_ready().expect("published record is exact"));
    let podman =
        super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
            .expect("concrete process wrapper accepts the compiled fixture");
    let second =
        super::super::install_with(registry, podman, &saya_agent::CancellationToken::new())
            .await
            .expect("the exact record is revalidated and reused");
    assert!(matches!(
        second,
        super::super::NodeInstallReceipt::Reused { .. }
    ));

    let calls = fs::read_to_string(root.join("state/managed-node-runtime/calls.log"))
        .expect("compiled process recorded invocations");
    assert_eq!(
        calls
            .lines()
            .filter(|line| line.contains("args=[\"pull\""))
            .count(),
        1
    );
    assert!(
        calls
            .lines()
            .all(|line| line.contains(&format!("auth={}", authfile.display())))
    );
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn cancellation_with_unknown_cleanup_keeps_cancelled_disposition_and_blocks_reuse() {
    own_process::run(
        module_path!(),
        "cancellation_with_unknown_cleanup_keeps_cancelled_disposition_and_blocks_reuse",
        true,
        async {
            let root = private_root("cancelled-cleanup-unknown");
            let state_root = root.join("state");
            let registry = super::super::Registry::open(&state_root).expect("registry opens");
            let executable = process_fixture::compile_fake_podman(registry.root());
            fs::write(
                registry.root().join("fake-plan"),
                "run_delay_ms=2000\nrm_exit=125\nexists_exit=125\n",
            )
            .expect("fake plan written");
            let authfile = registry.authfile();
            let podman = super::super::podman::Podman::from_candidate(
                executable.clone(),
                Some(authfile.clone()),
            )
            .expect("concrete process wrapper accepts the compiled fixture");
            let cancellation = saya_agent::CancellationToken::new();
            let cancel = cancellation.clone();
            let started = registry.root().join("run-started");
            let waiter = tokio::spawn(async move {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !started.exists() && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                assert!(started.exists(), "the owner reached the probe process");
                cancel.cancel();
            });

            let result = podman.probe_node(&registry, &cancellation).await;
            waiter.await.expect("cancellation controller completes");
            assert!(matches!(
                result,
                Err(super::super::podman::PodmanError::CleanupUnknown {
                    operation: Some(super::super::podman::ProbeFailure::Cancelled)
                })
            ));
            drop(registry);

            let registry =
                super::super::Registry::open(&state_root).expect("pending locator is retained");
            let podman =
                super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
                    .expect("concrete process wrapper accepts the compiled fixture");
            let result =
                super::super::install_with(registry, podman, &saya_agent::CancellationToken::new())
                    .await;
            assert!(matches!(
                result,
                Err(super::super::NodeInstallError::OperationFailed {
                    phase: super::super::NodeInstallPhase::ReconcileProbe,
                    cleanup: super::super::CleanupStatus::Unknown,
                    ..
                })
            ));
            let registry =
                super::super::Registry::open(&state_root).expect("registry remains inspectable");
            assert!(
                !registry
                    .has_ready()
                    .expect("unknown cleanup blocks ready reuse")
            );
            drop(registry);
            fs::remove_dir_all(root).expect("test-owned root removed");
        },
    );
}
