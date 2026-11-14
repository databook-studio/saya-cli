//! Recorded runtime identity inspection is bounded, private and read-only.
#![cfg(all(target_arch = "aarch64", any(target_os = "macos", target_os = "linux")))]
use saya_harness::runtime::{
    NODE_IMAGE_MANIFEST_DIGEST, NODE_VERSION, NPM_VERSION, node_installation_recorded,
};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "saya-ready-observation-{}-{stamp}",
            std::process::id()
        ));
        private_directory(&root);
        Self(root.canonicalize().unwrap())
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        if !std::thread::panicking() {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
}
fn private_directory(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn record() -> String {
    format!("image={NODE_IMAGE_MANIFEST_DIGEST}\nnode={NODE_VERSION}\nnpm={NPM_VERSION}\n")
}
fn write_private(path: &Path, content: &[u8]) {
    fs::write(path, content).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, rows: &mut BTreeMap<PathBuf, (u32, Vec<u8>)>) {
        assert!(rows.len() < 32);
        let metadata = fs::symlink_metadata(path).unwrap();
        let bytes = if metadata.file_type().is_symlink() {
            fs::read_link(path)
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
                .to_vec()
        } else if metadata.is_file() {
            assert!(metadata.len() <= 1024);
            fs::read(path).unwrap()
        } else {
            Vec::new()
        };
        rows.insert(
            path.strip_prefix(root).unwrap().to_owned(),
            (metadata.permissions().mode(), bytes),
        );
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            for child in fs::read_dir(path).unwrap() {
                visit(root, &child.unwrap().path(), rows);
            }
        }
    }
    let mut rows = BTreeMap::new();
    visit(root, root, &mut rows);
    rows
}

#[test]
fn installation_record_inspection_preserves_state_and_rejects_unsafe_identity() {
    let root = Root::new();
    for case in [
        "missing_state",
        "missing_registry",
        "missing_record",
        "exact",
        "tampered",
        "oversized",
        "record_symlink",
        "registry_symlink",
        "insecure_record",
        "insecure_ancestor",
    ] {
        let parent = root.0.join(case);
        private_directory(&parent);
        let state = parent.join("state");
        if case != "missing_state" {
            private_directory(&state);
        }
        let registry = state.join("managed-node-runtime");
        if !matches!(case, "missing_state" | "missing_registry") {
            if case == "registry_symlink" {
                symlink(&parent, &registry).unwrap();
            } else {
                private_directory(&registry);
            }
        }
        if !matches!(
            case,
            "missing_state" | "missing_registry" | "missing_record" | "registry_symlink"
        ) {
            let ready = registry.join("ready");
            match case {
                "record_symlink" => {
                    let target = parent.join("foreign-ready");
                    write_private(&target, record().as_bytes());
                    symlink(target, &ready).unwrap();
                }
                "tampered" => write_private(&ready, b"image=synthetic-wrong-identity\n"),
                "oversized" => write_private(&ready, &[b'x'; 513]),
                _ => write_private(&ready, record().as_bytes()),
            }
            if case == "insecure_record" {
                fs::set_permissions(&ready, fs::Permissions::from_mode(0o644)).unwrap();
            }
            if case == "insecure_ancestor" {
                fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
            }
        }
        let before = snapshot(&parent);
        let outcome = node_installation_recorded(&state);
        match case {
            "missing_state" | "missing_registry" | "missing_record" => {
                assert_eq!(outcome, Ok(false), "{case}")
            }
            "exact" => assert_eq!(outcome, Ok(true), "{case}"),
            _ => assert!(outcome.is_err(), "{case}: {outcome:?}"),
        }
        assert_eq!(
            snapshot(&parent),
            before,
            "inspection changed private state in {case}"
        );
    }
}
