use std::{fs, path::Path, time::Duration};

use saya_agent::CancellationToken;

use crate::{
    runtime::{
        CleanupStatus, NpmCheckAction, NpmCheckRequest, NpmCheckSource, NpmCheckStatus, Registry,
        install_with,
        podman::{NodeAction, NpmActionRequest, Podman, PodmanError},
        podman_process::ticket,
        tests::process_fixture,
    },
    workspace::Workspace,
};

use super::{CountingTransport, private_root, project_for_tarball};

const TARBALL: &[u8] = include_bytes!("fixtures/saya-offline-probe-1.0.0.tgz");

async fn ready_runtime() -> (std::path::PathBuf, std::path::PathBuf) {
    let state_root = private_root("npm-effect-state");
    let registry = Registry::open(&state_root).expect("private registry opens");
    let executable = process_fixture::compile_fake_podman(registry.root());
    let podman = Podman::from_candidate(executable.clone(), Some(registry.authfile()))
        .expect("compiled fake process is a trusted concrete candidate");
    install_with(registry, podman, &CancellationToken::new())
        .await
        .expect("the existing Node owner establishes readiness");
    (state_root, executable)
}

fn assert_one_install_action(state_root: &Path) {
    let calls = fs::read_to_string(state_root.join("managed-node-runtime/calls.log"))
        .expect("concrete fake Podman records every owner invocation");
    let install_calls: Vec<_> = calls
        .lines()
        .filter(|line| line.contains("SAYA_NODE_ACTION=install-deps"))
        .collect();
    assert_eq!(
        install_calls.len(),
        1,
        "one closed install action is invoked"
    );
    assert!(install_calls[0].contains("\"--network\", \"none\""));
    assert!(install_calls[0].contains("\"--read-only\""));
}

async fn invoke_input_install(
    state_root: &Path,
    executable: &Path,
    project: &Workspace,
    transport: &CountingTransport,
) -> Result<crate::runtime::NpmInputBundleReceipt, crate::runtime::NpmRuntimeError> {
    let registry = Registry::open(state_root).expect("the installed runtime registry reopens");
    let podman = Podman::from_candidate(executable.to_path_buf(), Some(registry.authfile()))
        .expect("the same trusted concrete Podman owner is selected");
    let binding = crate::runtime::npm_lock::bind_project_inputs(project)
        .expect("the project lock binds before effects");
    crate::runtime::npm_install::install_with(
        registry,
        podman,
        binding,
        transport,
        &CancellationToken::new(),
    )
    .await
}

#[test]
fn verified_lock_tarball_reaches_the_closed_container_validation_owner() {
    crate::runtime::tests::own_process::run(
        module_path!(),
        "verified_lock_tarball_reaches_the_closed_container_validation_owner",
        true,
        async {
            let (project_root, project) = project_for_tarball(TARBALL, None);
            let (state_root, executable) = ready_runtime().await;
            let transport = CountingTransport::with_body(TARBALL);

            let receipt = invoke_input_install(&state_root, &executable, &project, &transport)
                .await
                .expect("SRI-verified lock inputs publish after offline validation");

            assert_eq!(
                transport.gets(),
                1,
                "one lock-authorized tarball is fetched"
            );
            assert!(transport.resolutions() >= 1);
            assert_eq!(receipt.tarball_count(), 1);
            assert_eq!(receipt.tarball_bytes(), TARBALL.len() as u64);
            assert_eq!(receipt.bundle_sha256().len(), 64);
            assert_one_install_action(&state_root);
            let identity = crate::runtime::verified_npm_toolchain_identity(&state_root, &project)
                .expect("the ready registry verifies the installed bundle for this exact project");
            assert_eq!(
                identity.input_key_sha256().as_str(),
                receipt.input_key_sha256()
            );
            assert_eq!(identity.bundle_sha256().as_str(), receipt.bundle_sha256());
            assert_eq!(
                identity.effective_toolchain_id(),
                receipt.effective_toolchain_id()
            );

            fs::write(
                state_root.join("managed-node-runtime/fake-plan"),
                "run_exit=1\n",
            )
            .expect("fake process returns one settled npm action failure");
            let cancellation = CancellationToken::new();
            let source_sha256 = crate::coding::integration::operator_project_source_digest(
                project.root(),
                &crate::coding::git::IgnoredPaths::none(),
                &cancellation,
                std::time::Duration::from_secs(30),
            )
            .expect("the source identity is computed over the current workspace")
            .as_str()
            .to_owned();
            let registry =
                Registry::open(&state_root).expect("the ready registry reopens for checks");
            let podman = Podman::from_candidate(executable, Some(registry.authfile()))
                .expect("the same trusted concrete Podman owner is selected");
            let binding = crate::runtime::npm_lock::bind_project_inputs(&project)
                .expect("the project lock binds before checks");
            let outcome = crate::runtime::npm_check::run_with(
                registry,
                podman,
                binding,
                NpmCheckRequest {
                    state_root: &state_root,
                    project: &project,
                    source_sha256: &source_sha256,
                    source: NpmCheckSource::OperatorProject,
                    action: NpmCheckAction::TestCi,
                    timeout: std::time::Duration::from_secs(30),
                    cancellation: &cancellation,
                },
            )
            .await
            .expect("a joined nonzero check returns process facts");
            assert_eq!(outcome.status(), NpmCheckStatus::Failed);
            assert_eq!(outcome.exit_code(), Some(1));
            assert!(outcome.client_pid() > 0);
            assert!(outcome.joined());
            assert_eq!(outcome.cleanup(), CleanupStatus::Removed);
            assert!(outcome.stdout_total_bytes() > 0);
            assert!(outcome.stdout().contains("v24.21.0"));
            assert_eq!(outcome.output_sha256().len(), 64);
            assert_eq!(outcome.snapshot_sha256().len(), 64);
            assert_eq!(
                outcome.source_snapshot_policy_id(),
                "saya-node-source-snapshot-v1-64mib"
            );
            assert_eq!(
                outcome.effective_toolchain_id(),
                receipt.effective_toolchain_id()
            );
            assert_eq!(
                identity.effective_toolchain_id(),
                outcome.effective_toolchain_id()
            );
            fs::write(
                state_root
                    .join("managed-node-runtime/npm-inputs")
                    .join(identity.input_key_sha256().as_str())
                    .join("bundle.json"),
                b"{}\n",
            )
            .unwrap();
            assert!(matches!(
                crate::runtime::verified_npm_toolchain_identity(&state_root, &project),
                Err(crate::runtime::NpmRuntimeError::StateConflict)
            ));
            fs::write(project.root().join("package-lock.json"), b"{}\n").unwrap();
            assert!(matches!(
                crate::runtime::verified_npm_toolchain_identity(&state_root, &project),
                Err(crate::runtime::NpmRuntimeError::UnsupportedLock)
            ));
            let calls = fs::read_to_string(state_root.join("managed-node-runtime/calls.log"))
                .expect("both managed actions are recorded by the concrete process fixture");
            assert!(calls.contains("SAYA_NODE_ACTION=test-ci"));

            drop(project);
            fs::remove_dir_all(project_root).unwrap();
            fs::remove_dir_all(state_root).unwrap();
        },
    );
}

#[test]
fn downloaded_bytes_with_wrong_sri_are_refused_before_offline_install() {
    crate::runtime::tests::own_process::run(
        module_path!(),
        "downloaded_bytes_with_wrong_sri_are_refused_before_offline_install",
        true,
        async {
            let (project_root, project) = project_for_tarball(TARBALL, Some(0x5a));
            let (state_root, executable) = ready_runtime().await;
            let transport = CountingTransport::with_body(TARBALL);

            let result = invoke_input_install(&state_root, &executable, &project, &transport).await;

            assert_eq!(
                result,
                Err(crate::runtime::NpmRuntimeError::InvalidIntegrity)
            );
            assert_eq!(transport.gets(), 1);
            assert!(transport.resolutions() >= 1);
            assert_eq!(
                crate::runtime::verified_npm_toolchain_identity(&state_root, &project),
                Err(crate::runtime::NpmRuntimeError::InputBundleMissing)
            );
            let calls = fs::read_to_string(state_root.join("managed-node-runtime/calls.log"))
                .expect("Node setup process calls remain observable");
            assert!(
                !calls
                    .lines()
                    .any(|line| line.contains("SAYA_NODE_ACTION=install-deps"))
            );

            drop(project);
            fs::remove_dir_all(project_root).unwrap();
            fs::remove_dir_all(state_root).unwrap();
        },
    );
}

#[tokio::test]
async fn npm_action_timeout_without_observed_container_keeps_the_probe_unknown() {
    let state_root = private_root("npm-incomplete-create");
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
    let controller = stage.join("npm-controller.js");
    let cancellation = CancellationToken::new();
    fs::write(
        registry.root().join("fake-plan"),
        "run_no_create=1\nrun_delay_ms=5000\n",
    )
    .expect("the concrete process fixture delays without creating a container");

    let result = podman
        .run_npm_action(
            &registry,
            NpmActionRequest {
                ticket: &operation_ticket,
                bundle: &stage,
                controller: &controller,
                source: None,
                action: NodeAction::InstallDeps,
                timeout: Duration::from_millis(50),
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
