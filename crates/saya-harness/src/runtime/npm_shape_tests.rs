use std::fs;

use serde_json::json;

use super::{
    bind_npm_project, integrity, package_lock, package_manifest, private_root, write_project,
};
use crate::runtime::NpmRuntimeError;

#[test]
fn nonregistry_dependency_protocols_and_git_shorthands_are_refused() {
    for specifier in [
        "file:../outside",
        "git+https://example.invalid/package.git",
        "workspace:*",
        "owner/repo",
        "owner/repo#branch",
    ] {
        let root = private_root("npm-nonregistry-spec");
        let manifest = json!({
            "name": "react-dashboard",
            "version": "1.0.0",
            "dependencies": {"outside": specifier}
        });
        let lock = json!({
            "name": "react-dashboard",
            "version": "1.0.0",
            "lockfileVersion": 3,
            "packages": {
                "": {
                    "name": "react-dashboard",
                    "version": "1.0.0",
                    "dependencies": {"outside": specifier}
                },
                "node_modules/outside": {
                    "version": "1.0.0",
                    "resolved": "https://registry.npmjs.org/outside/-/outside-1.0.0.tgz",
                    "integrity": integrity(9)
                }
            }
        });
        write_project(&root, manifest, lock);
        let workspace = crate::workspace::Workspace::open(&root).unwrap();

        assert!(matches!(
            bind_npm_project(&workspace),
            Err(NpmRuntimeError::InvalidLock | NpmRuntimeError::InvalidManifest)
        ));
        drop(workspace);
        fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn escaping_package_path_keys_are_refused() {
    let root = private_root("npm-escaping-key");
    let manifest = json!({
        "name": "react-dashboard",
        "version": "1.0.0",
        "dependencies": {"outside": "1.0.0"}
    });
    let lock = json!({
        "name": "react-dashboard",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "react-dashboard",
                "version": "1.0.0",
                "dependencies": {"outside": "1.0.0"}
            },
            "node_modules/../../outside": {
                "version": "1.0.0",
                "resolved": "https://registry.npmjs.org/outside/-/outside-1.0.0.tgz",
                "integrity": integrity(10)
            }
        }
    });
    write_project(&root, manifest, lock);
    let workspace = crate::workspace::Workspace::open(&root).unwrap();

    assert!(matches!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::InvalidLock)
    ));
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn link_package_entries_are_refused() {
    let root = private_root("npm-link-entry");
    let manifest = json!({
        "name": "react-dashboard",
        "version": "1.0.0",
        "dependencies": {"outside": "1.0.0"}
    });
    let lock = json!({
        "name": "react-dashboard",
        "version": "1.0.0",
        "lockfileVersion": 3,
        "packages": {
            "": {
                "name": "react-dashboard",
                "version": "1.0.0",
                "dependencies": {"outside": "1.0.0"}
            },
            "node_modules/outside": {
                "version": "1.0.0",
                "link": true,
                "resolved": "../outside"
            }
        }
    });
    write_project(&root, manifest, lock);
    let workspace = crate::workspace::Workspace::open(&root).unwrap();

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::InvalidLock)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn project_npmrc_is_refused() {
    let root = private_root("npmrc-refused");
    write_project(
        &root,
        package_manifest("react-dashboard", "1.0.0"),
        package_lock(
            "react-dashboard",
            "1.0.0",
            3,
            "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
            &integrity(11),
        ),
    );
    fs::write(
        root.join(".npmrc"),
        "registry=https://packages.attacker.example\n",
    )
    .unwrap();
    let workspace = crate::workspace::Workspace::open(&root).unwrap();

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::NpmrcRefused)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn mount_delimiters_in_workspace_root_are_refused() {
    let root = private_root("npm:mount");
    write_project(
        &root,
        package_manifest("react-dashboard", "1.0.0"),
        package_lock(
            "react-dashboard",
            "1.0.0",
            3,
            "https://registry.npmjs.org/saya-offline-probe/-/saya-offline-probe-1.0.0.tgz",
            &integrity(12),
        ),
    );
    let workspace = crate::workspace::Workspace::open(&root).unwrap();

    assert_eq!(
        bind_npm_project(&workspace),
        Err(NpmRuntimeError::InvalidProject)
    );
    drop(workspace);
    fs::remove_dir_all(root).unwrap();
}
