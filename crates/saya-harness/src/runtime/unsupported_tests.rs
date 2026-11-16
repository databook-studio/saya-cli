//! Windows has no proven managed Node runtime or podman registry: the public
//! entry points refuse by name, and the refusal happens before a state root
//! is opened or created. The private registry, install, and npm machinery is
//! not compiled into the Windows test binary.

use std::path::PathBuf;

use saya_agent::CancellationToken;

use super::{NodeInstallConsent, NodeInstallError, install_node, node_installation_recorded};

fn state_root(case: &str) -> PathBuf {
    std::env::temp_dir().join(format!("saya-runtime-{case}-{}", std::process::id()))
}

#[test]
fn the_record_probe_refuses_an_unsupported_host() {
    let root = state_root("record");
    let refusal = node_installation_recorded(&root).expect_err("the record probe must refuse");
    assert!(
        matches!(refusal, NodeInstallError::UnsupportedPlatform),
        "{refusal:?}"
    );
    assert!(!root.exists(), "the refusal creates no state root");
}

#[tokio::test]
async fn install_refuses_before_opening_the_state_root() {
    let root = state_root("install");
    let _ = std::fs::remove_dir_all(&root);
    let refusal = install_node(
        &root,
        Some(NodeInstallConsent::Granted),
        &CancellationToken::new(),
    )
    .await
    .expect_err("install must refuse");
    assert!(
        matches!(refusal, NodeInstallError::UnsupportedPlatform),
        "{refusal:?}"
    );
    assert!(!root.exists(), "the refusal creates no state root");
}

#[test]
fn consent_still_comes_first() {
    let root = state_root("consent");
    let refusal = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime")
        .block_on(install_node(&root, None, &CancellationToken::new()))
        .expect_err("install must refuse");
    assert!(
        matches!(refusal, NodeInstallError::ConsentRequired),
        "{refusal:?}"
    );
    assert!(!root.exists(), "the refusal creates no state root");
}
