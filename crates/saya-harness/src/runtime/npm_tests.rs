use std::{fs, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};

use super::super::{NpmRuntimeError, bind_npm_project};
use super::private_root;

fn package_manifest(name: &str, version: &str) -> Value {
    json!({
        "name": name,
        "version": version,
        "dependencies": {"saya-offline-probe": "1.0.0"}
    })
}

fn package_lock(
    name: &str,
    version: &str,
    lock_version: u64,
    resolved: &str,
    integrity: &str,
) -> Value {
    json!({
        "name": name,
        "version": version,
        "lockfileVersion": lock_version,
        "requires": true,
        "packages": {
            "": {
                "name": name,
                "version": version,
                "dependencies": {"saya-offline-probe": "1.0.0"}
            },
            "node_modules/saya-offline-probe": {
                "version": "1.0.0",
                "resolved": resolved,
                "integrity": integrity
            }
        }
    })
}

fn integrity(fill: u8) -> String {
    format!("sha512-{}", STANDARD.encode([fill; 64]))
}

fn project(
    name: &str,
    version: &str,
    lock_version: u64,
    resolved: &str,
    sri: &str,
) -> (PathBuf, crate::workspace::Workspace) {
    let root = private_root("npm-project");
    write_project(
        &root,
        package_manifest(name, version),
        package_lock(name, version, lock_version, resolved, sri),
    );
    let workspace = crate::workspace::Workspace::open(&root).expect("test project opens");
    (root, workspace)
}

fn write_project(root: &std::path::Path, manifest: Value, lock: Value) {
    fs::write(
        root.join("package.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("package-lock.json"),
        serde_json::to_vec(&lock).unwrap(),
    )
    .unwrap();
}

#[test]
fn valid_v3_project_binding_tracks_manifest_and_lock_bytes() {
    let sri = integrity(7);
    let (first_root, first) = project(
        "react-dashboard",
        "1.0.0",
        3,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &sri,
    );
    let first_binding = bind_npm_project(&first).expect("valid project inputs bind");
    let repeated = bind_npm_project(&first).expect("same bytes have the same binding");
    assert_eq!(first_binding, repeated);
    assert_eq!(first_binding.tarball_count(), 1);
    assert_eq!(first_binding.manifest_sha256().len(), 64);
    assert_eq!(first_binding.lock_sha256().len(), 64);
    assert_eq!(first_binding.input_key_sha256().len(), 64);
    drop(first);

    let (changed_root, changed) = project(
        "react-dashboard",
        "1.0.1",
        3,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &sri,
    );
    let changed_binding = bind_npm_project(&changed).expect("changed lock remains valid");
    assert_ne!(
        first_binding.input_key_sha256(),
        changed_binding.input_key_sha256()
    );
    drop(changed);
    fs::remove_dir_all(first_root).unwrap();
    fs::remove_dir_all(changed_root).unwrap();
}

#[test]
fn foreign_registry_origin_is_refused() {
    let (root, workspace) = project(
        "react-dashboard",
        "1.0.0",
        3,
        "https://packages.attacker.example/saya-offline-probe.tgz",
        &integrity(8),
    );

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::UntrustedOrigin)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn sha512_integrity_is_required() {
    let (root, workspace) = project(
        "react-dashboard",
        "1.0.0",
        3,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &format!("sha256-{}", STANDARD.encode([8; 32])),
    );

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::InvalidIntegrity)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn unsupported_lockfile_versions_are_refused() {
    let (root, workspace) = project(
        "react-dashboard",
        "1.0.0",
        2,
        "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
        &integrity(8),
    );

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::UnsupportedLock)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn missing_operator_consent_refuses_before_state_or_project_access() {
    let root = private_root("npm-no-consent");
    let project = crate::workspace::Workspace::open(&root).expect("empty project root opens");
    let state_root = root.join("saya-state");
    let transport = crate::fetch::ReqwestTransport::new().expect("fetch transport builds");

    let result = super::super::install_npm_inputs(
        &state_root,
        &project,
        None,
        &transport,
        &saya_agent::CancellationToken::new(),
    )
    .await;

    assert_eq!(result, Err(super::super::NpmRuntimeError::ConsentRequired));
    assert!(!state_root.exists(), "no consent creates no state");
    drop(project);
    fs::remove_dir_all(root).unwrap();
}

#[path = "npm_owner_tests.rs"]
mod owner_tests;
#[path = "npm_shape_tests.rs"]
mod shape_tests;
