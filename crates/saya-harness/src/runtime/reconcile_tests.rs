use std::{
    fs,
    time::{Duration, Instant},
};

use super::{own_process, private_root, process_fixture};

#[tokio::test]
async fn pre_cancelled_reconciliation_starts_no_engine_process() {
    let root = private_root("reconcile-pre-cancel");
    let state_root = root.join("state");
    let registry = super::super::Registry::open(&state_root).expect("registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    let podman =
        super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
            .expect("compiled fake process passes concrete path checks");
    let cancellation = saya_agent::CancellationToken::new();
    cancellation.cancel();

    let result = super::super::install_with(registry, podman, &cancellation).await;

    assert!(matches!(
        result,
        Err(super::super::NodeInstallError::Cancelled {
            phase: super::super::NodeInstallPhase::ReconcileProbe,
            ..
        })
    ));
    assert!(!root.join("state/managed-node-runtime/calls.log").exists());
    fs::remove_dir_all(root).expect("test-owned root removed");
}

#[test]
fn excess_pending_probe_tickets_block_before_engine_work_and_remain_durable() {
    own_process::run(
        module_path!(),
        "excess_pending_probe_tickets_block_before_engine_work_and_remain_durable",
        true,
        async {
            const EXCESS_TICKETS: usize = 9;

            let root = private_root("reconcile-ticket-bound");
            let state_root = root.join("state");
            let registry = super::super::Registry::open(&state_root).expect("registry opens");
            let tickets: Vec<String> = (0..EXCESS_TICKETS)
                .map(|index| format!("{index:048x}"))
                .collect();
            for ticket in &tickets {
                registry
                    .begin_probe(ticket)
                    .expect("valid operation ticket is durably recorded");
            }
            let executable = process_fixture::compile_fake_podman(registry.root());
            let podman =
                super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
                    .expect("compiled fake process passes concrete path checks");

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
            let registry = super::super::Registry::open(&state_root).expect("registry reopens");
            for ticket in &tickets {
                let bytes = fs::read(registry.root().join(format!("probe-{ticket}")))
                    .expect("every refused operation locator remains present");
                assert_eq!(bytes.as_slice(), ticket.as_bytes());
            }
            assert!(!registry.root().join("calls.log").exists());
            drop(registry);
            fs::remove_dir_all(root).expect("test-owned root removed");
        },
    );
}

#[tokio::test]
async fn cancellation_during_pending_probe_inspect_stops_the_same_install() {
    let root = private_root("reconcile-cancel-in-flight");
    let state_root = root.join("state");
    let registry = super::super::Registry::open(&state_root).expect("registry opens");
    let ticket = format!("{:048x}", 1);
    let name = format!("saya-node-probe-{ticket}");
    registry
        .begin_probe(&ticket)
        .expect("pending operation is durable");
    process_fixture::write_container_record(registry.root(), &ticket, &name);
    fs::write(registry.root().join("fake-plan"), "inspect_delay_ms=5000\n")
        .expect("fake inspection remains bounded while cancellation arrives");
    let executable = process_fixture::compile_fake_podman(registry.root());
    let podman =
        super::super::podman::Podman::from_candidate(executable, Some(registry.authfile()))
            .expect("compiled fake process passes concrete path checks");
    let cancellation = saya_agent::CancellationToken::new();
    let install_cancellation = cancellation.clone();
    let inspect_started = registry.root().join("inspect-started");
    let inspect_completed = registry.root().join("inspect-completed");
    let mut install = tokio::spawn(async move {
        super::super::install_with(registry, podman, &install_cancellation).await
    });
    let inspect_deadline = Instant::now() + Duration::from_secs(10);
    while !inspect_started.exists() && Instant::now() < inspect_deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let inspect_reached = inspect_started.exists();
    cancellation.cancel();
    let result = match tokio::time::timeout(Duration::from_secs(2), &mut install).await {
        Ok(result) => result.expect("installation task completes"),
        Err(_) => {
            install.abort();
            let _ = install.await;
            panic!("cancellation settles the same install promptly");
        }
    };

    assert!(inspect_reached, "reconciliation reached engine inspect");
    assert!(matches!(
        result,
        Err(super::super::NodeInstallError::Cancelled {
            phase: super::super::NodeInstallPhase::ReconcileProbe,
            cleanup: super::super::CleanupStatus::Unknown,
            ..
        })
    ));
    assert!(!inspect_completed.exists());
    let registry =
        super::super::Registry::open(&state_root).expect("uncertain ticket stays durable");
    assert_eq!(registry.pending_probe_tickets().unwrap(), vec![ticket]);
    assert!(
        !registry
            .has_ready()
            .expect("uncertain recovery blocks ready")
    );
    drop(registry);
    fs::remove_dir_all(root).expect("test-owned root removed");
}
