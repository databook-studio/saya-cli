//! The managed Node runtime registry is one durable location derived from the
//! actual state database path: `<database parent>/managed-node-runtime`.
//! Operator install and check commands resolve it through the state database
//! path, never from a coding artifact root or its parent.
//!
//! The state root is built with the harness's own private-root fixture pattern
//! (existing 0700 directory, canonicalized once), because `Registry::open`
//! refuses state roots that are not private and canonical.

use super::temp_root;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

const MANIFEST: &str = r#"{"name":"registry-binding","version":"1.0.0"}"#;
const LOCK: &str = r#"{"name":"registry-binding","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"registry-binding","version":"1.0.0"}}}"#;

fn write_npm_project(project: &Path) {
    fs::write(project.join("package.json"), MANIFEST).unwrap();
    fs::write(project.join("package-lock.json"), LOCK).unwrap();
}

fn private_state_root(root: &Path) -> PathBuf {
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))
        .expect("the state root is made private");
    root.canonicalize()
        .expect("the state root resolves without symlinks")
}

#[test]
fn operator_check_node_binds_the_state_database_parent_registry() {
    let root = temp_root();
    let state_root = private_state_root(&root.0);
    let project = state_root.join("project");
    fs::create_dir(&project).unwrap();
    write_npm_project(&project);
    let state_db = state_root.join("state.sqlite3");

    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--non-interactive",
            "runtime",
            "check-node",
            "--yes",
            "--action",
            "test-ci",
        ])
        .arg("--project")
        .arg(&project)
        .current_dir(state_root.join("launch"))
        .env_clear()
        .env("HOME", state_root.join("home"))
        .env("SAYA_CONFIG_HOME", state_root.join("config-home"))
        .env("SAYA_STATE_DB", &state_db)
        .env("PATH", "")
        .env("LANG", "C")
        .env("LC_ALL", "C");
    for (name, value) in
        std::env::vars_os().filter(|(name, _)| name.to_string_lossy().starts_with("SAYA_TEST_"))
    {
        command.env(name, value);
    }
    let output = command.output().unwrap();
    let message = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(
        output.status.code(),
        Some(5),
        "an unusable managed registry is reported as a runtime failure: {message}"
    );
    assert!(
        message.contains("pinned Node runtime"),
        "the operator check reaches the managed registry readiness boundary before any transport: {message}"
    );
    assert!(
        state_root.join("managed-node-runtime").exists(),
        "the operator check composes the registry under the state database parent: {message}"
    );
    assert!(
        !state_root.join("coding").exists(),
        "the operator path never composes a registry beside a coding state root"
    );
    assert!(
        !state_db.exists(),
        "the operator check does not open or create the state database"
    );
}
