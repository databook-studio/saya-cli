use std::{fs, time::Duration};

use saya_agent::CancellationToken;

#[cfg(unix)]
use crate::runtime::podman::PodmanError;
use crate::runtime::{
    CleanupStatus, Registry,
    podman::{NodeAction, NpmActionRequest, Podman},
    podman_process::ticket,
    tests::process_fixture,
};

use super::private_root;

#[tokio::test]
async fn npm_action_reserved_client_exit_is_known_after_owned_container_removal() {
    let state_root = private_root("npm-reserved-exit-owned");
    let registry = Registry::open(&state_root).expect("private registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    let podman = Podman::from_candidate(executable, Some(registry.authfile()))
        .expect("compiled fake process is a trusted concrete candidate");
    let operation_ticket = ticket().expect("ticket generation succeeds");
    let stage = registry
        .create_npm_stage(&operation_ticket)
        .expect("a private action stage is created");
    registry
        .begin_probe(&operation_ticket)
        .expect("the owner durably records the pending action");
    fs::write(registry.root().join("fake-plan"), "run_exit=125\n")
        .expect("the concrete fixture creates an owned container then exits 125");
    let controller = stage.join("npm-controller.js");
    let cancellation = CancellationToken::new();

    let result = podman
        .run_npm_action(
            &registry,
            NpmActionRequest {
                ticket: &operation_ticket,
                bundle: &stage,
                controller: &controller,
                source: None,
                action: NodeAction::InstallDeps,
                timeout: Duration::from_secs(5),
                cancellation: &cancellation,
            },
        )
        .await;
    let known_removed = matches!(
        &result,
        Ok((output, CleanupStatus::Removed)) if output.exit_code == Some(125)
    );
    let _ = podman.finish_npm_ticket(&registry, &operation_ticket);
    fs::remove_dir_all(&stage).unwrap();
    drop(registry);
    fs::remove_dir_all(state_root).unwrap();
    assert!(known_removed, "owned cleanup must settle the reserved exit");
}

#[cfg(unix)]
#[tokio::test]
async fn npm_action_with_killed_orphan_and_no_container_keeps_probe_unknown() {
    let state_root = private_root("npm-orphan-without-container");
    let registry = Registry::open(&state_root).expect("private registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    let podman = Podman::from_candidate(executable, Some(registry.authfile()))
        .expect("compiled fake process is a trusted concrete candidate");
    let operation_ticket = ticket().expect("ticket generation succeeds");
    let stage = registry
        .create_npm_stage(&operation_ticket)
        .expect("a private action stage is created");
    registry
        .begin_probe(&operation_ticket)
        .expect("the owner durably records the pending action");
    fs::write(
        registry.root().join("fake-plan"),
        "run_no_create=1\nrun_orphan=1\n",
    )
    .expect("the concrete fixture leaves an unjoined process-group child");
    let controller = stage.join("npm-controller.js");
    let cancellation = CancellationToken::new();

    let result = podman
        .run_npm_action(
            &registry,
            NpmActionRequest {
                ticket: &operation_ticket,
                bundle: &stage,
                controller: &controller,
                source: None,
                action: NodeAction::InstallDeps,
                timeout: Duration::from_secs(5),
                cancellation: &cancellation,
            },
        )
        .await;

    let unknown = matches!(result, Err(PodmanError::CleanupUnknown { .. }));
    let ticket_retained =
        registry.pending_probe_tickets().unwrap() == vec![operation_ticket.clone()];
    fs::remove_dir_all(&stage).unwrap();
    registry.finish_probe(&operation_ticket).unwrap();
    let _ = fs::remove_file(
        registry
            .root()
            .join(format!("container-{operation_ticket}")),
    );
    drop(registry);
    fs::remove_dir_all(state_root).unwrap();
    assert!(unknown);
    assert!(ticket_retained);
}
