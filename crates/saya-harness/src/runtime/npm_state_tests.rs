use std::fs;

use crate::runtime::{
    NpmCheckAction, NpmCheckRequest, NpmCheckSource, NpmInstallConsent, NpmRuntimeError, Registry,
    install_npm_inputs, run_npm_check,
};

use super::super::project;
use super::CountingTransport;

#[test]
fn altered_ready_record_is_a_known_conflict_before_fetching() {
    crate::runtime::tests::own_process::run(
        module_path!(),
        "altered_ready_record_is_a_known_conflict_before_fetching",
        false,
        async {
            let (project_root, project) = project(
                "react-dashboard",
                "1.0.0",
                3,
                "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
                &super::super::integrity(30),
            );
            let state_root = super::super::private_root("npm-state-conflict");
            let registry = Registry::open(&state_root).expect("private registry opens");
            registry
                .publish()
                .expect("valid record is written by the owner");
            let ready_path = registry.root().join("ready");
            drop(registry);
            fs::write(&ready_path, b"not the pinned runtime\n").unwrap();
            let transport = CountingTransport::default();

            let result = install_npm_inputs(
                &state_root,
                &project,
                Some(NpmInstallConsent::Granted),
                &transport,
                &saya_agent::CancellationToken::new(),
            )
            .await;

            assert_eq!(result, Err(NpmRuntimeError::StateConflict));
            assert_eq!(transport.calls(), 0);
            assert_eq!(fs::read(&ready_path).unwrap(), b"not the pinned runtime\n");
            drop(project);
            fs::remove_dir_all(project_root).unwrap();
            fs::remove_dir_all(state_root).unwrap();
        },
    );
}

#[test]
fn pending_probe_blocks_acquisition_and_check_until_owner_recovery() {
    crate::runtime::tests::own_process::run(
        module_path!(),
        "pending_probe_blocks_acquisition_and_check_until_owner_recovery",
        false,
        async {
            let (project_root, project) = project(
                "react-dashboard",
                "1.0.0",
                3,
                "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
                &super::super::integrity(31),
            );
            let state_root = super::super::private_root("npm-pending-probe");
            let ticket = "0123456789abcdef0123456789abcdef0123456789abcdef";
            let registry = Registry::open(&state_root).expect("private registry opens");
            registry
                .begin_probe(ticket)
                .expect("owner records pending probe");
            drop(registry);
            let transport = CountingTransport::default();

            let install = install_npm_inputs(
                &state_root,
                &project,
                Some(NpmInstallConsent::Granted),
                &transport,
                &saya_agent::CancellationToken::new(),
            )
            .await;
            let source_sha256 = "b".repeat(64);
            let cancellation = saya_agent::CancellationToken::new();
            let check = run_npm_check(NpmCheckRequest {
                state_root: &state_root,
                project: &project,
                source_sha256: &source_sha256,
                source: NpmCheckSource::OperatorProject,
                action: NpmCheckAction::TestCi,
                timeout: std::time::Duration::from_secs(30),
                cancellation: &cancellation,
            })
            .await;

            assert_eq!(install, Err(NpmRuntimeError::RecoveryRequired));
            assert_eq!(check, Err(NpmRuntimeError::RecoveryRequired));
            assert_eq!(transport.calls(), 0);
            let registry = Registry::open(&state_root).expect("pending ticket remains readable");
            assert_eq!(registry.pending_probe_tickets().unwrap(), vec![ticket]);
            drop(registry);
            drop(project);
            fs::remove_dir_all(project_root).unwrap();
            fs::remove_dir_all(state_root).unwrap();
        },
    );
}
